//! Reasoning state issued by one Responses provider is not replayed to another.

mod common;

use common::*;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};
use yi_core::domain::Protocol;

const OTHER_MODEL: &str = "name-pr2/resp-up-2";
const ISSUED: &str = "ENC-FROM-PR";

/// The encrypted reasoning item that provider `pr` issues.
fn reasoning_item() -> Value {
    json!({"type": "reasoning", "id": "rs_issued",
        "summary": [{"type": "summary_text", "text": "planning"}],
        "encrypted_content": ISSUED})
}

/// A complete Responses answer from `pr`: reasoning, then the reply text.
fn answer_with_reasoning() -> Value {
    json!({"id": "r2", "object": "response", "created_at": 1, "status": "completed",
        "model": "resp-up",
        "output": [reasoning_item(),
            {"type": "message", "id": "m1", "role": "assistant", "status": "completed",
                "content": [{"type": "output_text", "text": "hello", "annotations": []}]}],
        "usage": {"input_tokens": 3, "output_tokens": 2, "total_tokens": 5}})
}

/// A client history that replays the reasoning item between two user turns.
fn history(reasoning: Value) -> Value {
    json!([
        {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]},
        reasoning,
        {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "again"}]}
    ])
}

async fn install_other_provider(env: &TestEnv) {
    let root = env.mock.uri();
    env.install(
        provider(
            "pr2",
            "rs2",
            Protocol::Responses,
            &format!("{root}/other/v1"),
        ),
        model("mr2", "resp-up-2"),
    );
    Mock::given(method("POST"))
        .and(path("/other/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(upstream_response(Protocol::Responses)),
        )
        .mount(&env.mock)
        .await;
}

fn assert_stripped_to_summary(item: &Value) {
    assert!(item.get("id").is_none(), "foreign id was sent: {item}");
    assert!(
        item.get("encrypted_content").is_none(),
        "foreign state was sent: {item}"
    );
    assert_eq!(item["summary"][0]["text"], "planning");
}

#[tokio::test]
async fn reasoning_from_a_complete_answer_is_not_replayed_to_another_provider() {
    let env = TestEnv::start().await;
    install_other_provider(&env).await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(answer_with_reasoning()))
        .mount(&env.mock)
        .await;

    let resp = env
        .post(
            "/v1/responses",
            &json!({"model": RESP_MODEL, "input": "hi"}),
        )
        .await;
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.json::<Value>().await.unwrap()["output"][0],
        reasoning_item()
    );

    let replay = json!({"model": "", "input": history(reasoning_item())});
    let mut to_other = replay.clone();
    to_other["model"] = json!(OTHER_MODEL);
    assert_eq!(env.post("/v1/responses", &to_other).await.status(), 200);
    let mut to_issuer = replay;
    to_issuer["model"] = json!(RESP_MODEL);
    assert_eq!(env.post("/v1/responses", &to_issuer).await.status(), 200);

    let reqs = env.upstream_requests().await;
    assert_eq!(reqs.len(), 3);
    assert_eq!(reqs[1].0, "/other/v1/responses");
    assert_stripped_to_summary(&reqs[1].1["input"][1]);
    assert_eq!(
        reqs[2].1["input"][1],
        reasoning_item(),
        "the issuer keeps its own state"
    );
}

#[tokio::test]
async fn reasoning_from_a_stream_is_not_replayed_to_another_provider() {
    let env = TestEnv::start().await;
    install_other_provider(&env).await;
    let frames = format!(
        "event: response.output_item.done\ndata: {}\n\nevent: response.completed\ndata: {}\n\n",
        json!({"type": "response.output_item.done", "output_index": 0, "item": reasoning_item()}),
        json!({"type": "response.completed", "response": answer_with_reasoning()}),
    );
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(frames),
        )
        .mount(&env.mock)
        .await;

    let resp = env
        .post(
            "/v1/responses",
            &json!({"model": RESP_MODEL, "input": "hi", "stream": true}),
        )
        .await;
    assert_eq!(resp.status(), 200);
    // Origins are recorded while the client reads the stream.
    resp.text().await.unwrap();

    let mut to_other = json!({"input": history(reasoning_item())});
    to_other["model"] = json!(OTHER_MODEL);
    assert_eq!(env.post("/v1/responses", &to_other).await.status(), 200);

    let reqs = env.upstream_requests().await;
    assert_eq!(reqs.len(), 2);
    assert_stripped_to_summary(&reqs[1].1["input"][1]);
}
