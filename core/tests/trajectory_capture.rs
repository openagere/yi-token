//! End-to-end trajectory capture: a real proxy in front of a wiremock upstream,
//! with the trajectory writer recording into an in-memory trajectory database.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, Request, Respond, ResponseTemplate};
use yi_core::domain::{LivePhase, Protocol, TrajectorySettings};
use yi_core::storage::Db;
use yi_core::storage::trajectory::TrajectoryDb;
use yi_core::storage::trajectory::blobs;
use yi_core::storage::trajectory::writer::{
    TrajectoryWriterHandle, spawn_trajectory_writer, spawn_trajectory_writer_with_limit,
};
use yi_core::trajectory::record::CaptureSink;

/// A proxy with a trajectory writer attached.
struct Capture {
    env: TestEnv,
    /// The trajectory database the writer records into.
    traj: Db,
    writer: TrajectoryWriterHandle,
}

/// Mounts a plain JSON upstream answer for `p`.
async fn mount_json(env: &TestEnv, p: Protocol) {
    Mock::given(method("POST"))
        .and(path(upstream_path(p)))
        .respond_with(ResponseTemplate::new(200).set_body_json(upstream_response(p)))
        .mount(&env.mock)
        .await;
}

async fn start(limit: Option<usize>) -> Capture {
    start_with(limit, TrajectorySettings::default()).await
}

async fn start_with(limit: Option<usize>, settings: TrajectorySettings) -> Capture {
    let env = TestEnv::start().await;
    let traj = TrajectoryDb::open_in_memory().unwrap();
    let traj_db = traj.0.clone();
    let main = env.services.db().clone();
    let live = env.state.live.clone();
    let (sink, writer) = match limit {
        Some(bytes) => spawn_trajectory_writer_with_limit(traj, main, live, settings, bytes),
        None => spawn_trajectory_writer(traj, main, live, settings),
    };
    let sink: Arc<dyn CaptureSink> = sink;
    env.state.capture.set(Some(sink));
    Capture {
        env,
        traj: traj_db,
        writer,
    }
}

impl Capture {
    fn traj_count(&self, sql: &str) -> i64 {
        self.traj
            .with_conn(|c| Ok(c.query_row(sql, [], |r| r.get(0)).unwrap()))
            .unwrap()
    }

    fn main_count(&self, sql: &str) -> i64 {
        self.env
            .services
            .db()
            .with_conn(|c| Ok(c.query_row(sql, [], |r| r.get(0)).unwrap()))
            .unwrap()
    }

    /// Waits until the proxy has finished every request and the writer has
    /// committed at least `min` steps.
    async fn settle_steps(&self, min: i64) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let idle = self.env.state.live.snapshot(0).in_flight.is_empty();
            if idle {
                self.writer.flush_for_test();
            }
            let n = self.traj_count("SELECT COUNT(*) FROM steps");
            if (idle && n >= min) || Instant::now() >= deadline {
                assert!(n >= min, "expected at least {min} steps, found {n}");
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

/// Text of the output message items of a step, in output order.
fn output_text(cap: &Capture, step_id: &str) -> String {
    let hashes: Vec<(String, String)> = cap
        .traj
        .with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT kind, blob FROM step_items WHERE step_id = ?1 AND side = 'output' ORDER BY ord",
            )?;
            let rows = stmt
                .query_map([step_id], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .unwrap();
    let mut text = String::new();
    for (kind, hash) in hashes {
        if kind != "message" {
            continue;
        }
        let stored = cap
            .traj
            .with_conn(|c| blobs::get(c, &hash))
            .unwrap()
            .expect("output blob");
        let v: Value = serde_json::from_str(&stored.json).unwrap();
        match &v["content"] {
            Value::String(s) => text.push_str(s),
            Value::Array(parts) => {
                for p in parts {
                    if let Some(s) = p.get("text").and_then(Value::as_str) {
                        text.push_str(s);
                    }
                }
            }
            _ => {}
        }
    }
    text
}

fn step_id_of(cap: &Capture) -> String {
    cap.traj
        .with_conn(|c| {
            Ok(
                c.query_row("SELECT id FROM steps ORDER BY seq LIMIT 1", [], |r| {
                    r.get(0)
                })?,
            )
        })
        .unwrap()
}

// ------------------------------------------------------- tool loop upstream

/// Upstream for the five-step tool loop. The number of messages in the request
/// tells which step this is: steps 1 to 4 call the `shell` tool, step 5 answers.
struct ToolLoop;

impl Respond for ToolLoop {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
        let messages = body["messages"].as_array().map_or(0, Vec::len);
        let step = messages.div_ceil(2);
        let reply = if step < 5 {
            json!({
                "id": format!("msg_{step}"), "type": "message", "role": "assistant",
                "model": "claude-up",
                "content": [{"type": "tool_use", "id": format!("toolu_{step}"),
                    "name": "shell", "input": {"cmd": "ls"}}],
                "stop_reason": "tool_use", "stop_sequence": null,
                "usage": {"input_tokens": 3, "output_tokens": 2}})
        } else {
            json!({
                "id": "msg_5", "type": "message", "role": "assistant",
                "model": "claude-up",
                "content": [{"type": "text", "text": "done"}],
                "stop_reason": "end_turn", "stop_sequence": null,
                "usage": {"input_tokens": 3, "output_tokens": 2}})
        };
        ResponseTemplate::new(200).set_body_json(reply)
    }
}

/// The request for loop step `step` (1-based): the user prompt, then every earlier
/// tool call with its result, the way Claude Code replays history.
fn loop_request(step: usize, tools: Value) -> Value {
    let mut messages = vec![json!({"role": "user", "content": "list the files"})];
    for j in 1..step {
        messages.push(json!({"role": "assistant", "content": [
            {"type": "tool_use", "id": format!("toolu_{j}"), "name": "shell", "input": {"cmd": "ls"}}]}));
        messages.push(json!({"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": format!("toolu_{j}"), "content": "ok"}]}));
    }
    json!({"model": CLAUDE_MODEL, "max_tokens": 64, "messages": messages, "tools": tools})
}

async fn run_tool_loop(cap: &Capture, tools: Value) {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ToolLoop)
        .mount(&cap.env.mock)
        .await;
    let route = cap.env.route_to(Protocol::Anthropic, Protocol::Anthropic);
    for step in 1..=5 {
        let resp = cap
            .env
            .post(&route, &loop_request(step, tools.clone()))
            .await;
        assert_eq!(resp.status(), 200, "step {step}");
    }
}

/// One tool definition. A request with tools is a real client turn, not a side
/// request, so fixtures that need recording send it.
fn shell_tools() -> Value {
    json!([{
        "name": "shell",
        "description": "run a command",
        "input_schema": {"type": "object", "properties": {"cmd": {"type": "string"}}}
    }])
}

// ----------------------------------------------------------------- tests

#[tokio::test]
async fn tool_loop_writes_linear_storage() {
    let cap = start(None).await;
    run_tool_loop(&cap, shell_tools()).await;
    cap.settle_steps(5).await;

    assert_eq!(cap.traj_count("SELECT COUNT(*) FROM sessions"), 1);
    assert_eq!(cap.traj_count("SELECT COUNT(*) FROM threads"), 1);
    assert_eq!(cap.traj_count("SELECT COUNT(*) FROM steps"), 5);
    // The one thread is the main thread: its first step called a tool.
    assert_eq!(
        cap.traj_count("SELECT COUNT(*) FROM threads WHERE kind = 'main'"),
        1
    );
    // Each step stores only what it adds: the prompt, then one result per step.
    // Copying the full history each time would store 1 + 3 + 5 + 7 + 9 = 25.
    assert_eq!(
        cap.traj_count("SELECT COUNT(*) FROM step_items WHERE side = 'input'"),
        5
    );
    assert_eq!(
        cap.traj_count("SELECT COUNT(*) FROM step_items WHERE side = 'output'"),
        5
    );
    // Each step links to the previous one with the length of the shared prefix.
    let prefixes: Vec<i64> = cap
        .traj
        .with_conn(|c| {
            let mut stmt = c.prepare("SELECT prefix_len FROM steps ORDER BY seq")?;
            let rows = stmt
                .query_map([], |r| r.get(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .unwrap();
    assert_eq!(prefixes, vec![0, 2, 4, 6, 8]);
    assert_eq!(
        cap.traj_count("SELECT COUNT(*) FROM steps WHERE parent_step_id IS NOT NULL"),
        4
    );
    cap.writer.shutdown();
}

#[tokio::test]
async fn system_and_tools_are_stored_once() {
    let cap = start(None).await;
    let big_tools = json!([{
        "name": "shell",
        "description": "x".repeat(30 * 1024),
        "input_schema": {"type": "object", "properties": {"cmd": {"type": "string"}}}
    }]);
    run_tool_loop(&cap, big_tools).await;
    cap.settle_steps(5).await;

    assert_eq!(
        cap.traj_count("SELECT COUNT(*) FROM blobs WHERE kind = 'tools'"),
        1
    );
    assert_eq!(
        cap.traj_count("SELECT COUNT(*) FROM steps WHERE tools_blob IS NOT NULL"),
        5
    );
    assert_eq!(
        cap.traj_count("SELECT COUNT(DISTINCT tools_blob) FROM steps"),
        1
    );
    cap.writer.shutdown();
}

#[tokio::test]
async fn client_disconnect_mid_stream_keeps_partial_output() {
    let cap = start(None).await;
    let frame = |event: &str, data: Value| format!("event: {event}\ndata: {data}\n\n").into_bytes();
    let delta = |text: &str| {
        frame(
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0,
                "delta": {"type": "text_delta", "text": text}}),
        )
    };
    let upstream = raw_sse_upstream(vec![
        Step::Send(frame(
            "message_start",
            json!({"type": "message_start", "message": {"id": "msg_1", "type": "message",
                "role": "assistant", "model": "claude-up", "content": [],
                "usage": {"input_tokens": 3, "output_tokens": 0}}}),
        )),
        Step::Send(frame(
            "content_block_start",
            json!({"type": "content_block_start", "index": 0,
                "content_block": {"type": "text", "text": ""}}),
        )),
        Step::Send(delta("one ")),
        Step::Send(delta("two ")),
        Step::Send(delta("three")),
        // The upstream then stalls; the client gives up long before this ends.
        Step::Sleep(Duration::from_secs(60)),
    ])
    .await;
    cap.env.install(
        provider("pa", "an", Protocol::Anthropic, &upstream.base_url),
        model("ma", "claude-up"),
    );

    let route = cap.env.route_to(Protocol::Anthropic, Protocol::Anthropic);
    let mut body = request_body(Protocol::Anthropic, CLAUDE_MODEL);
    body["stream"] = json!(true);
    body["tools"] = shell_tools();
    let mut resp = cap
        .env
        .http
        .post(cap.env.url(&route))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let mut seen = String::new();
    while !seen.contains("two ") {
        let chunk = resp.chunk().await.unwrap().expect("stream ended early");
        seen.push_str(&String::from_utf8_lossy(&chunk));
    }
    drop(resp);

    // The request must leave the in-flight table once the client is gone.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cap.env.state.live.snapshot(0).in_flight.is_empty() {
        assert!(Instant::now() < deadline, "request still in flight");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let live = cap.env.state.live.snapshot(0);
    assert!(
        matches!(live.completed[0].phase, LivePhase::Done | LivePhase::Error),
        "{:?}",
        live.completed[0].phase
    );

    cap.settle_steps(1).await;
    assert_eq!(
        cap.traj_count(
            "SELECT COUNT(*) FROM steps WHERE stop_reason = 'cancelled' AND finished_at IS NULL"
        ),
        1
    );
    let text = output_text(&cap, &step_id_of(&cap));
    assert!(
        text == "one two " || text == "one two three",
        "partial output: {text:?}"
    );
    // The usage row is written once; the cancelled request must not add another.
    let deadline = Instant::now() + Duration::from_secs(5);
    while cap.main_count("SELECT COUNT(*) FROM usage") < 1 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(cap.main_count("SELECT COUNT(*) FROM usage"), 1);
    cap.writer.shutdown();
}

#[tokio::test]
async fn oversized_or_saturated_capture_degrades_to_metadata() {
    // A 1 MiB in-flight budget makes the 8 MiB request below too large to capture.
    let cap = start(Some(1024 * 1024)).await;
    mount_json(&cap.env, Protocol::Anthropic).await;
    let route = cap.env.route_to(Protocol::Anthropic, Protocol::Anthropic);
    let mut body = request_body(Protocol::Anthropic, CLAUDE_MODEL);
    body["messages"][0]["content"] = json!("x".repeat(8 * 1024 * 1024));

    let resp = cap.env.post(&route, &body).await;
    assert_eq!(resp.status(), 200);
    let got: Value = resp.json().await.unwrap();
    assert_eq!(got, upstream_response(Protocol::Anthropic));

    cap.settle_steps(1).await;
    assert_eq!(
        cap.traj_count("SELECT COUNT(*) FROM steps WHERE capture = 'metadata_only'"),
        1
    );
    assert_eq!(cap.traj_count("SELECT COUNT(*) FROM steps"), 1);
    assert_eq!(cap.traj_count("SELECT COUNT(*) FROM sessions"), 1);
    cap.writer.shutdown();
}

#[tokio::test]
async fn usage_session_id_is_backfilled() {
    let cap = start(None).await;
    mount_json(&cap.env, Protocol::Anthropic).await;
    let route = cap.env.route_to(Protocol::Anthropic, Protocol::Anthropic);
    let mut body = request_body(Protocol::Anthropic, CLAUDE_MODEL);
    body["tools"] = shell_tools();
    let resp = cap.env.post(&route, &body).await;
    assert_eq!(resp.status(), 200);
    cap.settle_steps(1).await;

    let step: (String, String) = cap
        .traj
        .with_conn(|c| {
            Ok(c.query_row("SELECT id, session_id FROM steps", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?)
        })
        .unwrap();
    // The usage row can land a little after the step; the backfill retries until it does.
    let deadline = Instant::now() + Duration::from_secs(10);
    let usage_session: Option<String> = loop {
        cap.writer.flush_for_test();
        let found: Option<Option<String>> = cap
            .env
            .services
            .db()
            .with_conn(|c| {
                let row = c.query_row(
                    "SELECT session_id FROM usage WHERE step_id = ?1",
                    [&step.0],
                    |r| r.get(0),
                );
                Ok(row.ok())
            })
            .unwrap();
        if let Some(Some(s)) = found {
            break Some(s);
        }
        if Instant::now() >= deadline {
            break None;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(usage_session.as_deref(), Some(step.1.as_str()));
    cap.writer.shutdown();
}

#[tokio::test]
async fn bad_request_json_degrades_without_failing_proxy() {
    let cap = start(None).await;
    mount_json(&cap.env, Protocol::Anthropic).await;
    let route = cap.env.route_to(Protocol::Anthropic, Protocol::Anthropic);
    // Valid JSON, but `messages` is not a list, so the request cannot be parsed into IR.
    // (A missing `messages` is a valid empty conversation and is stored in full.)
    let body = json!({"model": CLAUDE_MODEL, "max_tokens": 16, "messages": "not-a-list"});

    let resp = cap.env.post(&route, &body).await;
    assert_eq!(resp.status(), 200);
    cap.settle_steps(1).await;
    assert_eq!(
        cap.traj_count("SELECT COUNT(*) FROM steps WHERE capture = 'metadata_only'"),
        1
    );
    cap.writer.shutdown();
}

/// Upstream for the subagent test: the main prompt calls `Task`, the subagent answers.
struct Subagent;

impl Respond for Subagent {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
        let is_main = body["messages"][0]["content"] == "fix the build";
        let reply = if is_main {
            json!({
                "id": "msg_main", "type": "message", "role": "assistant", "model": "claude-up",
                "content": [{"type": "tool_use", "id": "task-1", "name": "Task",
                    "input": {"prompt": "检查测试"}}],
                "stop_reason": "tool_use", "stop_sequence": null,
                "usage": {"input_tokens": 3, "output_tokens": 2}})
        } else {
            json!({
                "id": "msg_sub", "type": "message", "role": "assistant", "model": "claude-up",
                "content": [{"type": "text", "text": "all green"}],
                "stop_reason": "end_turn", "stop_sequence": null,
                "usage": {"input_tokens": 3, "output_tokens": 2}})
        };
        ResponseTemplate::new(200).set_body_json(reply)
    }
}

#[tokio::test]
async fn subagent_thread_links_to_its_task_call() {
    let cap = start(None).await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(Subagent)
        .mount(&cap.env.mock)
        .await;
    let route = cap.env.route_to(Protocol::Anthropic, Protocol::Anthropic);
    // Claude Code carries its session id in metadata.user_id.
    let user_id = "user_abc_account_def_session_0b6e5f3a-1c2d-4e5f-8a9b-0c1d2e3f4a5b";
    let main_req = json!({"model": CLAUDE_MODEL, "max_tokens": 64,
        "metadata": {"user_id": user_id},
        "messages": [{"role": "user", "content": "fix the build"}]});
    let sub_req = json!({"model": CLAUDE_MODEL, "max_tokens": 64,
        "metadata": {"user_id": user_id},
        "messages": [{"role": "user", "content": "检查测试"}]});
    assert_eq!(cap.env.post(&route, &main_req).await.status(), 200);
    assert_eq!(cap.env.post(&route, &sub_req).await.status(), 200);
    cap.settle_steps(2).await;

    assert_eq!(cap.traj_count("SELECT COUNT(*) FROM sessions"), 1);
    assert_eq!(cap.traj_count("SELECT COUNT(*) FROM threads"), 2);
    let (kind, spawn, parent): (String, Option<String>, Option<String>) = cap
        .traj
        .with_conn(|c| {
            Ok(c.query_row(
                "SELECT kind, spawn_call_id, parent_thread_id FROM threads WHERE kind = 'subagent'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?)
        })
        .unwrap();
    assert_eq!(kind, "subagent");
    assert_eq!(spawn.as_deref(), Some("task-1"));
    let main_thread: String = cap
        .traj
        .with_conn(|c| {
            Ok(
                c.query_row("SELECT id FROM threads WHERE kind = 'main'", [], |r| {
                    r.get(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(parent.as_deref(), Some(main_thread.as_str()));
    cap.writer.shutdown();
}

/// With the global-route switch off, a request on the global route (`api`, no
/// terminal client) writes no step, while a terminal client request is still recorded.
#[tokio::test]
async fn disabled_global_skips_global_route_only() {
    let settings = TrajectorySettings {
        disabled_global: true,
        ..TrajectorySettings::default()
    };
    let cap = start_with(None, settings).await;
    mount_json(&cap.env, Protocol::Anthropic).await;

    let global = cap
        .env
        .post(
            "/v1/messages",
            &request_body(Protocol::Anthropic, CLAUDE_MODEL),
        )
        .await;
    assert_eq!(global.status(), 200);
    cap.env.wait_idle(Duration::from_secs(10)).await;
    cap.writer.flush_for_test();
    assert_eq!(cap.traj_count("SELECT COUNT(*) FROM steps"), 0);

    let route = cap.env.route_to(Protocol::Anthropic, Protocol::Anthropic);
    let mut client_body = request_body(Protocol::Anthropic, CLAUDE_MODEL);
    client_body["tools"] = shell_tools();
    let client = cap.env.post(&route, &client_body).await;
    assert_eq!(client.status(), 200);
    cap.settle_steps(1).await;
    assert_eq!(cap.traj_count("SELECT COUNT(*) FROM steps"), 1);
    cap.writer.shutdown();
}

/// A side request (such as a title generator) has no client session key, no tools
/// and a single user message. It is not recorded, but the proxy still answers and
/// writes its usage row.
#[tokio::test]
async fn side_only_title_request_is_not_recorded() {
    let cap = start(None).await;
    mount_json(&cap.env, Protocol::Anthropic).await;
    let route = cap.env.route_to(Protocol::Anthropic, Protocol::Anthropic);

    let resp = cap
        .env
        .post(&route, &request_body(Protocol::Anthropic, CLAUDE_MODEL))
        .await;
    assert_eq!(resp.status(), 200);
    cap.env.wait_idle(Duration::from_secs(10)).await;
    cap.writer.flush_for_test();

    assert_eq!(cap.traj_count("SELECT COUNT(*) FROM sessions"), 0);
    assert_eq!(cap.traj_count("SELECT COUNT(*) FROM steps"), 0);
    let deadline = Instant::now() + Duration::from_secs(5);
    while cap.main_count("SELECT COUNT(*) FROM usage") < 1 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(cap.main_count("SELECT COUNT(*) FROM usage"), 1);
    cap.writer.shutdown();
}

/// A title request whose instructions come as a leading system message (the Chat
/// Completions shape DeepSeek Harness sends) is still a side request and is not recorded.
#[tokio::test]
async fn side_only_request_with_system_message_is_not_recorded() {
    let cap = start(None).await;
    mount_json(&cap.env, Protocol::OpenaiChat).await;
    let route = cap.env.route_to(Protocol::OpenaiChat, Protocol::OpenaiChat);
    let body = json!({"model": CHAT_MODEL, "messages": [
        {"role": "system", "content": "You write short session titles."},
        {"role": "user", "content": "Generate the session title from this JSON array of human messages: [{\"seq\":8,\"text\":\"fix the parser\"}]"}
    ]});

    assert_eq!(cap.env.post(&route, &body).await.status(), 200);
    cap.env.wait_idle(Duration::from_secs(10)).await;
    cap.writer.flush_for_test();

    assert_eq!(cap.traj_count("SELECT COUNT(*) FROM sessions"), 0);
    assert_eq!(cap.traj_count("SELECT COUNT(*) FROM steps"), 0);
    cap.writer.shutdown();
}

/// A system message followed by a real exchange is history, so the request is recorded.
#[tokio::test]
async fn system_message_with_history_is_still_recorded() {
    let cap = start(None).await;
    mount_json(&cap.env, Protocol::OpenaiChat).await;
    let route = cap.env.route_to(Protocol::OpenaiChat, Protocol::OpenaiChat);
    let body = json!({"model": CHAT_MODEL, "messages": [
        {"role": "system", "content": "You are helpful."},
        {"role": "user", "content": "hi"},
        {"role": "assistant", "content": "hello"},
        {"role": "user", "content": "fix the parser"}
    ]});

    assert_eq!(cap.env.post(&route, &body).await.status(), 200);
    cap.settle_steps(1).await;
    assert_eq!(cap.traj_count("SELECT COUNT(*) FROM steps"), 1);
    cap.writer.shutdown();
}

/// One user message is recorded when the request also defines tools.
#[tokio::test]
async fn request_with_tools_is_still_recorded() {
    let cap = start(None).await;
    mount_json(&cap.env, Protocol::Anthropic).await;
    let route = cap.env.route_to(Protocol::Anthropic, Protocol::Anthropic);
    let mut body = request_body(Protocol::Anthropic, CLAUDE_MODEL);
    body["tools"] = json!([{
        "name": "shell",
        "description": "run a command",
        "input_schema": {"type": "object", "properties": {"cmd": {"type": "string"}}}
    }]);

    assert_eq!(cap.env.post(&route, &body).await.status(), 200);
    cap.settle_steps(1).await;
    assert_eq!(cap.traj_count("SELECT COUNT(*) FROM sessions"), 1);
    assert_eq!(cap.traj_count("SELECT COUNT(*) FROM steps"), 1);
    cap.writer.shutdown();
}

/// One user message without tools is recorded when the client sent a session key.
#[tokio::test]
async fn keyed_request_without_tools_is_still_recorded() {
    let cap = start(None).await;
    mount_json(&cap.env, Protocol::Anthropic).await;
    let route = cap.env.route_to(Protocol::Anthropic, Protocol::Anthropic);

    let resp = cap
        .env
        .http
        .post(cap.env.url(&route))
        .header("session_id", "side-session-1")
        .json(&request_body(Protocol::Anthropic, CLAUDE_MODEL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    cap.settle_steps(1).await;
    assert_eq!(cap.traj_count("SELECT COUNT(*) FROM sessions"), 1);
    assert_eq!(cap.traj_count("SELECT COUNT(*) FROM steps"), 1);
    cap.writer.shutdown();
}
