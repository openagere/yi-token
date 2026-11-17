//! Which provider issued each Responses reasoning item.
//!
//! A reasoning item's `encrypted_content` can only be decrypted by the provider that
//! issued it. Clients that replay the history (Codex does) send these items back
//! verbatim, so after a switch to another Responses provider the proxy must not forward
//! that state. An item from another provider keeps its summary and loses its `id` and
//! `encrypted_content`, as an unsigned Anthropic thinking block does.
//!
//! The origins live in memory and are bounded. An item they do not know is treated as
//! foreign: after a restart, earlier reasoning loses its encrypted state, but a request
//! is never rejected because of it.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

use serde_json::Value;

use crate::lock::lock_unpoisoned;
use crate::protocol::anthropic::ANTHROPIC_SIGNATURE;

/// Origins kept before the oldest one is forgotten.
pub const DEFAULT_CAPACITY: usize = 65_536;

/// Internal markers that must never reach a Responses upstream.
const PRIVATE_KEYS: [&str; 2] = [ANTHROPIC_SIGNATURE, "redacted"];

pub struct ReasoningOrigins {
    capacity: usize,
    inner: Mutex<Origins>,
}

#[derive(Default)]
struct Origins {
    /// Fingerprint of `encrypted_content` -> id of the provider that issued it.
    provider_of: HashMap<u64, String>,
    /// Insertion order, for eviction.
    order: VecDeque<u64>,
}

impl Origins {
    fn insert(&mut self, key: u64, provider: &str, capacity: usize) {
        if self.provider_of.insert(key, provider.to_owned()).is_none() {
            self.order.push_back(key);
            if self.order.len() > capacity
                && let Some(oldest) = self.order.pop_front()
            {
                self.provider_of.remove(&oldest);
            }
        }
    }

    fn issued_by(&self, provider: &str, encrypted: &str) -> bool {
        self.provider_of
            .get(&fingerprint(encrypted))
            .is_some_and(|issuer| issuer == provider)
    }
}

impl Default for ReasoningOrigins {
    fn default() -> Self {
        Self::new()
    }
}

impl ReasoningOrigins {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            inner: Mutex::new(Origins::default()),
        }
    }

    /// Records the reasoning items in a Responses body: a complete response, or a
    /// response object or output item carried by an SSE event.
    pub fn record(&self, provider: &str, body: &Value) {
        let mut found = Vec::new();
        collect(body, &mut found);
        if found.is_empty() {
            return;
        }
        let mut origins = lock_unpoisoned(&self.inner);
        for encrypted in found {
            origins.insert(fingerprint(encrypted), provider, self.capacity);
        }
    }

    /// [`record`](Self::record) for one SSE `data` payload. Frames that cannot carry
    /// reasoning are not parsed.
    pub fn record_frame(&self, provider: &str, data: &str) {
        if data.contains("encrypted_content")
            && let Ok(body) = serde_json::from_str::<Value>(data)
        {
            self.record(provider, &body);
        }
    }

    /// Makes a Responses request body safe for `provider`: reasoning items that it did
    /// not issue lose their `id` and `encrypted_content`, and internal markers are
    /// removed from every reasoning item.
    pub fn strip_foreign(&self, provider: &str, body: &mut Value) {
        let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
            return;
        };
        let origins = lock_unpoisoned(&self.inner);
        for item in input {
            if item.get("type").and_then(Value::as_str) != Some("reasoning") {
                continue;
            }
            let Some(obj) = item.as_object_mut() else {
                continue;
            };
            for key in PRIVATE_KEYS {
                obj.remove(key);
            }
            let foreign = obj
                .get("encrypted_content")
                .and_then(Value::as_str)
                .is_some_and(|encrypted| !origins.issued_by(provider, encrypted));
            if foreign {
                obj.remove("id");
                obj.remove("encrypted_content");
            }
        }
    }

    #[cfg(test)]
    fn is_from(&self, provider: &str, encrypted: &str) -> bool {
        lock_unpoisoned(&self.inner).issued_by(provider, encrypted)
    }
}

fn fingerprint(encrypted: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    encrypted.hash(&mut hasher);
    hasher.finish()
}

/// Every non-empty `encrypted_content` of a reasoning item reachable from `value`
/// through an output list, a response object, or an output-item event.
fn collect<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
    let Some(obj) = value.as_object() else {
        return;
    };
    if obj.get("type").and_then(Value::as_str) == Some("reasoning")
        && let Some(encrypted) = obj
            .get("encrypted_content")
            .and_then(Value::as_str)
            .filter(|encrypted| !encrypted.is_empty())
    {
        out.push(encrypted);
    }
    for key in ["item", "response"] {
        if let Some(child) = obj.get(key) {
            collect(child, out);
        }
    }
    if let Some(items) = obj.get("output").and_then(Value::as_array) {
        for item in items {
            collect(item, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn reasoning(id: &str, encrypted: &str) -> Value {
        json!({"type": "reasoning", "id": id,
            "summary": [{"type": "summary_text", "text": "thought"}],
            "encrypted_content": encrypted})
    }

    #[test]
    fn origins_are_known_only_to_the_provider_that_issued_them() {
        let origins = ReasoningOrigins::new();
        origins.record(
            "a",
            &json!({"output": [reasoning("rs_1", "ENC-1"), {"type": "message"}]}),
        );
        assert!(origins.is_from("a", "ENC-1"));
        assert!(!origins.is_from("b", "ENC-1"));
        assert!(!origins.is_from("a", "ENC-2"));
    }

    #[test]
    fn sse_events_are_recorded_and_other_frames_are_skipped() {
        let origins = ReasoningOrigins::new();
        let done = json!({"type": "response.output_item.done", "item": reasoning("rs_1", "ENC-1")});
        let completed = json!({"type": "response.completed",
            "response": {"output": [reasoning("rs_2", "ENC-2")]}});
        origins.record_frame("a", &done.to_string());
        origins.record_frame("a", &completed.to_string());
        origins.record_frame("a", r#"{"type":"response.output_text.delta","delta":"hi"}"#);
        origins.record_frame("a", "[DONE]");
        assert!(origins.is_from("a", "ENC-1"));
        assert!(origins.is_from("a", "ENC-2"));
    }

    #[test]
    fn strip_foreign_keeps_own_items_and_strips_the_rest_to_their_summary() {
        let origins = ReasoningOrigins::new();
        origins.record("a", &json!({"output": [reasoning("rs_mine", "ENC-A")]}));
        origins.record("b", &json!({"output": [reasoning("rs_b", "ENC-B")]}));
        let message = json!({"type": "message", "role": "user", "content": "hi"});
        let mut body = json!({"model": "m", "input": [
            reasoning("rs_mine", "ENC-A"),
            reasoning("rs_b", "ENC-B"),
            reasoning("rs_unknown", "ENC-X"),
            message.clone(),
            {"type": "reasoning", "summary": [], "anthropic_signature": true},
        ]});

        origins.strip_foreign("a", &mut body);

        let input = body["input"].as_array().unwrap();
        assert_eq!(input[0], reasoning("rs_mine", "ENC-A"));
        for foreign in [&input[1], &input[2]] {
            assert!(foreign.get("id").is_none());
            assert!(foreign.get("encrypted_content").is_none());
            assert_eq!(foreign["summary"][0]["text"], "thought");
        }
        assert_eq!(input[3], message);
        assert_eq!(input[4], json!({"type": "reasoning", "summary": []}));
    }

    #[test]
    fn strip_foreign_ignores_bodies_without_an_input_list() {
        let origins = ReasoningOrigins::new();
        let mut body = json!({"model": "m", "input": "plain text"});
        origins.strip_foreign("a", &mut body);
        assert_eq!(body, json!({"model": "m", "input": "plain text"}));
    }

    #[test]
    fn the_oldest_origin_is_forgotten_at_capacity() {
        let origins = ReasoningOrigins::with_capacity(2);
        for (id, encrypted) in [("1", "E1"), ("2", "E2"), ("3", "E3")] {
            origins.record("a", &json!({"output": [reasoning(id, encrypted)]}));
        }
        assert!(!origins.is_from("a", "E1"));
        assert!(origins.is_from("a", "E2"));
        assert!(origins.is_from("a", "E3"));
    }
}
