//! Normalization of IR input items for the trajectory store: a canonical JSON
//! body (hashed for exact identity), a semantic fingerprint (hashed for
//! cross-request matching), and helpers for rolling fingerprints.

use crate::protocol::ir::{Content, ContentPart, ImagePart, InputItem, Tool};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

/// Kind of a normalized item, as seen by the trajectory store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Message,
    ToolCall,
    ToolResult,
    Reasoning,
    System,
    Tools,
    Unknown,
}

/// A conversation item reduced to its stored body, hash and fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedItem {
    pub kind: ItemKind,
    /// Role of a message item.
    pub role: Option<String>,
    pub call_id: Option<String>,
    pub tool_name: Option<String>,
    /// Canonical JSON (sorted keys, compact), truncated and image-stripped.
    pub body: Vec<u8>,
    /// Byte length of the canonical body before truncation.
    pub raw_bytes: u64,
    pub truncated: bool,
    /// hex(SHA-256(body)).
    pub hash: String,
    /// Semantic fingerprint: SHA-256 of the semantic JSON.
    pub fp: [u8; 32],
}

pub const EMPTY_FP: [u8; 32] = [0; 32];

/// Bodies larger than this are truncated.
const MAX_BODY: usize = 1024 * 1024;
/// Bytes kept from each end of a truncated string.
const HEAD_TAIL: usize = 256 * 1024;
/// `data:` strings longer than this are replaced by an `inline_image` summary.
const INLINE_MIN: usize = 64 * 1024;

pub fn normalize_item(item: &InputItem) -> NormalizedItem {
    let value = serde_json::to_value(item).unwrap_or(Value::Null);
    match item {
        InputItem::Message(m) => {
            let semantic = json!({"role": m.role, "text": message_text(&m.content)});
            build(
                ItemKind::Message,
                Some(m.role.clone()),
                None,
                None,
                value,
                &semantic,
            )
        }
        InputItem::FunctionCall(c) => {
            let args = match c.arguments.as_deref() {
                Some(raw) => serde_json::from_str::<Value>(raw)
                    .unwrap_or_else(|_| Value::String(strip_whitespace(raw))),
                None => Value::Null,
            };
            let semantic = json!({"name": c.name, "call_id": c.call_id, "args": args});
            build(
                ItemKind::ToolCall,
                None,
                c.call_id.clone(),
                c.name.clone(),
                value,
                &semantic,
            )
        }
        InputItem::FunctionCallOutput(o) => {
            let semantic = json!({
                "call_id": o.call_id,
                "output": output_text(o.output.as_ref()),
            });
            build(
                ItemKind::ToolResult,
                None,
                o.call_id.clone(),
                None,
                value,
                &semantic,
            )
        }
        InputItem::Reasoning(_) => build(
            ItemKind::Reasoning,
            None,
            None,
            None,
            value,
            &json!({"reasoning": true}),
        ),
        InputItem::Unknown(v) => {
            let semantic = json!({"type": v.get("type").cloned().unwrap_or(Value::Null)});
            build(ItemKind::Unknown, None, None, None, value, &semantic)
        }
    }
}

pub fn normalize_system(instructions: &str) -> NormalizedItem {
    build(
        ItemKind::System,
        Some("system".into()),
        None,
        None,
        json!({"instructions": instructions}),
        &json!({"role": "system", "text": instructions.trim()}),
    )
}

pub fn normalize_tools(tools: &[Tool]) -> Option<NormalizedItem> {
    if tools.is_empty() {
        return None;
    }
    let list = json!({"tools": serde_json::to_value(tools).unwrap_or(Value::Null)});
    // The fingerprint strips `created_at` exactly like the body does.
    let semantic = canonical(list.clone(), true);
    Some(build(ItemKind::Tools, None, None, None, list, &semantic))
}

pub fn rolling_fp(prev: &[u8; 32], next: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(prev);
    h.update(next);
    h.finalize().into()
}

/// Context clients inject as user messages (environment, instructions, reminders). It is not
/// what the user typed, so it must not become a session title.
const INJECTED_PREFIXES: &[&str] = &[
    "<environment_context>",
    "<user_instructions>",
    "<user_shell_command>",
    "<system-reminder>",
    "<command-name>",
    "<command-message>",
    "<command-args>",
    "<local-command-",
    "<turn_aborted>",
    "<INSTRUCTIONS>",
    "# AGENTS.md instructions",
    "# CLAUDE.md",
];

fn is_injected(text: &str) -> bool {
    let trimmed = text.trim_start();
    INJECTED_PREFIXES
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
}

/// Text the user typed in the first user message that has any, cut to 80 characters.
/// Injected context parts are skipped, so a message holding only them yields nothing.
pub fn first_user_text(items: &[NormalizedItem]) -> Option<String> {
    items
        .iter()
        .filter(|i| i.kind == ItemKind::Message && i.role.as_deref() == Some("user"))
        .find_map(|i| {
            let body: Value = serde_json::from_slice(&i.body).ok()?;
            let text: String = match &body["content"] {
                Value::String(s) if !is_injected(s) => s.clone(),
                Value::String(_) => String::new(),
                Value::Array(parts) => parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(Value::as_str))
                    .filter(|t| !is_injected(t))
                    .collect(),
                _ => return None,
            };
            let t = text.trim();
            (!t.is_empty()).then(|| t.chars().take(80).collect())
        })
}

// ------------------------------------------------------------ construction

fn build(
    kind: ItemKind,
    role: Option<String>,
    call_id: Option<String>,
    tool_name: Option<String>,
    value: Value,
    semantic: &Value,
) -> NormalizedItem {
    let canonical = inline_images(canonical(value, true));
    let raw = serde_json::to_vec(&canonical).unwrap_or_default();
    let raw_bytes = raw.len() as u64;
    let (body, truncated) = if raw.len() > MAX_BODY {
        (truncate_body(canonical), true)
    } else {
        (raw, false)
    };
    let fp_bytes = serde_json::to_vec(&canonical_keys(semantic.clone())).unwrap_or_default();
    NormalizedItem {
        kind,
        role,
        call_id,
        tool_name,
        hash: hex(&sha256(&body)),
        body,
        raw_bytes,
        truncated,
        fp: sha256(&fp_bytes),
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex(&sha256(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Sorts object keys recursively and, when `strip_created_at` is set, drops
/// `created_at` keys at every level.
fn canonical(v: Value, strip_created_at: bool) -> Value {
    match v {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map
                .into_iter()
                .filter(|(k, _)| !(strip_created_at && k == "created_at"))
                .map(|(k, v)| (k, canonical(v, strip_created_at)))
                .collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(entries.into_iter().collect::<Map<String, Value>>())
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|v| canonical(v, strip_created_at))
                .collect(),
        ),
        other => other,
    }
}

fn canonical_keys(v: Value) -> Value {
    canonical(v, false)
}

fn strip_whitespace(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

// ------------------------------------------------------------ inline images

fn inline_images(v: Value) -> Value {
    match v {
        Value::String(s) if s.starts_with("data:") && s.len() > INLINE_MIN => inline_summary(&s),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, inline_images(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(inline_images).collect()),
        other => other,
    }
}

fn inline_summary(s: &str) -> Value {
    let rest = &s["data:".len()..];
    let (head, payload) = rest.split_once(',').unwrap_or(("", rest));
    let media_type = head.split(';').next().unwrap_or("");
    let bytes = if head.contains(";base64") {
        let padding = payload.bytes().rev().take_while(|&b| b == b'=').count();
        (payload.len() / 4 * 3).saturating_sub(padding)
    } else {
        payload.len()
    };
    json!({
        "inline_image": {
            "media_type": media_type,
            "bytes": bytes,
            "sha256": sha256_hex(payload.as_bytes()),
        }
    })
}

// ---------------------------------------------------------------- truncation

fn truncate_body(canonical: Value) -> Vec<u8> {
    let mut v = canonical;
    let longest = longest_string_len(&v);
    if longest > 2 * HEAD_TAIL {
        truncate_first_string_of_len(&mut v, longest);
    }
    let bytes = serde_json::to_vec(&v).unwrap_or_default();
    if bytes.len() <= MAX_BODY {
        return bytes;
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    serde_json::to_vec(&json!({"truncated_json": head_tail(&text)})).unwrap_or_default()
}

fn longest_string_len(v: &Value) -> usize {
    match v {
        Value::String(s) => s.len(),
        Value::Array(items) => items.iter().map(longest_string_len).max().unwrap_or(0),
        Value::Object(map) => map.values().map(longest_string_len).max().unwrap_or(0),
        _ => 0,
    }
}

/// Truncates the first string of exactly `len` bytes (and of sufficient size).
/// Returns true when a string was truncated.
fn truncate_first_string_of_len(v: &mut Value, len: usize) -> bool {
    match v {
        Value::String(s) if s.len() == len => {
            *s = head_tail(s);
            true
        }
        Value::Array(items) => items
            .iter_mut()
            .any(|item| truncate_first_string_of_len(item, len)),
        Value::Object(map) => map
            .values_mut()
            .any(|item| truncate_first_string_of_len(item, len)),
        _ => false,
    }
}

/// Keeps the first and last `HEAD_TAIL` bytes (on char boundaries) around a
/// marker recording how many bytes were removed.
fn head_tail(s: &str) -> String {
    let len = s.len();
    let mut head = HEAD_TAIL.min(len);
    while !s.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = len.saturating_sub(HEAD_TAIL);
    while !s.is_char_boundary(tail) {
        tail += 1;
    }
    let removed = tail.saturating_sub(head);
    format!("{}…[truncated {removed} bytes]…{}", &s[..head], &s[tail..])
}

// ------------------------------------------------------------------- text

fn message_text(content: &Content) -> String {
    let text = match content {
        Content::Text(s) => s.clone(),
        Content::Parts(parts) => parts.iter().map(part_text).collect(),
    };
    text.trim().to_owned()
}

fn part_text(part: &ContentPart) -> String {
    match part {
        ContentPart::InputText(t) | ContentPart::OutputText(t) => t.text.clone(),
        ContentPart::InputImage(img) => image_token(img),
        ContentPart::Unknown(v) => fp_json(v),
    }
}

/// Compact text of an unmodelled JSON value for fingerprinting: `id` and
/// `created_at` are removed at every level and keys are sorted.
fn fp_json(v: &Value) -> String {
    serde_json::to_string(&canonical_keys(strip_ids(v))).unwrap_or_default()
}

fn strip_ids(v: &Value) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(k, _)| k.as_str() != "id" && k.as_str() != "created_at")
                .map(|(k, v)| (k.clone(), strip_ids(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(strip_ids).collect()),
        other => other.clone(),
    }
}

fn image_token(img: &ImagePart) -> String {
    image_token_from(img.image_url.as_deref(), img.file_id.as_deref())
}

fn image_token_from(image_url: Option<&str>, file_id: Option<&str>) -> String {
    match (image_url, file_id) {
        (Some(url), _) => sha256_hex(url.as_bytes()),
        (None, Some(id)) => id.to_owned(),
        (None, None) => String::new(),
    }
}

/// Text of a function call output, which is a string or a list of parts.
fn output_text(output: Option<&Value>) -> Value {
    let text = match output {
        None | Some(Value::Null) => return Value::Null,
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                let kind = item.get("type").and_then(Value::as_str);
                match (kind, item.get("text").and_then(Value::as_str)) {
                    (_, Some(text)) => text.to_owned(),
                    (Some("input_image"), None) => image_token_from(
                        item.get("image_url").and_then(Value::as_str),
                        item.get("file_id").and_then(Value::as_str),
                    ),
                    _ => fp_json(item),
                }
            })
            .collect(),
        Some(other) => fp_json(other),
    };
    Value::String(text.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(v: Value) -> InputItem {
        serde_json::from_value(v).unwrap()
    }

    fn normalized(v: Value) -> NormalizedItem {
        normalize_item(&item(v))
    }

    #[test]
    fn hash_is_independent_of_key_order() {
        let a = normalized(json!({"role": "user", "content": "hi"}));
        let b = normalized(json!({"content": "hi", "role": "user"}));
        assert_eq!(a.hash, b.hash);
        assert_eq!(a.body, b.body);
    }

    #[test]
    fn body_keys_are_sorted_recursively() {
        // Input keys are deliberately out of order, including the flattened
        // `extra` keys (zeta, alpha). This pins the exact sorted, compact body.
        // Note: serde_json runs with its default BTreeMap-backed Value (no
        // preserve_order), so key sorting already happens in to_value and the
        // sort in `canonical` is defence in depth. This test therefore guards
        // the output contract; it cannot detect removal of `canonical`.
        let v = json!({
            "type": "message", "role": "user", "content": "hi",
            "zeta": 1, "alpha": 2
        });
        let n = normalize_item(&item(v));
        assert_eq!(
            std::str::from_utf8(&n.body).unwrap(),
            r#"{"alpha":2,"content":"hi","role":"user","type":"message","zeta":1}"#
        );
    }

    #[test]
    fn unknown_parts_fp_ignores_id_and_created_at() {
        let with = normalized(json!({
            "role": "user",
            "content": [{"type": "x", "id": "p1", "created_at": 5}]
        }));
        let without = normalized(json!({"role": "user", "content": [{"type": "x"}]}));
        assert_eq!(with.fp, without.fp);
        let out_with = normalized(json!({
            "type": "function_call_output", "call_id": "c1",
            "output": [{"type": "x", "id": "p1"}]
        }));
        let out_without = normalized(json!({
            "type": "function_call_output", "call_id": "c1",
            "output": [{"type": "x"}]
        }));
        assert_eq!(out_with.fp, out_without.fp);
    }

    #[test]
    fn tools_fp_ignores_created_at() {
        let with =
            serde_json::from_value::<Tool>(json!({"type": "web_search", "created_at": 1})).unwrap();
        let without = serde_json::from_value::<Tool>(json!({"type": "web_search"})).unwrap();
        let a = normalize_tools(&[with]).unwrap();
        let b = normalize_tools(&[without]).unwrap();
        assert_eq!(a.fp, b.fp);
    }

    #[test]
    fn fp_ignores_ids_and_signatures() {
        let with = normalized(json!({
            "type": "message", "id": "msg_1", "role": "assistant",
            "content": [{"type": "output_text", "text": "hello"}], "signature": "sig"
        }));
        let without = normalized(json!({
            "type": "message", "role": "assistant",
            "content": [{"type": "output_text", "text": "hello"}]
        }));
        assert_eq!(with.fp, without.fp);
        assert_ne!(with.hash, without.hash);
    }

    #[test]
    fn reasoning_fp_is_constant() {
        let a = normalized(json!({
            "type": "reasoning", "id": "rs_1",
            "summary": [{"type": "summary_text", "text": "first"}]
        }));
        let b = normalized(json!({
            "type": "reasoning", "id": "rs_2",
            "summary": [{"type": "summary_text", "text": "second"}]
        }));
        assert_eq!(a.kind, ItemKind::Reasoning);
        assert_eq!(a.fp, b.fp);
    }

    #[test]
    fn tool_call_args_whitespace_does_not_change_fp() {
        let compact = normalized(json!({
            "type": "function_call", "call_id": "c1", "name": "f", "arguments": "{\"a\":1}"
        }));
        let spaced = normalized(json!({
            "type": "function_call", "call_id": "c1", "name": "f", "arguments": "{ \"a\" : 1 }"
        }));
        assert_eq!(compact.kind, ItemKind::ToolCall);
        assert_eq!(compact.call_id.as_deref(), Some("c1"));
        assert_eq!(compact.tool_name.as_deref(), Some("f"));
        assert_eq!(compact.fp, spaced.fp);
    }

    #[test]
    fn large_text_is_truncated_on_char_boundary() {
        let text: String = "中".repeat(1_200_000);
        assert!(text.len() > 3 * 1024 * 1024);
        let n = normalize_item(&InputItem::Message(crate::protocol::ir::Message {
            kind: None,
            role: "user".into(),
            content: crate::protocol::ir::Content::Text(text),
            extra: serde_json::Map::new(),
        }));
        assert!(n.truncated);
        assert!(n.raw_bytes > 3 * 1024 * 1024);
        assert!(n.body.len() <= 1024 * 1024 + 200, "{}", n.body.len());
        let s = std::str::from_utf8(&n.body).unwrap();
        let v: Value = serde_json::from_str(s).unwrap();
        assert!(v["content"].as_str().unwrap().contains("…[truncated"));
    }

    #[test]
    fn inline_image_over_64k_is_replaced() {
        let payload = "A".repeat(100 * 1024);
        let url = format!("data:image/png;base64,{payload}");
        let n = normalized(json!({
            "role": "user",
            "content": [{"type": "input_image", "image_url": url}]
        }));
        let s = std::str::from_utf8(&n.body).unwrap();
        assert!(!s.contains("base64"), "{s}");
        assert!(!s.contains(&payload));
        assert!(s.contains("inline_image"), "{s}");
        assert!(s.contains("image/png"), "{s}");
    }

    #[test]
    fn first_user_text_takes_80_chars() {
        let text: String = "汉".repeat(100);
        let items = vec![
            normalized(json!({"role": "user", "content": text})),
            normalized(json!({"role": "user", "content": "second"})),
        ];
        let got = first_user_text(&items).unwrap();
        assert_eq!(got.chars().count(), 80);
        assert_eq!(got, "汉".repeat(80));
    }

    #[test]
    fn first_user_text_skips_injected_context() {
        let items = vec![
            normalized(json!({"role": "user", "content": [
                {"type": "input_text", "text": "<environment_context>\n  <cwd>D:\\tmp</cwd>\n</environment_context>"},
                {"type": "input_text", "text": "# AGENTS.md instructions for D:\\tmp"},
            ]})),
            normalized(json!({"role": "user", "content": "帮我看下这个报错"})),
        ];
        assert_eq!(first_user_text(&items).as_deref(), Some("帮我看下这个报错"));
    }

    #[test]
    fn first_user_text_keeps_the_typed_part_of_a_mixed_message() {
        let items = vec![normalized(json!({"role": "user", "content": [
            {"type": "input_text", "text": "<system-reminder>context</system-reminder>"},
            {"type": "input_text", "text": "真正的问题"},
        ]}))];
        assert_eq!(first_user_text(&items).as_deref(), Some("真正的问题"));
    }

    #[test]
    fn first_user_text_is_none_for_tool_results_only() {
        let items = vec![normalized(json!({
            "type": "function_call_output", "call_id": "c1", "output": "ok"
        }))];
        assert_eq!(first_user_text(&items), None);
    }

    #[test]
    fn rolling_fp_is_order_sensitive() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        let ab = rolling_fp(&rolling_fp(&EMPTY_FP, &a), &b);
        let ba = rolling_fp(&rolling_fp(&EMPTY_FP, &b), &a);
        assert_ne!(ab, ba);
    }

    #[test]
    fn normalize_tools_is_none_when_empty() {
        assert!(normalize_tools(&[]).is_none());
    }

    #[test]
    fn normalize_system_has_system_kind() {
        let n = normalize_system("be nice");
        assert_eq!(n.kind, ItemKind::System);
        assert_eq!(n.role.as_deref(), Some("system"));
        assert!(!n.truncated);
    }
}
