//! Projection and session services (spec 7, 8.3.1): every request goes through a real
//! proxy into the services' own trajectory writer, then the session services read it back.

mod common;

use std::time::{Duration, Instant};

use common::*;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, Request, ResponseTemplate};
use yi_core::domain::{
    Client, LivePhase, Protocol, RecordKind, SessionListInput, SessionPage, SessionSummary,
    ThreadKindView, TrajectoryInput, TrajectoryPage, TrajectoryRecord, TrajectoryStep,
};
use yi_core::error::CoreError;
use yi_core::protocol::Usage;
use yi_core::protocol::ir::InputItem;
use yi_core::services::Services;
use yi_core::trajectory::normalize::normalize_item;

// ------------------------------------------------------------------ helpers

fn message_count(req: &Request) -> usize {
    let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
    body["messages"].as_array().map_or(0, Vec::len)
}

fn tool_use(call: &str) -> Value {
    json!({"id": format!("msg_{call}"), "type": "message", "role": "assistant",
        "model": "claude-up",
        "content": [{"type": "tool_use", "id": call, "name": "shell", "input": {"cmd": "ls"}}],
        "stop_reason": "tool_use", "stop_sequence": null,
        "usage": {"input_tokens": 3, "output_tokens": 2}})
}

fn text(t: &str) -> Value {
    json!({"id": "msg_t", "type": "message", "role": "assistant", "model": "claude-up",
        "content": [{"type": "text", "text": t}],
        "stop_reason": "end_turn", "stop_sequence": null,
        "usage": {"input_tokens": 3, "output_tokens": 2}})
}

fn user(t: &str) -> Value {
    json!({"role": "user", "content": t})
}

fn asst_text(t: &str) -> Value {
    json!({"role": "assistant", "content": t})
}

fn asst_call(call: &str) -> Value {
    json!({"role": "assistant", "content": [
        {"type": "tool_use", "id": call, "name": "shell", "input": {"cmd": "ls"}}]})
}

fn tool_result(call: &str) -> Value {
    json!({"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": call, "content": "ok"}]})
}

fn shell_tools(description: &str) -> Value {
    json!([{"name": "shell", "description": description,
        "input_schema": {"type": "object", "properties": {"cmd": {"type": "string"}}}}])
}

/// A Claude Code request body; `user_id` carries the client session key.
fn claude_body(messages: Vec<Value>, tools: Value, user_id: Option<&str>) -> Value {
    let mut body = json!({"model": CLAUDE_MODEL, "max_tokens": 64,
        "messages": messages, "tools": tools});
    if let Some(id) = user_id {
        body["metadata"] = json!({"user_id": id});
    }
    body
}

/// Turn `n` (1-based) of the text conversation: `q1 a1 … q(n-1) a(n-1) qn`.
fn history(n: usize) -> Vec<Value> {
    let mut messages = Vec::new();
    for k in 1..n {
        messages.push(user(&format!("q{k}")));
        messages.push(asst_text(&format!("a{k}")));
    }
    messages.push(user(&format!("q{n}")));
    messages
}

/// Replies `a{n}` where `n` is the turn number implied by the request length.
async fn mount_numbered_answers(env: &TestEnv) {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(|req: &Request| {
            let turn = message_count(req).div_ceil(2);
            ResponseTemplate::new(200).set_body_json(text(&format!("a{turn}")))
        })
        .mount(&env.mock)
        .await;
}

fn no_filter() -> SessionListInput {
    SessionListInput {
        client: None,
        query: None,
        model: None,
        start_ms: None,
        end_ms: None,
        cursor: None,
        limit: None,
    }
}

/// Follows the cursor until the last page, with `size` sessions per page.
async fn paginate(env: &TestEnv, base: SessionListInput, size: u32) -> Vec<SessionSummary> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page: SessionPage = env
            .services
            .list_sessions(SessionListInput {
                cursor: cursor.clone(),
                limit: Some(size),
                ..base.clone()
            })
            .await
            .unwrap();
        assert!(page.items.len() <= size as usize);
        out.extend(page.items);
        assert!(out.len() < 10_000, "pagination does not terminate");
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => return out,
        }
    }
}

async fn all_sessions(env: &TestEnv) -> Vec<SessionSummary> {
    paginate(env, no_filter(), 50).await
}

fn ids(sessions: &[SessionSummary]) -> Vec<String> {
    sessions.iter().map(|s| s.id.clone()).collect()
}

/// Waits until the writer has committed at least `min` steps in total.
async fn wait_steps(env: &TestEnv, min: u32) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let total: u32 = all_sessions(env).await.iter().map(|s| s.step_count).sum();
        if total >= min {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "only {total} of {min} steps committed"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn only_session(env: &TestEnv) -> String {
    let sessions = all_sessions(env).await;
    assert_eq!(sessions.len(), 1, "expected one session");
    sessions[0].id.clone()
}

async fn trajectory(
    env: &TestEnv,
    session: &str,
    thread: Option<&str>,
    before: Option<u32>,
    limit: Option<u32>,
) -> TrajectoryPage {
    env.services
        .get_trajectory(TrajectoryInput {
            session_id: session.into(),
            thread_id: thread.map(Into::into),
            before_turn: before,
            limit_turns: limit,
        })
        .await
        .unwrap()
}

fn steps_of(page: &TrajectoryPage) -> Vec<&TrajectoryStep> {
    page.turns.iter().flat_map(|t| t.steps.iter()).collect()
}

fn records_of(page: &TrajectoryPage) -> Vec<&TrajectoryRecord> {
    steps_of(page)
        .into_iter()
        .flat_map(|s| s.records.iter())
        .collect()
}

fn fp_of_value(v: &Value) -> [u8; 32] {
    let item: InputItem = serde_json::from_value(v.clone()).unwrap();
    normalize_item(&item).fp
}

fn fp_of_json(s: &str) -> [u8; 32] {
    let item: InputItem = serde_json::from_str(s).unwrap();
    normalize_item(&item).fp
}

async fn mount_all_upstreams(env: &TestEnv) {
    for p in [
        Protocol::Anthropic,
        Protocol::OpenaiChat,
        Protocol::Responses,
    ] {
        Mock::given(method("POST"))
            .and(path(upstream_path(p)))
            .respond_with(ResponseTemplate::new(200).set_body_json(upstream_response(p)))
            .mount(&env.mock)
            .await;
    }
}

/// A one-message request whose only user text is `title`, in the protocol's own shape.
fn session_body(p: Protocol, title: &str) -> Value {
    // Each body carries a tool: a tool-less single step is a side thread, and a session
    // made only of side threads is left out of the list.
    match p {
        Protocol::Anthropic => json!({"model": CLAUDE_MODEL, "max_tokens": 16,
            "messages": [{"role": "user", "content": title}],
            "tools": shell_tools("run a command")}),
        Protocol::OpenaiChat => json!({"model": CHAT_MODEL,
            "messages": [{"role": "user", "content": title}],
            "tools": [{"type": "function", "function": {"name": "shell",
                "parameters": {"type": "object", "properties": {"cmd": {"type": "string"}}}}}]}),
        Protocol::Responses => json!({"model": RESP_MODEL, "input": title,
            "tools": [{"type": "function", "name": "shell",
                "parameters": {"type": "object", "properties": {"cmd": {"type": "string"}}}}]}),
    }
}

// ------------------------------------------------------------------- tests

#[tokio::test]
async fn projection_pairs_tool_call_with_result() {
    let env = TestEnv::start_capturing().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(|req: &Request| {
            let reply = if message_count(req) == 1 {
                tool_use("toolu_1")
            } else {
                text("done")
            };
            ResponseTemplate::new(200).set_body_json(reply)
        })
        .mount(&env.mock)
        .await;
    let route = env.route_to(Protocol::Anthropic, Protocol::Anthropic);
    let tools = shell_tools("run a shell command");
    let first = env
        .post(
            &route,
            &claude_body(vec![user("list the files")], tools.clone(), None),
        )
        .await;
    assert_eq!(first.status(), 200);
    let second = env
        .post(
            &route,
            &claude_body(
                vec![
                    user("list the files"),
                    asst_call("toolu_1"),
                    tool_result("toolu_1"),
                ],
                tools,
                None,
            ),
        )
        .await;
    assert_eq!(second.status(), 200);
    wait_steps(&env, 2).await;

    let session = only_session(&env).await;
    let page = trajectory(&env, &session, None, None, None).await;
    let steps = steps_of(&page);
    assert_eq!(steps.len(), 2);
    let tools_rows: Vec<&TrajectoryRecord> = records_of(&page)
        .into_iter()
        .filter(|r| r.kind == RecordKind::Tool)
        .collect();
    assert_eq!(tools_rows.len(), 1, "one tool row for the call");
    let tool = tools_rows[0];
    assert_eq!(tool.id, "call:toolu_1");
    assert_eq!(tool.result_step_id.as_deref(), Some(steps[1].id.as_str()));
    assert!(tool.duration_estimated);
    assert!(tool.result.is_some());
}

#[tokio::test]
async fn system_record_on_first_step_and_on_change() {
    let env = TestEnv::start_capturing().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(|req: &Request| {
            let reply = match message_count(req) {
                1 => tool_use("toolu_1"),
                3 => tool_use("toolu_2"),
                _ => text("done"),
            };
            ResponseTemplate::new(200).set_body_json(reply)
        })
        .mount(&env.mock)
        .await;
    let route = env.route_to(Protocol::Anthropic, Protocol::Anthropic);
    let tools_a = shell_tools("first definition");
    let tools_b = shell_tools("second definition");
    let bodies = [
        claude_body(vec![user("list the files")], tools_a.clone(), None),
        claude_body(
            vec![
                user("list the files"),
                asst_call("toolu_1"),
                tool_result("toolu_1"),
            ],
            tools_a,
            None,
        ),
        claude_body(
            vec![
                user("list the files"),
                asst_call("toolu_1"),
                tool_result("toolu_1"),
                asst_call("toolu_2"),
                tool_result("toolu_2"),
            ],
            tools_b,
            None,
        ),
    ];
    for body in &bodies {
        assert_eq!(env.post(&route, body).await.status(), 200);
    }
    wait_steps(&env, 3).await;

    let session = only_session(&env).await;
    let page = trajectory(&env, &session, None, None, None).await;
    let steps = steps_of(&page);
    assert_eq!(steps.len(), 3);
    let system: Vec<&TrajectoryRecord> = records_of(&page)
        .into_iter()
        .filter(|r| r.kind == RecordKind::System)
        .collect();
    assert_eq!(system.len(), 2, "first step and the tool change");
    assert!(!system[0].system_changed);
    assert!(system[1].system_changed);
    assert_eq!(system[0].step_id, steps[0].id);
    assert_eq!(system[1].step_id, steps[2].id);
}

#[tokio::test]
async fn subagent_is_projected_as_subtool() {
    let env = TestEnv::start_capturing().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(|req: &Request| {
            let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
            let reply = if body["messages"][0]["content"] == "fix the build" {
                json!({"id": "msg_main", "type": "message", "role": "assistant",
                    "model": "claude-up",
                    "content": [{"type": "tool_use", "id": "task-1", "name": "Task",
                        "input": {"prompt": "检查测试"}}],
                    "stop_reason": "tool_use", "stop_sequence": null,
                    "usage": {"input_tokens": 3, "output_tokens": 2}})
            } else {
                text("all green")
            };
            ResponseTemplate::new(200).set_body_json(reply)
        })
        .mount(&env.mock)
        .await;
    let route = env.route_to(Protocol::Anthropic, Protocol::Anthropic);
    let user_id = "user_abc_account_def_session_0b6e5f3a-1c2d-4e5f-8a9b-0c1d2e3f4a5b";
    let main = claude_body(vec![user("fix the build")], json!([]), Some(user_id));
    let sub = claude_body(vec![user("检查测试")], json!([]), Some(user_id));
    assert_eq!(env.post(&route, &main).await.status(), 200);
    assert_eq!(env.post(&route, &sub).await.status(), 200);
    wait_steps(&env, 2).await;

    let session = only_session(&env).await;
    let detail = env.services.get_session(session.clone()).await.unwrap();
    let main_thread = detail
        .threads
        .iter()
        .find(|t| t.kind == ThreadKindView::Main)
        .expect("main thread");
    let sub_thread = detail
        .threads
        .iter()
        .find(|t| t.kind == ThreadKindView::Subagent)
        .expect("subagent thread");
    assert_eq!(sub_thread.spawn_call_id.as_deref(), Some("task-1"));

    let page = trajectory(&env, &session, Some(&main_thread.id), None, None).await;
    let subtools: Vec<&TrajectoryRecord> = records_of(&page)
        .into_iter()
        .filter(|r| r.kind == RecordKind::Subtool)
        .collect();
    assert_eq!(subtools.len(), 1);
    assert_eq!(
        subtools[0].sub_thread_id.as_deref(),
        Some(sub_thread.id.as_str())
    );
    assert_eq!(subtools[0].tool_name.as_deref(), Some("Task"));
}

#[tokio::test]
async fn live_placeholder_shows_on_main_thread_only() {
    let env = TestEnv::start_capturing().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(|req: &Request| {
            let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
            let reply = if body["messages"][0]["content"] == "fix the build" {
                json!({"id": "msg_main", "type": "message", "role": "assistant",
                    "model": "claude-up",
                    "content": [{"type": "tool_use", "id": "task-2", "name": "Task",
                        "input": {"prompt": "检查测试"}}],
                    "stop_reason": "tool_use", "stop_sequence": null,
                    "usage": {"input_tokens": 3, "output_tokens": 2}})
            } else {
                text("all green")
            };
            ResponseTemplate::new(200).set_body_json(reply)
        })
        .mount(&env.mock)
        .await;
    let route = env.route_to(Protocol::Anthropic, Protocol::Anthropic);
    let user_id = "user_abc_account_def_session_1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
    let main = claude_body(vec![user("fix the build")], json!([]), Some(user_id));
    let sub = claude_body(vec![user("检查测试")], json!([]), Some(user_id));
    assert_eq!(env.post(&route, &main).await.status(), 200);
    assert_eq!(env.post(&route, &sub).await.status(), 200);
    wait_steps(&env, 2).await;

    let session = only_session(&env).await;
    let detail = env.services.get_session(session.clone()).await.unwrap();
    let main_id = detail
        .threads
        .iter()
        .find(|t| t.kind == ThreadKindView::Main)
        .expect("main thread")
        .id
        .clone();
    let sub_id = detail
        .threads
        .iter()
        .find(|t| t.kind == ThreadKindView::Subagent)
        .expect("subagent thread")
        .id
        .clone();

    env.state
        .live
        .begin("live-main-1", Some(Client::ClaudeCode), CLAUDE_MODEL, "pa", 1_000);
    env.state.live.set_session("live-main-1", &session);

    let on_main = trajectory(&env, &session, Some(&main_id), None, None).await;
    let last = steps_of(&on_main).last().expect("steps").id.clone();
    assert_eq!(last, "live-main-1");

    let on_sub = trajectory(&env, &session, Some(&sub_id), None, None).await;
    assert!(
        steps_of(&on_sub).iter().all(|s| s.id != "live-main-1"),
        "placeholder must not appear on a subagent thread"
    );

    env.state
        .live
        .finish("live-main-1", LivePhase::Done, None, Usage::default(), None);
}

#[tokio::test]
async fn reset_inserts_compacted_record() {
    let env = TestEnv::start_capturing().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(|req: &Request| {
            let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
            let reply = if body["messages"][0]["content"] == "fix the build" {
                tool_use("toolu_a")
            } else {
                text("fresh start")
            };
            ResponseTemplate::new(200).set_body_json(reply)
        })
        .mount(&env.mock)
        .await;
    let route = env.route_to(Protocol::Anthropic, Protocol::Anthropic);
    let user_id = "user_abc_account_def_session_9f8e7d6c-5b4a-4321-8fed-cba987654321";
    let first = claude_body(vec![user("fix the build")], json!([]), Some(user_id));
    let second = claude_body(vec![user("start over")], json!([]), Some(user_id));
    assert_eq!(env.post(&route, &first).await.status(), 200);
    assert_eq!(env.post(&route, &second).await.status(), 200);
    wait_steps(&env, 2).await;

    let session = only_session(&env).await;
    let detail = env.services.get_session(session.clone()).await.unwrap();
    let reset = detail
        .threads
        .iter()
        .find(|t| t.reset)
        .expect("a reset thread");
    let page = trajectory(&env, &session, Some(&reset.id), None, None).await;
    let compacted = records_of(&page)
        .into_iter()
        .filter(|r| r.kind == RecordKind::Compacted)
        .count();
    assert_eq!(compacted, 1);
}

#[tokio::test]
async fn pagination_by_turns() {
    let env = TestEnv::start_capturing().await;
    mount_numbered_answers(&env).await;
    let route = env.route_to(Protocol::Anthropic, Protocol::Anthropic);
    for n in 1..=25 {
        let body = claude_body(history(n), shell_tools("run a command"), None);
        assert_eq!(env.post(&route, &body).await.status(), 200, "turn {n}");
    }
    wait_steps(&env, 25).await;

    let session = only_session(&env).await;
    let newest = trajectory(&env, &session, None, None, None).await;
    let turns: Vec<u32> = newest.turns.iter().map(|t| t.turn_no).collect();
    assert_eq!(turns, (6..=25).collect::<Vec<u32>>());
    assert!(newest.has_older);

    let older = trajectory(&env, &session, None, Some(6), None).await;
    let turns: Vec<u32> = older.turns.iter().map(|t| t.turn_no).collect();
    assert_eq!(turns, (1..=5).collect::<Vec<u32>>());
    assert!(!older.has_older);
}

#[tokio::test]
async fn long_session_reconstruction_is_bounded_by_checkpoints() {
    let env = TestEnv::start_capturing().await;
    mount_numbered_answers(&env).await;
    let route = env.route_to(Protocol::Anthropic, Protocol::Anthropic);
    for n in 1..=200 {
        let body = claude_body(history(n), shell_tools("run a command"), None);
        assert_eq!(env.post(&route, &body).await.status(), 200, "step {n}");
    }
    wait_steps(&env, 200).await;

    let session = only_session(&env).await;
    let page = trajectory(&env, &session, None, None, Some(200)).await;
    let steps = steps_of(&page);
    assert_eq!(steps.len(), 200);
    for n in [1usize, 2, 64, 65, 66, 130, 200] {
        let step = steps[n - 1];
        assert_eq!(step.number as usize, n);
        let ctx = env
            .services
            .get_step_context(step.id.clone())
            .await
            .unwrap();
        assert!(ctx.chain_depth <= 64, "step {n}: depth {}", ctx.chain_depth);
        let expected = history(n);
        assert_eq!(ctx.input.len(), expected.len(), "step {n} input count");
        for (i, (got, want)) in ctx.input.iter().zip(&expected).enumerate() {
            assert_eq!(
                fp_of_json(&got.json),
                fp_of_value(want),
                "step {n} item {i}"
            );
        }
    }
}

#[tokio::test]
async fn live_request_appears_as_placeholder_step() {
    let env = TestEnv::start_capturing().await;
    // The first step calls a tool, so its thread is the Main thread, where placeholders show.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(|req: &Request| {
            let reply = if message_count(req) == 1 {
                tool_use("toolu_live")
            } else {
                text("done")
            };
            ResponseTemplate::new(200).set_body_json(reply)
        })
        .mount(&env.mock)
        .await;
    let route = env.route_to(Protocol::Anthropic, Protocol::Anthropic);
    assert_eq!(
        env.post(&route, &claude_body(history(1), shell_tools("run a command"), None))
            .await
            .status(),
        200
    );
    wait_steps(&env, 1).await;
    let session = only_session(&env).await;

    env.state.live.begin(
        "live-step-1",
        Some(Client::ClaudeCode),
        CLAUDE_MODEL,
        "pa",
        1_000,
    );
    env.state.live.set_session("live-step-1", &session);

    let page = trajectory(&env, &session, None, None, None).await;
    let steps = steps_of(&page);
    let last = steps.last().expect("placeholder step");
    assert_eq!(last.id, "live-step-1");
    assert_eq!(last.number, 2);
    assert_eq!(last.live_phase, Some(LivePhase::Waiting));
    assert!(last.records.is_empty());
    assert!(all_sessions(&env).await[0].live);

    env.state
        .live
        .finish("live-step-1", LivePhase::Done, None, Usage::default(), None);
}

/// A session made only of side threads (such as a title request sent with its own client
/// key) is not listed; once its thread gets a second step it is listed and titled.
#[tokio::test]
async fn side_only_session_is_hidden_until_continued() {
    let env = TestEnv::start_capturing().await;
    mount_numbered_answers(&env).await;
    let route = env.route_to(Protocol::Anthropic, Protocol::Anthropic);
    let side = claude_body(
        history(1),
        Value::Null,
        Some("user_a_session_0b6e5f3a-1c2d-4e5f-8a9b-000000000001"),
    );
    assert_eq!(env.post(&route, &side).await.status(), 200);
    let main = claude_body(
        vec![user("main task")],
        shell_tools("run a command"),
        Some("user_a_session_0b6e5f3a-1c2d-4e5f-8a9b-000000000002"),
    );
    assert_eq!(env.post(&route, &main).await.status(), 200);
    wait_steps(&env, 1).await;

    let listed = all_sessions(&env).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].title, "main task");

    let next = claude_body(
        history(2),
        Value::Null,
        Some("user_a_session_0b6e5f3a-1c2d-4e5f-8a9b-000000000001"),
    );
    assert_eq!(env.post(&route, &next).await.status(), 200);
    wait_steps(&env, 3).await;
    let listed = all_sessions(&env).await;
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().any(|s| s.title == "q1" && s.step_count == 2));
}

/// A plain chat client on the global route sends no tools, so its single turn is a
/// conversation and is listed with its title.
#[tokio::test]
async fn global_route_single_turn_without_tools_is_listed() {
    let env = TestEnv::start_capturing().await;
    mount_numbered_answers(&env).await;
    let body = claude_body(history(1), Value::Null, None);
    assert_eq!(env.post("/v1/messages", &body).await.status(), 200);
    wait_steps(&env, 1).await;

    let listed = all_sessions(&env).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].client, None);
    assert_eq!(listed[0].title, "q1");
}

#[tokio::test]
async fn list_sessions_keyset_pagination_and_filters() {
    let env = TestEnv::start_capturing().await;
    mount_all_upstreams(&env).await;
    let protocols = [
        Protocol::Anthropic,
        Protocol::OpenaiChat,
        Protocol::Responses,
    ];
    for i in 0..25 {
        let p = protocols[i % 3];
        let tag = if i % 2 == 0 { "beta" } else { "alpha" };
        let title = format!("session {i:02} {tag}");
        let route = env.route_to(p, p);
        assert_eq!(
            env.post(&route, &session_body(p, &title)).await.status(),
            200
        );
    }
    wait_steps(&env, 25).await;

    let all = all_sessions(&env).await;
    assert_eq!(all.len(), 25);
    assert!(
        all.windows(2)
            .all(|w| (w[0].last_at, &w[0].id) >= (w[1].last_at, &w[1].id)),
        "newest first, ties broken by id"
    );
    // Keyset pages reproduce the whole listing exactly: no duplicates, no gaps.
    assert_eq!(ids(&paginate(&env, no_filter(), 4).await), ids(&all));

    let first_page = env
        .services
        .list_sessions(SessionListInput {
            limit: Some(4),
            ..no_filter()
        })
        .await
        .unwrap();
    let cursor = first_page.next_cursor.expect("more pages");
    let (last_at, _) = cursor.split_once(':').expect("cursor is {last_at}:{id}");
    assert_eq!(last_at, first_page.items[3].last_at.to_string());

    let filtered = |pred: &dyn Fn(&SessionSummary) -> bool| -> Vec<String> {
        all.iter()
            .filter(|s| pred(s))
            .map(|s| s.id.clone())
            .collect()
    };
    let by_client = SessionListInput {
        client: Some(Client::Codex),
        ..no_filter()
    };
    let got = paginate(&env, by_client, 3).await;
    assert!(!got.is_empty());
    assert!(got.iter().all(|s| s.client == Some(Client::Codex)));
    assert_eq!(
        ids(&got),
        filtered(&|s: &SessionSummary| s.client == Some(Client::Codex))
    );

    let by_title = SessionListInput {
        query: Some("BETA".into()),
        ..no_filter()
    };
    assert_eq!(
        ids(&paginate(&env, by_title, 3).await),
        filtered(&|s: &SessionSummary| s.title.to_lowercase().contains("beta"))
    );

    let by_model = SessionListInput {
        model: Some(CHAT_MODEL.into()),
        ..no_filter()
    };
    assert_eq!(
        ids(&paginate(&env, by_model, 3).await),
        filtered(&|s: &SessionSummary| s.models.iter().any(|m| m == CHAT_MODEL))
    );

    let mid = all[12].last_at;
    let by_range = SessionListInput {
        end_ms: Some(mid),
        ..no_filter()
    };
    assert_eq!(
        ids(&paginate(&env, by_range, 3).await),
        filtered(&|s: &SessionSummary| s.last_at < mid)
    );

    let combined = SessionListInput {
        client: Some(Client::ClaudeCode),
        query: Some("alpha".into()),
        ..no_filter()
    };
    assert_eq!(
        ids(&paginate(&env, combined, 2).await),
        filtered(
            &|s: &SessionSummary| s.client == Some(Client::ClaudeCode) && s.title.contains("alpha")
        )
    );
}

#[tokio::test]
async fn session_changes_returns_sessions_after_seq() {
    let env = TestEnv::start_capturing().await;
    mount_all_upstreams(&env).await;
    let route = env.route_to(Protocol::Anthropic, Protocol::Anthropic);
    assert_eq!(
        env.post(&route, &session_body(Protocol::Anthropic, "first session"))
            .await
            .status(),
        200
    );
    wait_steps(&env, 1).await;
    let first = only_session(&env).await;

    let c0 = env.services.session_changes(0).await.unwrap();
    assert_eq!(c0.session_ids, vec![first.clone()]);
    assert!(c0.seq > 0);

    assert_eq!(
        env.post(&route, &session_body(Protocol::Anthropic, "second session"))
            .await
            .status(),
        200
    );
    wait_steps(&env, 2).await;
    let second = all_sessions(&env)
        .await
        .into_iter()
        .map(|s| s.id)
        .find(|id| *id != first)
        .expect("second session");

    let c1 = env.services.session_changes(c0.seq).await.unwrap();
    assert_eq!(c1.session_ids, vec![second]);
    assert!(c1.seq > c0.seq);

    let none = env.services.session_changes(c1.seq).await.unwrap();
    assert!(none.session_ids.is_empty());
}

#[tokio::test]
async fn clear_all_recreates_db_and_capture_continues() {
    let env = TestEnv::start_capturing().await;
    mount_all_upstreams(&env).await;
    let route = env.route_to(Protocol::Anthropic, Protocol::Anthropic);
    assert_eq!(
        env.post(&route, &session_body(Protocol::Anthropic, "before clear"))
            .await
            .status(),
        200
    );
    wait_steps(&env, 1).await;

    assert_eq!(env.services.clear_sessions(None).await.unwrap(), 1);
    assert!(all_sessions(&env).await.is_empty());
    assert!(env.services.trajectory_status().await.unwrap().available);

    assert_eq!(
        env.post(&route, &session_body(Protocol::Anthropic, "after clear"))
            .await
            .status(),
        200
    );
    wait_steps(&env, 1).await;
    let sessions = all_sessions(&env).await;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].title, "after clear");
}

#[tokio::test]
async fn delete_session_evicts_tail_so_next_step_starts_new_session() {
    let env = TestEnv::start_capturing().await;
    mount_numbered_answers(&env).await;
    let route = env.route_to(Protocol::Anthropic, Protocol::Anthropic);
    assert_eq!(
        env.post(&route, &claude_body(history(1), shell_tools("run a command"), None))
            .await
            .status(),
        200
    );
    wait_steps(&env, 1).await;
    let doomed = only_session(&env).await;

    env.services.delete_session(doomed.clone()).await.unwrap();
    assert!(matches!(
        env.services.get_session(doomed.clone()).await,
        Err(CoreError::NotFound(_))
    ));

    // The next request continues the deleted conversation. Its tail must be gone from
    // the writer, so the step opens a new session instead of failing on a missing one.
    assert_eq!(
        env.post(&route, &claude_body(history(2), shell_tools("run a command"), None))
            .await
            .status(),
        200
    );
    wait_steps(&env, 1).await;
    let sessions = all_sessions(&env).await;
    assert_eq!(sessions.len(), 1);
    assert_ne!(sessions[0].id, doomed);
    assert_eq!(sessions[0].step_count, 1);
}

#[tokio::test]
async fn clear_client_removes_only_that_client() {
    let env = TestEnv::start_capturing().await;
    mount_all_upstreams(&env).await;
    let claude = env.route_to(Protocol::Anthropic, Protocol::Anthropic);
    let codex = env.route_to(Protocol::Responses, Protocol::Responses);
    assert_eq!(
        env.post(&claude, &session_body(Protocol::Anthropic, "claude side"))
            .await
            .status(),
        200
    );
    assert_eq!(
        env.post(&codex, &session_body(Protocol::Responses, "codex side"))
            .await
            .status(),
        200
    );
    wait_steps(&env, 2).await;

    assert_eq!(
        env.services
            .clear_sessions(Some(Client::Codex))
            .await
            .unwrap(),
        1
    );
    let left = all_sessions(&env).await;
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].client, Some(Client::ClaudeCode));
}

#[tokio::test]
async fn session_changes_survives_deleting_newest_session() {
    let env = TestEnv::start_capturing().await;
    mount_all_upstreams(&env).await;
    let route = env.route_to(Protocol::Anthropic, Protocol::Anthropic);
    assert_eq!(
        env.post(&route, &session_body(Protocol::Anthropic, "kept session"))
            .await
            .status(),
        200
    );
    wait_steps(&env, 1).await;
    assert_eq!(
        env.post(&route, &session_body(Protocol::Anthropic, "newest session"))
            .await
            .status(),
        200
    );
    wait_steps(&env, 2).await;
    let sessions = all_sessions(&env).await;
    let newest = sessions
        .iter()
        .find(|s| s.title == "newest session")
        .expect("newest session")
        .id
        .clone();
    let cursor = env.services.session_changes(0).await.unwrap().seq;

    // Deleting the newest session must not hand its sequence number to a later step.
    env.services.delete_session(newest).await.unwrap();
    assert_eq!(
        env.post(&route, &session_body(Protocol::Anthropic, "later session"))
            .await
            .status(),
        200
    );
    wait_steps(&env, 2).await;
    let later = all_sessions(&env)
        .await
        .into_iter()
        .find(|s| s.title == "later session")
        .expect("later session")
        .id;

    let changes = env.services.session_changes(cursor).await.unwrap();
    assert!(changes.session_ids.contains(&later), "{changes:?}");
    assert!(changes.seq > cursor, "{changes:?} vs cursor {cursor}");
}

#[tokio::test]
async fn unavailable_trajectory_returns_conflict_until_rebuilt() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(yi_core::paths::TRAJECTORY_DB_FILE), b"not a database").unwrap();
    let services = Services::for_test(dir.path());

    let status = services.trajectory_status().await.unwrap();
    assert!(!status.available);
    assert!(status.error.as_deref().is_some_and(|e| !e.is_empty()));

    match services.list_sessions(no_filter()).await {
        Err(CoreError::Conflict(msg)) => {
            assert!(msg.starts_with("轨迹库不可用："), "{msg}");
        }
        other => panic!("expected a conflict, got {other:?}"),
    }

    // Settings stay writable while the library is down.
    let mut settings = status.settings.clone();
    settings.notice_seen = true;
    services.save_trajectory_settings(settings).await.unwrap();

    services.rebuild_trajectory_db().await.unwrap();
    let status = services.trajectory_status().await.unwrap();
    assert!(status.available);
    assert!(status.error.is_none());
    assert!(status.settings.notice_seen);
    let kept = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(&format!("{}.broken-", yi_core::paths::TRAJECTORY_DB_FILE))
        });
    assert!(kept, "the unreadable file is renamed, not deleted");
    assert!(
        services
            .list_sessions(no_filter())
            .await
            .unwrap()
            .items
            .is_empty()
    );
}
