//! Step linker (spec 4.3, 4.4): decides which session, thread, parent step,
//! turn and checkpoint a captured step belongs to. The decision is a pure
//! function of the step and a read-only [`LinkStore`]; this module never
//! touches SQLite. The SQLite writer implements the store and records each
//! outcome after the call.

use serde_json::json;
use sha2::{Digest, Sha256};

use super::normalize::{EMPTY_FP, ItemKind, NormalizedItem, first_user_text, rolling_fp};
use super::record::ClientKeys;
use crate::domain::{Client, KeySource};

/// Checkpoint interval in steps along a thread (spec 4.5 / section 5).
pub const CHECKPOINT_EVERY: u32 = 64;
/// A step can only continue a tail that was active within this window.
pub const CHAIN_WINDOW_MS: i64 = 12 * 3600 * 1000;

/// One step as seen by the linker.
#[derive(Debug, Clone)]
pub struct StepInput<'a> {
    pub client: Option<Client>,
    pub keys: &'a ClientKeys,
    pub requested_at: i64,
    /// Full input items (excluding system and tools).
    pub input: &'a [NormalizedItem],
    /// Output items of this step.
    pub output: &'a [NormalizedItem],
    pub has_tools: bool,
    pub model: &'a str,
}

/// A step tail that a later request can continue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailEntry {
    pub step_id: String,
    pub session_id: String,
    pub thread_id: String,
    pub client: Option<Client>,
    pub last_at: i64,
    /// Number of input plus output items.
    pub tail_len: usize,
    pub turn_no: u32,
    pub depth: u32,
    /// `steps.client_turn_id` of the step, used by the turn rule (spec 4.4).
    pub client_turn_id: Option<String>,
}

/// A tool call in a session that has no matching result yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCall {
    pub thread_id: String,
    pub call_id: String,
    /// Fingerprint of the call's `prompt` argument text (see [`prompt_fp`]).
    pub prompt_fp: Option<[u8; 32]>,
}

/// Read-only lookups the linker needs. The SQLite writer implements this with
/// the LRU tail index in front of `steps.tail_fp`.
pub trait LinkStore {
    fn session_by_key(&self, client: Option<Client>, key: &str) -> Option<String>;
    /// Looks in the LRU first, then in `steps.tail_fp`.
    fn tail_by_fp(&self, fp: &[u8; 32]) -> Option<TailEntry>;
    /// Tool calls in the session that have no paired result yet.
    fn open_tool_calls(&self, session_id: &str) -> Vec<OpenCall>;
    fn thread_kind(&self, thread_id: &str) -> Option<ThreadKind>;
    /// Whether the session already has a `Main` thread (reset detection, spec 4.4).
    fn session_has_main(&self, session_id: &str) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadKind {
    Main,
    Subagent,
    Side,
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionDecision {
    Existing(String),
    New {
        key_source: KeySource,
        client_key: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadDecision {
    Continue {
        thread_id: String,
        parent_step_id: String,
        prefix_len: usize,
    },
    New {
        kind: ThreadKind,
        parent_thread_id: Option<String>,
        spawn_call_id: Option<String>,
        reset: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkResult {
    pub session: SessionDecision,
    pub thread: ThreadDecision,
    pub turn_no: u32,
    pub opens_turn: bool,
    /// When true, the full input is stored and `prefix_len` counts as 0.
    pub checkpoint: bool,
    pub depth: u32,
    /// fold(input ++ output).
    pub tail_fp: [u8; 32],
}

/// Fingerprint of a prompt text, equal to the `fp` of a plain user message
/// with that text. Used for `OpenCall::prompt_fp` and subagent matching.
pub fn prompt_fp(text: &str) -> [u8; 32] {
    let bytes =
        serde_json::to_vec(&json!({"role": "user", "text": text.trim()})).unwrap_or_default();
    Sha256::digest(bytes).into()
}

/// Links one step to a session, thread, parent step, turn and checkpoint.
pub fn link(store: &dyn LinkStore, step: &StepInput, now: i64) -> LinkResult {
    let input = step.input;

    // prefix_fps[k] = fold(input[..k]), built incrementally.
    let mut prefix_fps = Vec::with_capacity(input.len() + 1);
    let mut acc = EMPTY_FP;
    prefix_fps.push(acc);
    for item in input {
        acc = rolling_fp(&acc, &item.fp);
        prefix_fps.push(acc);
    }

    // Longest prefix match with k >= 1 (spec 4.3, step 2).
    let parent = (1..=input.len()).rev().find_map(|k| {
        let tail = store.tail_by_fp(&prefix_fps[k])?;
        (tail.client == step.client && now - tail.last_at <= CHAIN_WINDOW_MS && tail.tail_len == k)
            .then_some((k, tail))
    });

    // Session (spec 4.3): client key first, then the chain, then a new session.
    let (session, session_id, key_matched) = match (step.keys.session_key.as_deref(), &parent) {
        (Some(key), _) => match store.session_by_key(step.client, key) {
            Some(id) => (SessionDecision::Existing(id.clone()), Some(id), true),
            None => (
                SessionDecision::New {
                    key_source: KeySource::Client,
                    client_key: Some(key.to_owned()),
                },
                None,
                false,
            ),
        },
        (None, Some((_, tail))) => (
            SessionDecision::Existing(tail.session_id.clone()),
            Some(tail.session_id.clone()),
            false,
        ),
        (None, None) => (
            SessionDecision::New {
                key_source: KeySource::New,
                client_key: None,
            },
            None,
            false,
        ),
    };

    // A parent only continues a thread when it belongs to the decided session.
    let cont = parent
        .as_ref()
        .filter(|(_, tail)| session_id.as_deref() == Some(tail.session_id.as_str()));

    let (thread, prefix_len) = match cont {
        Some((k, tail)) => (
            ThreadDecision::Continue {
                thread_id: tail.thread_id.clone(),
                parent_step_id: tail.step_id.clone(),
                prefix_len: *k,
            },
            *k,
        ),
        None => (
            new_thread(store, step, session_id.as_deref(), key_matched),
            0,
        ),
    };

    // Turn (spec 4.4).
    let (turn_no, opens_turn) = match cont {
        Some((_, tail)) => {
            let changed = match step.keys.turn_id.as_deref() {
                Some(turn) => tail.client_turn_id.as_deref() != Some(turn),
                None => has_user_text(&input[prefix_len..]),
            };
            if changed {
                (tail.turn_no + 1, true)
            } else {
                (tail.turn_no, false)
            }
        }
        None => (1, step.keys.turn_id.is_some() || has_user_text(input)),
    };

    // Checkpoint (spec 4.4, step 7).
    let (depth, checkpoint) = match cont {
        Some((_, tail)) => {
            let depth = tail.depth + 1;
            if depth >= CHECKPOINT_EVERY {
                (0, true)
            } else {
                (depth, false)
            }
        }
        None => (0, true),
    };

    let tail_fp = step
        .output
        .iter()
        .fold(acc, |acc, item| rolling_fp(&acc, &item.fp));

    LinkResult {
        session,
        thread,
        turn_no,
        opens_turn,
        checkpoint,
        depth,
        tail_fp,
    }
}

/// Decides a new thread: a subagent when the first user message equals an
/// open tool call's prompt, a side thread when the step has no tool definitions
/// and no tool calls (spec 4.4), otherwise a pending thread. Side and pending
/// threads are flagged as a reset when the client key matched a session that
/// already has a main thread.
fn new_thread(
    store: &dyn LinkStore,
    step: &StepInput,
    session_id: Option<&str>,
    key_matched: bool,
) -> ThreadDecision {
    if let (Some(session_id), Some(first)) = (session_id, first_user_item(step.input)) {
        let spawn = store
            .open_tool_calls(session_id)
            .into_iter()
            .find(|call| call.prompt_fp == Some(first.fp));
        if let Some(call) = spawn {
            return ThreadDecision::New {
                kind: ThreadKind::Subagent,
                parent_thread_id: Some(call.thread_id),
                spawn_call_id: Some(call.call_id),
                reset: false,
            };
        }
    }
    let reset = key_matched && session_id.is_some_and(|id| store.session_has_main(id));
    let side = !step.has_tools
        && !step
            .output
            .iter()
            .any(|item| item.kind == ItemKind::ToolCall);
    ThreadDecision::New {
        kind: if side {
            ThreadKind::Side
        } else {
            ThreadKind::Pending
        },
        parent_thread_id: None,
        spawn_call_id: None,
        reset,
    }
}

fn first_user_item(items: &[NormalizedItem]) -> Option<&NormalizedItem> {
    items
        .iter()
        .find(|item| first_user_text(std::slice::from_ref(item)).is_some())
}

fn has_user_text(items: &[NormalizedItem]) -> bool {
    first_user_text(items).is_some()
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use serde_json::{Value, json};

    use super::*;
    use crate::protocol::ir::InputItem;
    use crate::trajectory::normalize::{ItemKind, normalize_item};

    fn item(v: Value) -> NormalizedItem {
        normalize_item(&serde_json::from_value::<InputItem>(v).unwrap())
    }

    fn user(text: &str) -> NormalizedItem {
        item(json!({"role": "user", "content": text}))
    }

    fn asst(text: &str) -> NormalizedItem {
        item(json!({
            "type": "message", "role": "assistant",
            "content": [{"type": "output_text", "text": text}]
        }))
    }

    fn call(call_id: &str, name: &str, arguments: &str) -> NormalizedItem {
        item(json!({
            "type": "function_call", "call_id": call_id,
            "name": name, "arguments": arguments
        }))
    }

    fn result(call_id: &str, output: &str) -> NormalizedItem {
        item(json!({
            "type": "function_call_output", "call_id": call_id, "output": output
        }))
    }

    fn keyed(session_key: &str) -> ClientKeys {
        ClientKeys {
            session_key: Some(session_key.into()),
            turn_id: None,
        }
    }

    fn with_turn(turn_id: &str) -> ClientKeys {
        ClientKeys {
            session_key: None,
            turn_id: Some(turn_id.into()),
        }
    }

    /// Prompt fingerprint of a `function_call` item whose `arguments` JSON has a `prompt` field.
    fn prompt_fp_of_call(item: &NormalizedItem) -> Option<[u8; 32]> {
        let body: Value = serde_json::from_slice(&item.body).ok()?;
        let args: Value = serde_json::from_str(body["arguments"].as_str()?).ok()?;
        Some(prompt_fp(args["prompt"].as_str()?))
    }

    /// In-memory `LinkStore`. The harness updates it after each `link` call,
    /// the way the SQLite writer does.
    #[derive(Default)]
    struct MemStore {
        sessions: HashMap<(Option<Client>, String), String>,
        tails: HashMap<[u8; 32], TailEntry>,
        open: HashMap<String, Vec<OpenCall>>,
        kinds: HashMap<String, ThreadKind>,
        mains: HashSet<String>,
    }

    impl LinkStore for MemStore {
        fn session_by_key(&self, client: Option<Client>, key: &str) -> Option<String> {
            self.sessions.get(&(client, key.to_owned())).cloned()
        }

        fn tail_by_fp(&self, fp: &[u8; 32]) -> Option<TailEntry> {
            self.tails.get(fp).cloned()
        }

        fn open_tool_calls(&self, session_id: &str) -> Vec<OpenCall> {
            self.open.get(session_id).cloned().unwrap_or_default()
        }

        fn thread_kind(&self, thread_id: &str) -> Option<ThreadKind> {
            self.kinds.get(thread_id).copied()
        }

        fn session_has_main(&self, session_id: &str) -> bool {
            self.mains.contains(session_id)
        }
    }

    struct Done {
        link: LinkResult,
        session_id: String,
        thread_id: String,
    }

    #[derive(Default)]
    struct Harness {
        store: MemStore,
        seq: u32,
    }

    impl Harness {
        fn fresh(&mut self, prefix: &str) -> String {
            self.seq += 1;
            format!("{prefix}{}", self.seq)
        }

        /// Links one step, then records it in the store the way the writer will.
        fn step(
            &mut self,
            client: Option<Client>,
            keys: &ClientKeys,
            now: i64,
            input: &[NormalizedItem],
            output: &[NormalizedItem],
        ) -> Done {
            let res = link(
                &self.store,
                &StepInput {
                    client,
                    keys,
                    requested_at: now,
                    input,
                    output,
                    has_tools: true,
                    model: "m",
                },
                now,
            );
            let session_id = match &res.session {
                SessionDecision::Existing(id) => id.clone(),
                SessionDecision::New { client_key, .. } => {
                    let id = self.fresh("s");
                    if let Some(key) = client_key {
                        self.store
                            .sessions
                            .insert((client, key.clone()), id.clone());
                    }
                    id
                }
            };
            let thread_id = match &res.thread {
                ThreadDecision::Continue { thread_id, .. } => thread_id.clone(),
                ThreadDecision::New { kind, .. } => {
                    let id = self.fresh("t");
                    self.store.kinds.insert(id.clone(), *kind);
                    id
                }
            };
            let step_id = self.fresh("step");
            let has_calls = output.iter().any(|i| i.kind == ItemKind::ToolCall);
            // The first tool call in a session without a Main thread makes this thread Main.
            if has_calls && self.store.mains.insert(session_id.clone()) {
                self.store.kinds.insert(thread_id.clone(), ThreadKind::Main);
            }
            let open = self.store.open.entry(session_id.clone()).or_default();
            for it in input.iter().chain(output) {
                if let (ItemKind::ToolResult, Some(id)) = (it.kind, &it.call_id) {
                    open.retain(|c| &c.call_id != id);
                }
            }
            for it in output.iter().filter(|i| i.kind == ItemKind::ToolCall) {
                open.push(OpenCall {
                    thread_id: thread_id.clone(),
                    call_id: it.call_id.clone().unwrap_or_default(),
                    prompt_fp: prompt_fp_of_call(it),
                });
            }
            self.store.tails.insert(
                res.tail_fp,
                TailEntry {
                    step_id,
                    session_id: session_id.clone(),
                    thread_id: thread_id.clone(),
                    client,
                    last_at: now,
                    tail_len: input.len() + output.len(),
                    turn_no: res.turn_no,
                    depth: res.depth,
                    client_turn_id: keys.turn_id.clone(),
                },
            );
            Done {
                link: res,
                session_id,
                thread_id,
            }
        }
    }

    #[test]
    fn five_step_tool_loop_is_one_session_one_thread_one_turn() {
        let mut h = Harness::default();
        let keys = ClientKeys::default();
        let mut input = vec![user("list the files")];
        let mut sessions = Vec::new();
        let mut threads = Vec::new();
        let mut prev_tail_len: Option<usize> = None;
        for i in 1..=5i64 {
            let output = if i < 5 {
                vec![call(&format!("c{i}"), "shell", "{}")]
            } else {
                vec![asst("done")]
            };
            let done = h.step(Some(Client::Codex), &keys, 1000 * i, &input, &output);
            if i == 1 {
                assert!(matches!(
                    done.link.session,
                    SessionDecision::New {
                        key_source: KeySource::New,
                        client_key: None
                    }
                ));
            }
            if let Some(prev) = prev_tail_len {
                match done.link.thread {
                    ThreadDecision::Continue { prefix_len, .. } => {
                        assert_eq!(prefix_len, prev, "step {i}");
                    }
                    ref other => panic!("step {i}: {other:?}"),
                }
            }
            assert_eq!(done.link.turn_no, 1, "step {i}");
            sessions.push(done.session_id.clone());
            threads.push(done.thread_id.clone());
            prev_tail_len = Some(input.len() + output.len());
            if i < 5 {
                input.extend(output);
                input.push(result(&format!("c{i}"), "ok"));
            }
        }
        assert!(sessions.iter().all(|s| *s == sessions[0]));
        assert!(threads.iter().all(|t| *t == threads[0]));
    }

    #[test]
    fn chain_survives_stripped_reasoning_and_new_ids() {
        let mut h = Harness::default();
        let keys = ClientKeys::default();
        let u1 = user("refactor the parser");
        let signed_reasoning = item(json!({
            "type": "reasoning", "id": "rs_a", "encrypted_content": "sig-123",
            "summary": [{"type": "summary_text", "text": "thinking"}]
        }));
        let msg_a = item(json!({
            "type": "message", "id": "msg_a", "role": "assistant",
            "content": [{"type": "output_text", "text": "Sure."}]
        }));
        let first = h.step(
            Some(Client::Codex),
            &keys,
            1000,
            std::slice::from_ref(&u1),
            &[signed_reasoning, msg_a],
        );
        let stripped_reasoning = item(json!({
            "type": "reasoning", "summary": [{"type": "summary_text", "text": "thinking"}]
        }));
        let msg_b = item(json!({
            "type": "message", "id": "msg_b", "role": "assistant",
            "content": [{"type": "output_text", "text": "Sure."}]
        }));
        let second = h.step(
            Some(Client::Codex),
            &keys,
            2000,
            &[u1, stripped_reasoning, msg_b, user("thanks")],
            &[asst("you are welcome")],
        );
        assert!(matches!(
            second.link.thread,
            ThreadDecision::Continue { .. }
        ));
        assert_eq!(second.session_id, first.session_id);
    }

    #[test]
    fn same_first_message_different_sessions_are_not_merged() {
        let mut h = Harness::default();
        let keys = ClientKeys::default();
        let a1 = h.step(
            Some(Client::Codex),
            &keys,
            1000,
            &[user("hi")],
            &[asst("hello from A")],
        );
        let b1 = h.step(
            Some(Client::Codex),
            &keys,
            1000,
            &[user("hi")],
            &[asst("hello from B")],
        );
        assert!(matches!(a1.link.session, SessionDecision::New { .. }));
        assert!(matches!(b1.link.session, SessionDecision::New { .. }));
        assert_ne!(a1.session_id, b1.session_id);

        let a2 = h.step(
            Some(Client::Codex),
            &keys,
            2000,
            &[user("hi"), asst("hello from A"), user("second A")],
            &[asst("ok")],
        );
        let b2 = h.step(
            Some(Client::Codex),
            &keys,
            2000,
            &[user("hi"), asst("hello from B"), user("second B")],
            &[asst("ok")],
        );
        assert_eq!(a2.session_id, a1.session_id);
        assert_eq!(b2.session_id, b1.session_id);
        assert!(matches!(b2.link.thread, ThreadDecision::Continue { .. }));
    }

    #[test]
    fn client_key_wins_and_reset_detected() {
        let mut h = Harness::default();
        let keys = keyed("codex:abc");
        let s1 = h.step(
            Some(Client::Codex),
            &keys,
            1000,
            &[user("start")],
            &[call("c1", "shell", "{}")],
        );
        assert!(matches!(
            s1.link.session,
            SessionDecision::New {
                key_source: KeySource::Client,
                ..
            }
        ));
        // Same client session, but the history was compacted and rewritten.
        let s2 = h.step(
            Some(Client::Codex),
            &keys,
            2000,
            &[user("compacted summary"), user("continue")],
            &[asst("ok")],
        );
        assert_eq!(
            s2.link.session,
            SessionDecision::Existing(s1.session_id.clone())
        );
        assert_eq!(
            s2.link.thread,
            ThreadDecision::New {
                kind: ThreadKind::Pending,
                parent_thread_id: None,
                spawn_call_id: None,
                reset: true,
            }
        );
    }

    #[test]
    fn subagent_thread_links_to_spawning_call() {
        let mut h = Harness::default();
        let keys = keyed("claude:xyz");
        let main = h.step(
            Some(Client::ClaudeCode),
            &keys,
            1000,
            &[user("fix the build")],
            &[call("task-1", "Task", r#"{"prompt":"检查测试"}"#)],
        );
        let sub = h.step(
            Some(Client::ClaudeCode),
            &keys,
            1100,
            &[user("检查测试")],
            &[asst("all green")],
        );
        assert_eq!(
            sub.link.session,
            SessionDecision::Existing(main.session_id.clone())
        );
        assert_eq!(
            sub.link.thread,
            ThreadDecision::New {
                kind: ThreadKind::Subagent,
                parent_thread_id: Some(main.thread_id.clone()),
                spawn_call_id: Some("task-1".into()),
                reset: false,
            }
        );
    }

    #[test]
    fn prompt_fp_matches_user_message_fp() {
        assert_eq!(prompt_fp("  检查测试 "), user("检查测试").fp);
    }

    #[test]
    fn new_user_message_opens_turn() {
        let mut h = Harness::default();
        let keys = ClientKeys::default();
        let s1 = h.step(
            Some(Client::Codex),
            &keys,
            1000,
            &[user("first")],
            &[call("c1", "shell", "{}")],
        );
        assert!(s1.link.opens_turn);
        assert_eq!(s1.link.turn_no, 1);
        let s2 = h.step(
            Some(Client::Codex),
            &keys,
            2000,
            &[
                user("first"),
                call("c1", "shell", "{}"),
                result("c1", "ok"),
                user("second"),
            ],
            &[asst("done")],
        );
        assert!(s2.link.opens_turn);
        assert_eq!(s2.link.turn_no, 2);
    }

    #[test]
    fn tool_results_only_do_not_open_turn() {
        let mut h = Harness::default();
        let keys = ClientKeys::default();
        let s1 = h.step(
            Some(Client::Codex),
            &keys,
            1000,
            &[user("first")],
            &[call("c1", "shell", "{}")],
        );
        let s2 = h.step(
            Some(Client::Codex),
            &keys,
            2000,
            &[user("first"), call("c1", "shell", "{}"), result("c1", "ok")],
            &[call("c2", "shell", "{}")],
        );
        assert_eq!(s2.session_id, s1.session_id);
        assert!(!s2.link.opens_turn);
        assert_eq!(s2.link.turn_no, 1);
    }

    #[test]
    fn codex_turn_id_controls_turns() {
        let mut h = Harness::default();
        let s1 = h.step(
            Some(Client::Codex),
            &with_turn("turn-1"),
            1000,
            &[user("q")],
            &[call("c1", "shell", "{}")],
        );
        assert_eq!(s1.link.turn_no, 1);
        let s2 = h.step(
            Some(Client::Codex),
            &with_turn("turn-1"),
            2000,
            &[user("q"), call("c1", "shell", "{}"), result("c1", "ok")],
            &[call("c2", "shell", "{}")],
        );
        assert_eq!(s2.link.turn_no, 1);
        assert!(!s2.link.opens_turn);
        let s3 = h.step(
            Some(Client::Codex),
            &with_turn("turn-2"),
            3000,
            &[
                user("q"),
                call("c1", "shell", "{}"),
                result("c1", "ok"),
                call("c2", "shell", "{}"),
                result("c2", "ok"),
            ],
            &[asst("done")],
        );
        assert_eq!(s3.link.turn_no, 2);
        assert!(s3.link.opens_turn);
    }

    #[test]
    fn checkpoint_every_64_steps() {
        let mut h = Harness::default();
        let keys = ClientKeys::default();
        let mut history = vec![user("long task")];
        let mut seen = Vec::new();
        for i in 1..=130i64 {
            let output = vec![call(&format!("c{i}"), "shell", "{}")];
            let done = h.step(Some(Client::Codex), &keys, 1000 * i, &history, &output);
            seen.push((done.link.checkpoint, done.link.depth));
            history.extend(output);
            history.push(result(&format!("c{i}"), "ok"));
        }
        for (idx, (checkpoint, depth)) in seen.iter().enumerate() {
            let step = idx + 1;
            if matches!(step, 1 | 65 | 129) {
                assert!(*checkpoint, "step {step}");
                assert_eq!(*depth, 0, "step {step}");
            } else {
                assert!(!*checkpoint, "step {step}");
                assert!((1..=63).contains(depth), "step {step}: depth {depth}");
            }
        }
    }

    #[test]
    fn chain_window_expires_after_12h() {
        let mut h = Harness::default();
        let keys = ClientKeys::default();
        let s1 = h.step(
            Some(Client::Codex),
            &keys,
            0,
            &[user("q")],
            &[call("c1", "shell", "{}")],
        );
        let s2 = h.step(
            Some(Client::Codex),
            &keys,
            13 * 3600 * 1000,
            &[user("q"), call("c1", "shell", "{}"), result("c1", "ok")],
            &[asst("done")],
        );
        assert_ne!(s2.session_id, s1.session_id);
        assert!(matches!(
            s2.link.session,
            SessionDecision::New {
                key_source: KeySource::New,
                ..
            }
        ));
    }

    #[test]
    fn chain_does_not_cross_clients() {
        let mut h = Harness::default();
        let keys = ClientKeys::default();
        let s1 = h.step(
            Some(Client::Codex),
            &keys,
            1000,
            &[user("q")],
            &[call("c1", "shell", "{}")],
        );
        let s2 = h.step(
            Some(Client::ClaudeCode),
            &keys,
            2000,
            &[user("q"), call("c1", "shell", "{}"), result("c1", "ok")],
            &[asst("done")],
        );
        assert_ne!(s2.session_id, s1.session_id);
        assert!(matches!(
            s2.link.session,
            SessionDecision::New {
                key_source: KeySource::New,
                ..
            }
        ));
    }
}
