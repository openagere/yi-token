mod common;

use std::sync::atomic::Ordering;
use std::time::Duration;

use common::*;
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};
use yi_core::domain::capabilities::{Effort, Support};
use yi_core::domain::{Capabilities, Client, Protocol, Thinking};
use yi_core::services::{ProtocolSupport, ProviderInput};
use yi_core::CoreError;

async fn mount_json(env: &TestEnv, p: Protocol) {
    Mock::given(method("POST"))
        .and(path(upstream_path(p)))
        .respond_with(ResponseTemplate::new(200).set_body_json(upstream_response(p)))
        .mount(&env.mock)
        .await;
}

async fn mount_sse(env: &TestEnv, p: Protocol, text: &str) {
    let body: String = upstream_stream_frames(p, text).concat();
    Mock::given(method("POST"))
        .and(path(upstream_path(p)))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&env.mock)
        .await;
}

// ------------------------------------------------------------ passthrough

#[tokio::test]
async fn passthrough_anthropic_to_anthropic_preserves_unknown_fields() {
    let env = TestEnv::start().await;
    mount_json(&env, Protocol::Anthropic).await;
    let mut body = request_body(Protocol::Anthropic, CLAUDE_MODEL);
    body["x_vendor"] = json!({"deep": [1, 2.5, {"k": null}]});
    body["metadata"] = json!({"user_id": "u"});
    body["tools"] = json!([{"type": "server_tool_x", "name": "n", "weird": true}]);
    let resp = env.post("/v1/messages", &body).await;
    assert_eq!(resp.status(), 200);
    let got: Value = resp.json().await.unwrap();
    assert_eq!(got, upstream_response(Protocol::Anthropic));
    let reqs = env.upstream_requests().await;
    assert_eq!(reqs.len(), 1);
    let mut expected = body.clone();
    expected["model"] = json!("claude-up");
    assert_eq!(reqs[0].1, expected);
}

// ------------------------------------------------------------- 3 x 3 matrix

macro_rules! translate_case {
    ($name:ident, $inbound:expr, $upstream:expr) => {
        #[tokio::test]
        async fn $name() {
            let (inbound, upstream): (Protocol, Protocol) = ($inbound, $upstream);
            let env = TestEnv::start().await;
            mount_json(&env, upstream).await;
            let body = request_body(inbound, exposed(upstream));
            let route = env.route_to(inbound, upstream);
            let resp = env.post(&route, &body).await;
            assert_eq!(resp.status(), 200);
            let got: Value = resp.json().await.unwrap();
            assert_eq!(response_text(inbound, &got), "hello", "downstream shape");
            let reqs = env.upstream_requests().await;
            assert_eq!(reqs.len(), 1);
            let (p, sent, _) = &reqs[0];
            assert_eq!(p, upstream_path(upstream));
            assert!(sent.is_object());
            match upstream {
                Protocol::Anthropic => {
                    assert_eq!(sent["model"], "claude-up");
                    assert!(sent["messages"].is_array() && sent.get("input").is_none());
                    assert!(sent["max_tokens"].is_u64());
                }
                Protocol::OpenaiChat => {
                    assert_eq!(sent["model"], "chat-up");
                    assert!(sent["messages"].is_array() && sent.get("input").is_none());
                }
                Protocol::Responses => {
                    assert_eq!(sent["model"], "resp-up");
                    assert!(sent.get("input").is_some() && sent.get("messages").is_none());
                }
            }
        }
    };
}

translate_case!(
    translate_anthropic_to_anthropic_non_stream,
    Protocol::Anthropic,
    Protocol::Anthropic
);
translate_case!(
    translate_anthropic_to_chat_non_stream,
    Protocol::Anthropic,
    Protocol::OpenaiChat
);
translate_case!(
    translate_anthropic_to_responses_non_stream,
    Protocol::Anthropic,
    Protocol::Responses
);
translate_case!(
    translate_chat_to_anthropic_non_stream,
    Protocol::OpenaiChat,
    Protocol::Anthropic
);
translate_case!(
    translate_chat_to_chat_non_stream,
    Protocol::OpenaiChat,
    Protocol::OpenaiChat
);
translate_case!(
    translate_chat_to_responses_non_stream,
    Protocol::OpenaiChat,
    Protocol::Responses
);
translate_case!(
    translate_responses_to_anthropic_non_stream,
    Protocol::Responses,
    Protocol::Anthropic
);
translate_case!(
    translate_responses_to_chat_non_stream,
    Protocol::Responses,
    Protocol::OpenaiChat
);
#[tokio::test]
async fn claude_auto_mode_safeguards_pass_through_an_anthropic_provider() {
    let env = TestEnv::start().await;
    let mut template = upstream_response(Protocol::Anthropic);
    template["safeguard_results"] = json!([{"id": "sg1", "action": "allow"}]);
    Mock::given(method("POST"))
        .and(path(upstream_path(Protocol::Anthropic)))
        .respond_with(ResponseTemplate::new(200).set_body_json(template.clone()))
        .mount(&env.mock)
        .await;
    let mut upstream = provider("pax", "an", Protocol::Anthropic, &env.mock.uri());
    upstream.protocols = Protocol::all().to_vec();
    env.install(upstream, model("max", "claude-up"));
    let exposed = yi_core::domain::exposed_name("name-pax", "claude-up");
    let route = env.route(Protocol::Anthropic, "pax", &exposed);
    let mut body = request_body(Protocol::Anthropic, &exposed);
    body["safeguards"] = json!({"mode": "auto"});
    let resp = env
        .http
        .post(env.url(&route))
        .header("anthropic-beta", "safeguards-2026-01-01")
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let got: Value = resp.json().await.unwrap();
    assert_eq!(got["safeguard_results"], template["safeguard_results"]);
    let reqs = env.upstream_requests().await;
    assert_eq!(reqs.len(), 1);
    let (path, sent, headers) = &reqs[0];
    assert_eq!(path, upstream_path(Protocol::Anthropic));
    assert_eq!(sent["safeguards"], json!({"mode": "auto"}));
    assert_eq!(
        headers.get("anthropic-beta").unwrap().to_str().unwrap(),
        "safeguards-2026-01-01"
    );
    assert_eq!(headers.get("x-api-key").unwrap().to_str().unwrap(), "key-pax");
}

#[tokio::test]
async fn claude_translates_to_responses_when_the_provider_accepts_every_client_protocol() {
    let env = TestEnv::start().await;
    mount_json(&env, Protocol::Responses).await;
    let mut upstream = provider(
        "prx",
        "rs",
        Protocol::Responses,
        &format!("{}/v1", env.mock.uri()),
    );
    upstream.protocols = Protocol::all().to_vec();
    upstream.api_key = "ark-key".into();
    env.install(upstream, model("mrx", "resp-up"));
    let exposed = yi_core::domain::exposed_name("name-prx", "resp-up");
    let route = env.route(Protocol::Anthropic, "prx", &exposed);
    let resp = env
        .post(&route, &request_body(Protocol::Anthropic, &exposed))
        .await;
    assert_eq!(resp.status(), 200);
    let reqs = env.upstream_requests().await;
    assert_eq!(reqs.len(), 1);
    let (path, sent, headers) = &reqs[0];
    assert_eq!(path, upstream_path(Protocol::Responses));
    assert!(
        sent.get("input").is_some(),
        "translated Responses body: {sent}"
    );
    assert_eq!(
        headers.get("authorization").unwrap().to_str().unwrap(),
        "Bearer ark-key"
    );
    assert!(headers.get("x-api-key").is_none());
}

translate_case!(
    translate_responses_to_responses_non_stream,
    Protocol::Responses,
    Protocol::Responses
);

// ---------------------------------------------------------------- streaming

async fn stream_case(inbound: Protocol, upstream: Protocol) -> (TestEnv, String) {
    let env = TestEnv::start().await;
    mount_sse(&env, upstream, "hello").await;
    let mut body = request_body(inbound, exposed(upstream));
    body["stream"] = json!(true);
    let route = env.route_to(inbound, upstream);
    let resp = env.post(&route, &body).await;
    assert_eq!(resp.status(), 200);
    assert!(resp.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/event-stream"));
    let text = resp.text().await.unwrap();
    (env, text)
}

#[tokio::test]
async fn stream_responses_from_anthropic() {
    let (_env, text) = stream_case(Protocol::Responses, Protocol::Anthropic).await;
    assert_eq!(stream_text(Protocol::Responses, &text), "hello");
    assert!(text.contains("response.completed"));
}

#[tokio::test]
async fn stream_anthropic_from_chat() {
    let (_env, text) = stream_case(Protocol::Anthropic, Protocol::OpenaiChat).await;
    assert_eq!(stream_text(Protocol::Anthropic, &text), "hello");
    assert!(text.contains("message_stop"));
}

#[tokio::test]
async fn stream_chat_from_responses() {
    let (_env, text) = stream_case(Protocol::OpenaiChat, Protocol::Responses).await;
    assert_eq!(stream_text(Protocol::OpenaiChat, &text), "hello");
    assert!(text.contains("[DONE]"));
}

#[tokio::test]
async fn stream_chinese_split_across_chunks() {
    let env = TestEnv::start().await;
    let frames: String = upstream_stream_frames(Protocol::OpenaiChat, "你好世界🙂").concat();
    let bytes = frames.into_bytes();
    // Cut inside the 3-byte character 好 and again inside the 4-byte emoji.
    let good = bytes.windows(3).position(|w| w == "好".as_bytes()).unwrap();
    let emoji = bytes.windows(4).position(|w| w == "🙂".as_bytes()).unwrap();
    let cuts = [good + 1, emoji + 2];
    let mut steps = Vec::new();
    let mut last = 0;
    for cut in cuts {
        steps.push(Step::Send(bytes[last..cut].to_vec()));
        steps.push(Step::Sleep(Duration::from_millis(30)));
        last = cut;
    }
    steps.push(Step::Send(bytes[last..].to_vec()));
    let raw = raw_sse_upstream(steps).await;
    let mut p = provider(
        "pz",
        "zz",
        Protocol::OpenaiChat,
        &format!("{}/v1", raw.base_url),
    );
    p.api_key = "k".into();
    env.install(p, model("mz", "chat-up"));

    for inbound in [Protocol::Anthropic, Protocol::OpenaiChat] {
        let mut body = request_body(inbound, "name-pz/chat-up");
        body["stream"] = json!(true);
        let route = env.route(inbound, "pz", "name-pz/chat-up");
        let resp = env.post(&route, &body).await;
        assert_eq!(resp.status(), 200);
        let raw_bytes = resp.bytes().await.unwrap();
        let text = String::from_utf8(raw_bytes.to_vec()).expect("valid utf-8");
        assert!(
            !text.contains('\u{FFFD}'),
            "replacement char in {inbound:?}"
        );
        assert_eq!(stream_text(inbound, &text), "你好世界🙂");
    }
}

#[tokio::test]
async fn encrypted_content_passthrough() {
    let env = TestEnv::start().await;
    let item = json!({"type": "reasoning", "id": "rs_1", "summary": [],
        "encrypted_content": "gAAAA-opaque+/=="});
    let resp_obj = json!({"id": "r1", "object": "response", "created_at": 1,
        "status": "completed", "model": "resp-up", "output": [item.clone()],
        "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}});
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&resp_obj))
        .mount(&env.mock)
        .await;
    // Non-streaming Responses -> Responses keeps the field.
    let resp = env
        .post(
            "/v1/responses",
            &request_body(Protocol::Responses, RESP_MODEL),
        )
        .await;
    let got: Value = resp.json().await.unwrap();
    assert_eq!(got["output"][0]["encrypted_content"], "gAAAA-opaque+/==");

    // Streaming passthrough keeps it byte for byte.
    env.mock.reset().await;
    let sse = format!(
        "event: response.output_item.done\ndata: {}\n\nevent: response.completed\ndata: {}\n\n",
        json!({"type": "response.output_item.done", "output_index": 0, "item": item}),
        json!({"type": "response.completed", "response": resp_obj}),
    );
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .mount(&env.mock)
        .await;
    let mut body = request_body(Protocol::Responses, RESP_MODEL);
    body["stream"] = json!(true);
    let text = env.post("/v1/responses", &body).await.text().await.unwrap();
    assert!(text.contains("gAAAA-opaque+/=="));
}

#[tokio::test]
async fn upstream_429_returned_verbatim() {
    let env = TestEnv::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(429).set_body_raw(
            r#"{"error":{"message":"slow down","type":"rate_limit"}}"#,
            "application/json",
        ))
        .mount(&env.mock)
        .await;
    for stream in [false, true] {
        let mut body = request_body(Protocol::Anthropic, CHAT_MODEL);
        body["stream"] = json!(stream);
        let route = env.route_to(Protocol::Anthropic, Protocol::OpenaiChat);
        let resp = env.post(&route, &body).await;
        assert_eq!(resp.status(), 429);
        assert_eq!(resp.headers()["content-type"], "application/json");
        assert_eq!(
            resp.text().await.unwrap(),
            r#"{"error":{"message":"slow down","type":"rate_limit"}}"#
        );
    }
}

#[tokio::test]
async fn auth_headers_replaced() {
    let env = TestEnv::start().await;
    mount_json(&env, Protocol::OpenaiChat).await;
    mount_json(&env, Protocol::Anthropic).await;
    let body = request_body(Protocol::OpenaiChat, CHAT_MODEL);
    let resp = env
        .http
        .post(env.url("/v1/chat/completions"))
        .header("authorization", "Bearer user")
        .header("x-api-key", "user-key")
        .header("cookie", "session=1")
        .header("openai-beta", "assistants=v2")
        .header("anthropic-beta", "tools-2024")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let reqs = env.upstream_requests().await;
    let h = &reqs[0].2;
    assert_eq!(
        h.get("authorization").unwrap().to_str().unwrap(),
        "Bearer key-pc"
    );
    assert!(h.get("x-api-key").is_none());
    assert!(h.get("cookie").is_none());
    assert_eq!(
        h.get("openai-beta").unwrap().to_str().unwrap(),
        "assistants=v2"
    );

    // Anthropic provider: x-api-key plus the pinned version, no bearer.
    let resp = env
        .http
        .post(env.url("/v1/messages"))
        .header("authorization", "Bearer user")
        .header("x-api-key", "user-key")
        .header("anthropic-beta", "tools-2024")
        .json(&request_body(Protocol::Anthropic, CLAUDE_MODEL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let reqs = env.upstream_requests().await;
    let h = &reqs[1].2;
    assert_eq!(h.get("x-api-key").unwrap().to_str().unwrap(), "key-pa");
    assert_eq!(
        h.get("anthropic-version").unwrap().to_str().unwrap(),
        "2023-06-01"
    );
    assert!(h.get("authorization").is_none());
    assert_eq!(
        h.get("anthropic-beta").unwrap().to_str().unwrap(),
        "tools-2024"
    );
}

// --------------------------------------------------------------- usage

#[tokio::test]
async fn usage_recorded_once_for_stream() {
    let env = TestEnv::start().await;
    mount_sse(&env, Protocol::OpenaiChat, "hello").await;
    let mut body = request_body(Protocol::Anthropic, CHAT_MODEL);
    body["stream"] = json!(true);
    let route = env.route_to(Protocol::Anthropic, Protocol::OpenaiChat);
    let _ = env.post(&route, &body).await.text().await.unwrap();
    assert_eq!(env.usage_count(1, Duration::from_secs(5)).await, 1);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let report = env.usage_report();
    assert_eq!(report.total_requests, 1);
    assert_eq!(report.input_tokens, 3);
    assert_eq!(report.output_tokens, 2);
    assert_eq!(env.state.in_flight(), 0);
}

#[tokio::test]
async fn usage_recorded_for_non_stream_translation() {
    let env = TestEnv::start().await;
    mount_json(&env, Protocol::Responses).await;
    let route = env.route_to(Protocol::OpenaiChat, Protocol::Responses);
    let resp = env
        .post(&route, &request_body(Protocol::OpenaiChat, RESP_MODEL))
        .await;
    assert_eq!(resp.status(), 200);
    assert_eq!(env.usage_count(1, Duration::from_secs(5)).await, 1);
    let report = env.usage_report();
    assert_eq!((report.input_tokens, report.output_tokens), (3, 2));
}

#[tokio::test]
async fn client_disconnect_cancels_upstream() {
    let env = TestEnv::start().await;
    let first = upstream_stream_frames(Protocol::Anthropic, "hello");
    let raw = raw_sse_upstream(vec![
        Step::Send(first[0].clone().into_bytes()),
        Step::Send(first[1].clone().into_bytes()),
        Step::Send(first[2].clone().into_bytes()),
        Step::Sleep(Duration::from_secs(1)),
        Step::Send(first[3].clone().into_bytes()),
    ])
    .await;
    env.install(
        provider("pz", "zz", Protocol::Anthropic, &raw.base_url),
        model("mz", "claude-up"),
    );
    let mut body = request_body(Protocol::Anthropic, "name-pz/claude-up");
    body["stream"] = json!(true);
    let route = env.route(Protocol::Anthropic, "pz", "name-pz/claude-up");
    let mut resp = env.post(&route, &body).await;
    assert_eq!(resp.status(), 200);
    let first_chunk = resp.chunk().await.unwrap().expect("a first frame");
    assert!(!first_chunk.is_empty());
    assert_eq!(env.state.in_flight(), 1);
    drop(resp);

    // The upstream sees its connection close well before the 1 s sleep ends.
    let deadline = std::time::Instant::now() + Duration::from_millis(800);
    while !raw.cancelled.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        raw.cancelled.load(Ordering::SeqCst),
        "upstream was not cancelled"
    );
    assert_eq!(env.wait_idle(Duration::from_secs(2)).await, 0);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(env.usage_total() <= 1, "usage recorded more than once");
}

// ---------------------------------------------------------------- rulings

fn reasoning_caps() -> Capabilities {
    Capabilities {
        max_output_tokens: Some(100_000),
        effort: Effort {
            support: Support::Supported,
            levels: vec!["low".into(), "medium".into(), "high".into()],
            default: None,
        },
        ..Capabilities::default()
    }
}

/// Anthropic upstream with reasoning support and provider thinking = High.
fn install_thinking_provider(env: &TestEnv) {
    let mut p = provider("pt", "tt", Protocol::Anthropic, &env.mock.uri());
    p.thinking = Some(Thinking::High);
    let mut m = model("mt", "claude-up");
    m.capabilities = reasoning_caps();
    env.install(p, m);
}

#[tokio::test]
async fn client_reasoning_effort_beats_thinking_fallback() {
    let env = TestEnv::start().await;
    install_thinking_provider(&env);
    mount_json(&env, Protocol::Anthropic).await;

    // No reasoning from the client: the provider fallback (high = 32000) applies.
    let mut body = request_body(Protocol::OpenaiChat, "name-pt/claude-up");
    body["max_tokens"] = json!(50_000);
    let route = env.route(Protocol::OpenaiChat, "pt", "name-pt/claude-up");
    assert_eq!(env.post(&route, &body).await.status(), 200);
    // The client's own effort (low = 4096) is kept.
    body["reasoning_effort"] = json!("low");
    assert_eq!(env.post(&route, &body).await.status(), 200);

    let reqs = env.upstream_requests().await;
    assert_eq!(reqs[0].1["thinking"]["budget_tokens"], 32000, "fallback");
    // Kept as the client's own effort; the fallback budget is not added.
    assert_eq!(reqs[1].1["output_config"]["effort"], "low", "client effort");
    assert!(
        reqs[1].1.get("thinking").is_none(),
        "fallback applied: {}",
        reqs[1].1
    );
}

#[tokio::test]
async fn passthrough_applies_thinking_fallback_only_without_client_reasoning() {
    let env = TestEnv::start().await;
    let mut p = provider(
        "pt",
        "tt",
        Protocol::Responses,
        &format!("{}/v1", env.mock.uri()),
    );
    p.thinking = Some(Thinking::High);
    let mut m = model("mt", "resp-up");
    m.capabilities = reasoning_caps();
    env.install(p, m);
    mount_json(&env, Protocol::Responses).await;

    let mut body = request_body(Protocol::Responses, "name-pt/resp-up");
    env.post("/v1/responses", &body).await;
    body["reasoning"] = json!({"effort": "low"});
    env.post("/v1/responses", &body).await;
    let reqs = env.upstream_requests().await;
    assert_eq!(reqs[0].1["reasoning"]["effort"], "high");
    assert_eq!(reqs[1].1["reasoning"]["effort"], "low");
}

#[tokio::test]
async fn chat_include_usage_follows_raw_client_stream_options() {
    let env = TestEnv::start().await;
    mount_sse(&env, Protocol::Responses, "hello").await;
    let run = |include: Option<bool>| {
        let env = &env;
        async move {
            let mut body = request_body(Protocol::OpenaiChat, RESP_MODEL);
            body["stream"] = json!(true);
            if let Some(i) = include {
                body["stream_options"] = json!({"include_usage": i});
            }
            let route = env.route_to(Protocol::OpenaiChat, Protocol::Responses);
            env.post(&route, &body).await.text().await.unwrap()
        }
    };
    let with = run(Some(true)).await;
    assert!(
        with.contains("\"usage\":{"),
        "usage chunk missing when requested: {with}"
    );
    let without = run(None).await;
    assert!(
        !without.contains("\"usage\""),
        "unrequested usage: {without}"
    );
    let off = run(Some(false)).await;
    assert!(!off.contains("\"usage\""), "include_usage=false: {off}");
}

#[tokio::test]
async fn previous_response_id_passes_same_protocol_and_is_rejected_when_translating() {
    let env = TestEnv::start().await;
    mount_json(&env, Protocol::Responses).await;
    mount_json(&env, Protocol::OpenaiChat).await;

    let mut body = request_body(Protocol::Responses, RESP_MODEL);
    body["previous_response_id"] = json!("resp_prev");
    body["store"] = json!(true);
    // Also a json_schema request, which is a translation-only rejection.
    body["text"] = json!({"format": {"type": "json_schema", "name": "x", "schema": {}}});
    let route = env.route_to(Protocol::Responses, Protocol::Responses);
    let resp = env.post(&route, &body).await;
    assert_eq!(resp.status(), 200);
    let reqs = env.upstream_requests().await;
    assert_eq!(reqs[0].1["previous_response_id"], "resp_prev");
    assert_eq!(reqs[0].1["text"]["format"]["type"], "json_schema");

    body["model"] = json!(CHAT_MODEL);
    let route = env.route_to(Protocol::Responses, Protocol::OpenaiChat);
    let resp = env.post(&route, &body).await;
    assert_eq!(resp.status(), 400);
    assert_eq!(env.upstream_requests().await.len(), 1, "nothing was sent");
}

#[tokio::test]
async fn large_body_reaches_upstream_through_client_route() {
    let env = TestEnv::start().await;
    mount_json(&env, Protocol::Responses).await;
    env.fixed_profile(Client::Codex, Protocol::Responses, "pr", RESP_MODEL);
    let big = "x".repeat(8 * 1024 * 1024);
    let body = json!({"model": RESP_MODEL, "input": big});
    let resp = env.post("/clients/codex/v1/responses", &body).await;
    assert_ne!(resp.status(), 413);
    assert_eq!(resp.status(), 200);
    let received = env.mock.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    assert!(received[0].body.len() > 8 * 1024 * 1024);
}

#[tokio::test]
async fn unresolvable_model_is_not_a_silent_success() {
    let env = TestEnv::start().await;
    let resp = env
        .post("/v1/responses", &request_body(Protocol::Responses, "nope"))
        .await;
    assert_eq!(resp.status(), 404);
    let resp = env
        .http
        .post(env.url("/v1/responses"))
        .body("not json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

// ------------------------------------------------------------ test_provider

fn input_for(base_url: &str, native: Protocol) -> ProviderInput {
    ProviderInput {
        id: String::new(),
        provider_type: native,
        name: "probe".into(),
        base_url: base_url.into(),
        api_key: "probe-key".into(),
        enabled: true,
        thinking: Thinking::Medium,
        extra: json!({}),
        is_default: false,
        protocol_support: ProtocolSupport::default(),
    }
}

#[tokio::test]
async fn test_provider_reports_success_and_failure() {
    let env = TestEnv::start().await;
    mount_json(&env, Protocol::OpenaiChat).await;
    let ok = env
        .services
        .test_provider(
            input_for(&format!("{}/v1", env.mock.uri()), Protocol::OpenaiChat),
            "chat-up".into(),
        )
        .await
        .unwrap();
    assert!(ok.ok, "{}", ok.message);
    let probe = &env.upstream_requests().await[0];
    assert_eq!(probe.0, "/v1/chat/completions");
    assert_eq!(probe.1["model"], "chat-up");
    assert_ne!(probe.1["stream"], json!(true));
    assert_eq!(
        probe.2.get("authorization").unwrap().to_str().unwrap(),
        "Bearer probe-key"
    );

    env.mock.reset().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_string(r#"{"error":{"message":"The model `bad` does not exist"}}"#),
        )
        .mount(&env.mock)
        .await;
    let bad = env
        .services
        .test_provider(
            input_for(&env.mock.uri(), Protocol::Anthropic),
            "bad".into(),
        )
        .await
        .unwrap();
    assert!(!bad.ok);
    assert!(
        bad.message.contains("404") && bad.message.contains("bad"),
        "{}",
        bad.message
    );

    // Unreachable host is a failed test, not an error.
    let down = env
        .services
        .test_provider(
            input_for("http://127.0.0.1:1", Protocol::Responses),
            "m".into(),
        )
        .await
        .unwrap();
    assert!(!down.ok);

    let err = env
        .services
        .test_provider(input_for(&env.mock.uri(), Protocol::Responses), "  ".into())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::Validation(_)));
}
