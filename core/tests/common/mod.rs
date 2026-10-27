//! Shared integration-test environment: a wiremock upstream, real services, one
//! provider per protocol and the proxy listening on a random port.
#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use wiremock::MockServer;
use yi_core::domain::{
    Capabilities, Client, Model, ProfileModel, Protocol, Provider, ProxyProfile, RouteMode,
};
use yi_core::proxy::ProxyHandle;
use yi_core::proxy::router::{ProxyState, build};
use yi_core::services::Services;
use yi_core::storage::usage::{self, UsageQuery};
use yi_core::storage::{providers, terminal};

pub const CLAUDE_MODEL: &str = "name-pa/claude-up";
pub const CHAT_MODEL: &str = "name-pc/chat-up";
pub const RESP_MODEL: &str = "name-pr/resp-up";

pub struct TestEnv {
    /// `http://127.0.0.1:<port>` of the proxy.
    pub base_url: String,
    pub mock: MockServer,
    pub services: Arc<Services>,
    pub state: ProxyState,
    pub http: reqwest::Client,
    _proxy: ProxyHandle,
    _dir: tempfile::TempDir,
}

pub fn provider(id: &str, _code: &str, native: Protocol, base_url: &str) -> Provider {
    Provider {
        id: id.into(),
        provider_type: native,
        name: format!("name-{id}"),
        base_url: base_url.into(),
        api_key: format!("key-{id}"),
        enabled: true,
        thinking: None,
        is_default: false,
        sort_order: 0,
        extra: json!({}),
        protocols: vec![native],
    }
}

pub fn model(id: &str, upstream: &str) -> Model {
    Model {
        id: id.into(),
        provider_id: String::new(),
        standard_model_id: None,
        upstream_model: upstream.into(),
        exposed_name: String::new(),
        capabilities: Capabilities::default(),
        non_standard: false,
    }
}

impl TestEnv {
    pub async fn start() -> TestEnv {
        let dir = tempfile::tempdir().unwrap();
        let services = Services::for_test(dir.path());
        let mock = MockServer::start().await;
        let state = ProxyState::new(services.routes(), services.usage_sink());
        let st = state.clone();
        let proxy = ProxyHandle::new(move || build(st.clone()));
        proxy.start("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let addr = proxy.status().addr.unwrap();
        let env = TestEnv {
            base_url: format!("http://{addr}"),
            mock,
            services,
            state,
            http: reqwest::Client::builder().no_proxy().build().unwrap(),
            _proxy: proxy,
            _dir: dir,
        };
        let root = env.mock.uri();
        env.install(
            provider("pa", "an", Protocol::Anthropic, &root),
            model("ma", "claude-up"),
        );
        env.install(
            provider("pc", "oc", Protocol::OpenaiChat, &format!("{root}/v1")),
            model("mc", "chat-up"),
        );
        env.install(
            provider("pr", "rs", Protocol::Responses, &format!("{root}/v1")),
            model("mr", "resp-up"),
        );
        env
    }

    /// Saves a provider with one model and refreshes the route snapshot.
    pub fn install(&self, p: Provider, m: Model) {
        self.services
            .db()
            .with_tx(|tx| providers::upsert(tx, &p, &[m]))
            .unwrap();
        self.services.routes().refresh().unwrap();
    }

    /// A fixed proxy profile whose only model is `exposed`.
    pub fn fixed_profile(&self, client: Client, protocol: Protocol, pid: &str, exposed: &str) {
        let profile = ProxyProfile {
            client,
            protocol,
            route_mode: RouteMode::Fixed,
            models: vec![ProfileModel {
                provider_id: pid.into(),
                model: exposed.into(),
            }],
            default_model: Some(exposed.into()),
        };
        self.services
            .db()
            .with_conn(|c| terminal::save_proxy(c, &profile))
            .unwrap();
        self.services.routes().refresh().unwrap();
    }

    /// Points the client that speaks `inbound` at `exposed` of provider `pid`
    /// and returns the request path. Global routes only match providers that
    /// speak the inbound protocol, so translation is reached through a client
    /// route.
    pub fn route(&self, inbound: Protocol, pid: &str, exposed: &str) -> String {
        let client = match inbound {
            Protocol::Anthropic => Client::ClaudeCode,
            Protocol::OpenaiChat => Client::Opencode,
            Protocol::Responses => Client::Codex,
        };
        self.fixed_profile(client, inbound, pid, exposed);
        format!("/clients/{}{}", client.as_str(), inbound_path(inbound))
    }

    /// `route` for the default provider of the `upstream` protocol.
    pub fn route_to(&self, inbound: Protocol, upstream: Protocol) -> String {
        self.route(inbound, provider_id(upstream), exposed(upstream))
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    pub async fn post(&self, path: &str, body: &Value) -> reqwest::Response {
        self.http
            .post(self.url(path))
            .json(body)
            .send()
            .await
            .unwrap()
    }

    /// Bodies the mock upstream received, parsed as JSON, with their paths.
    pub async fn upstream_requests(&self) -> Vec<(String, Value, wiremock::http::HeaderMap)> {
        self.mock
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .map(|r| {
                (
                    r.url.path().to_owned(),
                    serde_json::from_slice(&r.body).unwrap_or(Value::Null),
                    r.headers,
                )
            })
            .collect()
    }

    /// Number of persisted usage records, waiting up to `wait` for at least `min`.
    pub async fn usage_count(&self, min: u64, wait: Duration) -> u64 {
        let deadline = std::time::Instant::now() + wait;
        loop {
            let n = self.usage_total();
            if n >= min || std::time::Instant::now() >= deadline {
                return n;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub fn usage_total(&self) -> u64 {
        self.usage_report().total_requests
    }

    pub fn usage_report(&self) -> yi_core::domain::UsageReport {
        self.services
            .db()
            .with_conn(|c| {
                usage::query(
                    c,
                    UsageQuery {
                        start_ms: 0,
                        end_ms: i64::MAX,
                        model: None,
                    },
                )
            })
            .unwrap()
    }

    /// Waits until the proxy has no in-flight request.
    pub async fn wait_idle(&self, wait: Duration) -> usize {
        let deadline = std::time::Instant::now() + wait;
        loop {
            let n = self.state.in_flight();
            if n == 0 || std::time::Instant::now() >= deadline {
                return n;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

// ----------------------------------------------------------------- bodies

pub fn request_body(inbound: Protocol, model: &str) -> Value {
    match inbound {
        Protocol::Anthropic => {
            json!({"model": model, "max_tokens": 16,
                "messages": [{"role": "user", "content": "hi"}]})
        }
        Protocol::OpenaiChat => {
            json!({"model": model, "messages": [{"role": "user", "content": "hi"}]})
        }
        Protocol::Responses => json!({"model": model, "input": "hi"}),
    }
}

pub fn inbound_path(p: Protocol) -> &'static str {
    match p {
        Protocol::Anthropic => "/v1/messages",
        Protocol::OpenaiChat => "/v1/chat/completions",
        Protocol::Responses => "/v1/responses",
    }
}

pub fn upstream_path(p: Protocol) -> &'static str {
    inbound_path(p)
}

pub fn provider_id(p: Protocol) -> &'static str {
    match p {
        Protocol::Anthropic => "pa",
        Protocol::OpenaiChat => "pc",
        Protocol::Responses => "pr",
    }
}

pub fn exposed(p: Protocol) -> &'static str {
    match p {
        Protocol::Anthropic => CLAUDE_MODEL,
        Protocol::OpenaiChat => CHAT_MODEL,
        Protocol::Responses => RESP_MODEL,
    }
}

/// A non-streaming upstream response that says "hello" with usage 3 / 2.
pub fn upstream_response(p: Protocol) -> Value {
    match p {
        Protocol::Anthropic => json!({
            "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-up",
            "content": [{"type": "text", "text": "hello"}],
            "stop_reason": "end_turn", "stop_sequence": null,
            "usage": {"input_tokens": 3, "output_tokens": 2}}),
        Protocol::OpenaiChat => json!({
            "id": "c1", "object": "chat.completion", "created": 1, "model": "chat-up",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "hello"},
                "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5}}),
        Protocol::Responses => json!({
            "id": "r1", "object": "response", "created_at": 1, "status": "completed",
            "model": "resp-up",
            "output": [{"type": "message", "id": "m1", "role": "assistant",
                "status": "completed",
                "content": [{"type": "output_text", "text": "hello", "annotations": []}]}],
            "usage": {"input_tokens": 3, "output_tokens": 2, "total_tokens": 5}}),
    }
}

/// The text of a non-streaming client response.
pub fn response_text(p: Protocol, v: &Value) -> String {
    let s = match p {
        Protocol::Anthropic => v.pointer("/content/0/text"),
        Protocol::OpenaiChat => v.pointer("/choices/0/message/content"),
        Protocol::Responses => v.pointer("/output/0/content/0/text"),
    };
    s.and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn sse(event: Option<&str>, data: &Value) -> String {
    match event {
        Some(e) => format!("event: {e}\ndata: {data}\n\n"),
        None => format!("data: {data}\n\n"),
    }
}

/// Upstream SSE frames (each one complete frame) saying `text` in one delta.
pub fn upstream_stream_frames(p: Protocol, text: &str) -> Vec<String> {
    match p {
        Protocol::Anthropic => vec![
            sse(
                Some("message_start"),
                &json!({"type": "message_start", "message": {"id": "msg_1", "type": "message",
                    "role": "assistant", "model": "claude-up", "content": [],
                    "usage": {"input_tokens": 3, "output_tokens": 0}}}),
            ),
            sse(
                Some("content_block_start"),
                &json!({"type": "content_block_start", "index": 0,
                    "content_block": {"type": "text", "text": ""}}),
            ),
            sse(
                Some("content_block_delta"),
                &json!({"type": "content_block_delta", "index": 0,
                    "delta": {"type": "text_delta", "text": text}}),
            ),
            sse(
                Some("content_block_stop"),
                &json!({"type": "content_block_stop", "index": 0}),
            ),
            sse(
                Some("message_delta"),
                &json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"},
                    "usage": {"output_tokens": 2}}),
            ),
            sse(Some("message_stop"), &json!({"type": "message_stop"})),
        ],
        Protocol::OpenaiChat => vec![
            sse(
                None,
                &json!({"id": "c1", "object": "chat.completion.chunk", "created": 1,
                    "model": "chat-up",
                    "choices": [{"index": 0, "delta": {"role": "assistant", "content": ""},
                        "finish_reason": null}]}),
            ),
            sse(
                None,
                &json!({"id": "c1", "object": "chat.completion.chunk", "created": 1,
                    "model": "chat-up",
                    "choices": [{"index": 0, "delta": {"content": text},
                        "finish_reason": null}]}),
            ),
            sse(
                None,
                &json!({"id": "c1", "object": "chat.completion.chunk", "created": 1,
                    "model": "chat-up",
                    "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
            ),
            sse(
                None,
                &json!({"id": "c1", "object": "chat.completion.chunk", "created": 1,
                    "model": "chat-up", "choices": [],
                    "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5}}),
            ),
            "data: [DONE]\n\n".to_owned(),
        ],
        Protocol::Responses => {
            let msg = json!({"type": "message", "id": "m1", "role": "assistant",
                "status": "completed",
                "content": [{"type": "output_text", "text": text, "annotations": []}]});
            vec![
                sse(
                    Some("response.created"),
                    &json!({"type": "response.created", "response": {"id": "r1",
                        "object": "response", "created_at": 1, "status": "in_progress",
                        "model": "resp-up", "output": []}}),
                ),
                sse(
                    Some("response.output_item.added"),
                    &json!({"type": "response.output_item.added", "output_index": 0,
                        "item": {"type": "message", "id": "m1", "role": "assistant",
                            "status": "in_progress", "content": []}}),
                ),
                sse(
                    Some("response.content_part.added"),
                    &json!({"type": "response.content_part.added", "output_index": 0,
                        "content_index": 0, "item_id": "m1",
                        "part": {"type": "output_text", "text": "", "annotations": []}}),
                ),
                sse(
                    Some("response.output_text.delta"),
                    &json!({"type": "response.output_text.delta", "output_index": 0,
                        "content_index": 0, "item_id": "m1", "delta": text}),
                ),
                sse(
                    Some("response.output_text.done"),
                    &json!({"type": "response.output_text.done", "output_index": 0,
                        "content_index": 0, "item_id": "m1", "text": text}),
                ),
                sse(
                    Some("response.content_part.done"),
                    &json!({"type": "response.content_part.done", "output_index": 0,
                        "content_index": 0, "item_id": "m1",
                        "part": {"type": "output_text", "text": text, "annotations": []}}),
                ),
                sse(
                    Some("response.output_item.done"),
                    &json!({"type": "response.output_item.done", "output_index": 0,
                        "item": msg}),
                ),
                sse(
                    Some("response.completed"),
                    &json!({"type": "response.completed", "response": {"id": "r1",
                        "object": "response", "created_at": 1, "status": "completed",
                        "model": "resp-up", "output": [msg],
                        "usage": {"input_tokens": 3, "output_tokens": 2, "total_tokens": 5}}}),
                ),
            ]
        }
    }
}

/// Concatenated assistant text of a client-side SSE body.
pub fn stream_text(p: Protocol, body: &str) -> String {
    let mut out = String::new();
    for frame in body.split("\n\n") {
        let Some(data) = frame
            .lines()
            .find_map(|l| l.strip_prefix("data: ").or_else(|| l.strip_prefix("data:")))
        else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(data.trim()) else {
            continue;
        };
        let piece = match p {
            Protocol::Anthropic => v.pointer("/delta/text"),
            Protocol::OpenaiChat => v.pointer("/choices/0/delta/content"),
            Protocol::Responses if v["type"] == "response.output_text.delta" => v.get("delta"),
            Protocol::Responses => None,
        };
        if let Some(s) = piece.and_then(Value::as_str) {
            out.push_str(s);
        }
    }
    out
}

// ------------------------------------------------------- scripted raw server

pub enum Step {
    Send(Vec<u8>),
    Sleep(Duration),
}

pub struct RawUpstream {
    pub base_url: String,
    /// Set when the proxy closed the connection while a script step was pending.
    pub cancelled: Arc<AtomicBool>,
}

/// An HTTP/1.1 server answering every request with a chunked `text/event-stream`
/// response that follows `script`. wiremock cannot emit separate chunks, which
/// the split-character and disconnect tests need.
pub async fn raw_sse_upstream(script: Vec<Step>) -> RawUpstream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancelled);
    let script = Arc::new(script);
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let flag = Arc::clone(&flag);
            let script = Arc::clone(&script);
            tokio::spawn(async move {
                if read_request(&mut sock).await.is_none() {
                    return;
                }
                let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                            transfer-encoding: chunked\r\n\r\n";
                if sock.write_all(head.as_bytes()).await.is_err() {
                    return;
                }
                for step in script.iter() {
                    match step {
                        Step::Send(bytes) => {
                            let mut chunk = format!("{:x}\r\n", bytes.len()).into_bytes();
                            chunk.extend_from_slice(bytes);
                            chunk.extend_from_slice(b"\r\n");
                            if sock.write_all(&chunk).await.is_err() || sock.flush().await.is_err()
                            {
                                flag.store(true, Ordering::SeqCst);
                                return;
                            }
                        }
                        Step::Sleep(d) => {
                            let mut probe = [0u8; 1];
                            tokio::select! {
                                () = tokio::time::sleep(*d) => {}
                                r = sock.read(&mut probe) => {
                                    // EOF or error: the peer went away.
                                    if matches!(r, Ok(0) | Err(_)) {
                                        flag.store(true, Ordering::SeqCst);
                                        return;
                                    }
                                }
                            }
                        }
                    }
                }
                let _ = sock.write_all(b"0\r\n\r\n").await;
            });
        }
    });
    RawUpstream {
        base_url: format!("http://{addr}"),
        cancelled,
    }
}

async fn read_request(sock: &mut tokio::net::TcpStream) -> Option<()> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let head_end = loop {
        let n = sock.read(&mut tmp).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_lowercase();
    let len: usize = head
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    while buf.len() < head_end + len {
        let n = sock.read(&mut tmp).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    Some(())
}
