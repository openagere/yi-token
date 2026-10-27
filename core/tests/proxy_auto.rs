mod common;

use std::time::Duration;

use common::*;
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, ResponseTemplate};
use yi_core::domain::{
    Client, DirectProfile, ModelSource, ProfileModel, Protocol, ProxyProfile, RouteMode,
};
use yi_core::services::TerminalProfileView;
use yi_core::storage::{engines, terminal};

fn auto_profile(client: Client, protocol: Protocol) -> ProxyProfile {
    ProxyProfile {
        client,
        protocol,
        route_mode: RouteMode::Auto,
        models: Vec::new(),
        default_model: None,
    }
}

fn save_auto(env: &TestEnv, client: Client, protocol: Protocol) {
    env.services
        .db()
        .with_conn(|conn| terminal::save_proxy(conn, &auto_profile(client, protocol)))
        .unwrap();
    env.services.routes().refresh().unwrap();
}

fn auto_view(client: Client, protocol: Protocol) -> TerminalProfileView {
    TerminalProfileView {
        client,
        models: Vec::new(),
        default_model: String::new(),
        protocol: Some(protocol),
        route_mode: RouteMode::Auto,
        active_engine: None,
    }
}

async fn mount_json(env: &TestEnv, protocol: Protocol) {
    Mock::given(method("POST"))
        .and(path(upstream_path(protocol)))
        .respond_with(ResponseTemplate::new(200).set_body_json(upstream_response(protocol)))
        .mount(&env.mock)
        .await;
}

#[tokio::test]
async fn auto_without_engine_is_409() {
    let env = TestEnv::start().await;
    save_auto(&env, Client::Codex, Protocol::Responses);
    let resp = env
        .post(
            "/clients/codex/v1/responses",
            &request_body(Protocol::Responses, "yi-token"),
        )
        .await;
    assert_eq!(resp.status(), 409);
    let body: Value = resp.json().await.unwrap();
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("选择当前引擎"),
        "{body}"
    );
}

#[tokio::test]
async fn auto_routes_to_engine_regardless_of_model_name() {
    let env = TestEnv::start().await;
    mount_json(&env, Protocol::Responses).await;
    save_auto(&env, Client::Codex, Protocol::Responses);
    env.services
        .set_active_engine(Client::Codex, Some("mr".into()))
        .await
        .unwrap();

    for name in ["yi-token", "whatever"] {
        let resp = env
            .post(
                "/clients/codex/v1/responses",
                &request_body(Protocol::Responses, name),
            )
            .await;
        assert_eq!(resp.status(), 200, "{name}");
        let got: Value = resp.json().await.unwrap();
        assert_eq!(response_text(Protocol::Responses, &got), "hello");
    }
    let requests = env.upstream_requests().await;
    assert_eq!(requests.len(), 2);
    for (path, body, _) in requests {
        assert_eq!(path, "/v1/responses");
        assert_eq!(body["model"], "resp-up");
    }
}

#[tokio::test]
async fn auto_cross_protocol() {
    let env = TestEnv::start().await;
    mount_json(&env, Protocol::Anthropic).await;
    save_auto(&env, Client::Codex, Protocol::Responses);
    env.services
        .set_active_engine(Client::Codex, Some("ma".into()))
        .await
        .unwrap();

    let resp = env
        .post(
            "/clients/codex/v1/responses",
            &request_body(Protocol::Responses, "yi-token"),
        )
        .await;
    assert_eq!(resp.status(), 200);
    let got: Value = resp.json().await.unwrap();
    assert_eq!(response_text(Protocol::Responses, &got), "hello");
    let requests = env.upstream_requests().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, "/v1/messages");
    assert_eq!(requests[0].1["model"], "claude-up");
}

#[tokio::test]
async fn auto_switch_affects_only_new_requests() {
    let env = TestEnv::start().await;
    let base = format!("{}/v1", env.mock.uri());
    env.install(
        provider("pb", "rb", Protocol::Responses, &base),
        model("mb", "resp-b"),
    );
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(body_partial_json(json!({"model": "resp-up"})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(400))
                .set_body_json(upstream_response(Protocol::Responses)),
        )
        .mount(&env.mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(body_partial_json(json!({"model": "resp-b"})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(upstream_response(Protocol::Responses)),
        )
        .mount(&env.mock)
        .await;

    save_auto(&env, Client::Codex, Protocol::Responses);
    env.services
        .set_active_engine(Client::Codex, Some("mr".into()))
        .await
        .unwrap();

    let http = env.http.clone();
    let url = env.url("/clients/codex/v1/responses");
    let first = tokio::spawn(async move {
        http.post(url)
            .json(&request_body(Protocol::Responses, "yi-token"))
            .send()
            .await
            .unwrap()
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while env.upstream_requests().await.is_empty() && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(env.upstream_requests().await.len(), 1);

    env.services
        .set_active_engine(Client::Codex, Some("mb".into()))
        .await
        .unwrap();
    let second = env
        .post(
            "/clients/codex/v1/responses",
            &request_body(Protocol::Responses, "yi-token"),
        )
        .await;
    assert_eq!(second.status(), 200);
    let first = first.await.unwrap();
    assert_eq!(first.status(), 200);
    let first_body: Value = first.json().await.unwrap();
    assert_eq!(response_text(Protocol::Responses, &first_body), "hello");

    let models: Vec<String> = env
        .upstream_requests()
        .await
        .into_iter()
        .map(|(_, body, _)| body["model"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(models, vec!["resp-up", "resp-b"]);
}

#[tokio::test]
async fn deleting_engine_provider_resets_to_unselected() {
    let env = TestEnv::start().await;
    save_auto(&env, Client::Codex, Protocol::Responses);
    env.services
        .set_active_engine(Client::Codex, Some("mr".into()))
        .await
        .unwrap();
    env.services
        .db()
        .with_conn(|conn| {
            terminal::save_proxy(
                conn,
                &ProxyProfile {
                    client: Client::Opencode,
                    protocol: Protocol::Responses,
                    route_mode: RouteMode::Fixed,
                    models: vec![ProfileModel {
                        provider_id: "pr".into(),
                        model: RESP_MODEL.into(),
                    }],
                    default_model: Some(RESP_MODEL.into()),
                },
            )?;
            terminal::save_direct(
                conn,
                &DirectProfile {
                    client: Client::Codex,
                    provider_id: "pr".into(),
                    model_source: ModelSource::Native,
                    model: None,
                },
            )
        })
        .unwrap();
    env.services.routes().refresh().unwrap();
    env.services.delete_provider("pr".into()).await.unwrap();

    assert_eq!(
        env.services
            .db()
            .with_conn(|conn| engines::get(conn, Client::Codex))
            .unwrap(),
        None
    );
    assert!(
        env.services
            .db()
            .with_conn(|conn| terminal::get_direct(conn, "codex"))
            .unwrap()
            .is_none()
    );

    let auto = env
        .post(
            "/clients/codex/v1/responses",
            &request_body(Protocol::Responses, "yi-token"),
        )
        .await;
    assert_eq!(auto.status(), 409);

    let fixed = env
        .post(
            "/clients/opencode/v1/responses",
            &request_body(Protocol::Responses, RESP_MODEL),
        )
        .await;
    assert_eq!(fixed.status(), 404);
}

#[tokio::test]
async fn usage_records_client_and_route_mode() {
    let env = TestEnv::start().await;
    mount_json(&env, Protocol::Responses).await;
    save_auto(&env, Client::Codex, Protocol::Responses);
    env.services
        .set_active_engine(Client::Codex, Some("mr".into()))
        .await
        .unwrap();
    let resp = env
        .post(
            "/clients/codex/v1/responses",
            &request_body(Protocol::Responses, "yi-token"),
        )
        .await;
    assert_eq!(resp.status(), 200);
    assert_eq!(env.usage_count(1, Duration::from_secs(2)).await, 1);
    let report = env.usage_report();
    let record = &report.recent[0];
    assert_eq!(record.client, Some(Client::Codex));
    assert_eq!(record.route_mode, Some(RouteMode::Auto));
    assert_eq!(record.model, RESP_MODEL);
}

#[tokio::test]
async fn set_active_engine_does_not_touch_terminal_files() {
    let env = TestEnv::start().await;
    let applied = env
        .services
        .apply_terminal_config(auto_view(Client::Codex, Protocol::Responses))
        .await
        .unwrap();
    let path = std::path::PathBuf::from(&applied.config_path);
    let before = std::fs::read(&path).unwrap();
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();

    env.services
        .set_active_engine(Client::Codex, Some("mr".into()))
        .await
        .unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        modified
    );
    let active = env.services.get_active_engines().await.unwrap();
    assert!(
        active
            .iter()
            .any(|engine| engine.client == Client::Codex && engine.model_id == "mr")
    );
}

#[tokio::test]
async fn auto_models_endpoint_lists_only_auto() {
    let env = TestEnv::start().await;
    save_auto(&env, Client::Codex, Protocol::Responses);
    let resp = env
        .http
        .get(env.url("/clients/codex/v1/models"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let data = body["data"].as_array().unwrap();
    assert_eq!(data.len(), 1);
    assert_eq!(data[0]["id"], "yi-token");
}

#[tokio::test]
async fn fixed_profile_also_routes_auto_model_to_active_engine() {
    let env = TestEnv::start().await;
    mount_json(&env, Protocol::Responses).await;
    env.services
        .db()
        .with_conn(|conn| {
            terminal::save_proxy(
                conn,
                &ProxyProfile {
                    client: Client::Codex,
                    protocol: Protocol::Responses,
                    route_mode: RouteMode::Fixed,
                    models: vec![ProfileModel {
                        provider_id: "pr".into(),
                        model: RESP_MODEL.into(),
                    }],
                    default_model: Some(RESP_MODEL.into()),
                },
            )
        })
        .unwrap();
    env.services.routes().refresh().unwrap();
    env.services
        .set_active_engine(Client::Codex, Some("mr".into()))
        .await
        .unwrap();

    for name in ["yi-token", RESP_MODEL] {
        let resp = env
            .post(
                "/clients/codex/v1/responses",
                &request_body(Protocol::Responses, name),
            )
            .await;
        assert_eq!(resp.status(), 200, "{name}");
    }
}
