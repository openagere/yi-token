use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;
use yi_core::CoreError;
use yi_core::domain::{
    Capabilities, Client, Model, ProfileModel, Protocol, Provider, ProxyProfile, RouteMode,
};
use yi_core::proxy::resolve::{Routes, Scope, resolve};
use yi_core::proxy::router::{ProxyState, build};
use yi_core::services::Services;
use yi_core::storage::route_source::DbRouteSource;
use yi_core::storage::{Db, providers, terminal};

fn provider(id: &str, _code: &str, protocols: &[Protocol]) -> Provider {
    Provider {
        id: id.into(),
        provider_type: protocols[0],
        name: format!("name-{id}"),
        base_url: "http://127.0.0.1:1".into(),
        api_key: "sk".into(),
        enabled: true,
        thinking: None,
        is_default: false,
        sort_order: 0,
        extra: serde_json::json!({}),
        protocols: protocols.to_vec(),
    }
}

fn model(id: &str, upstream: &str) -> Model {
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

fn save(db: &Db, p: &Provider, ms: &[Model]) {
    db.with_tx(|tx| providers::upsert(tx, p, ms)).unwrap();
}

fn routes(db: &Db) -> Routes {
    Routes::new(Arc::new(DbRouteSource { db: db.clone() })).unwrap()
}

fn fixed_profile(client: Client, protocol: Protocol, pid: &str, m: &str) -> ProxyProfile {
    ProxyProfile {
        client,
        protocol,
        route_mode: RouteMode::Fixed,
        models: vec![ProfileModel {
            provider_id: pid.into(),
            model: m.into(),
        }],
        default_model: Some(m.into()),
    }
}

#[test]
fn global_exact_match() {
    let db = Db::open_in_memory().unwrap();
    save(
        &db,
        &provider("p1", "oa", &[Protocol::OpenaiChat]),
        &[model("m1", "gpt-5")],
    );
    let snap = routes(&db).load();
    let r = resolve(&snap, Scope::Global, Protocol::OpenaiChat, "name-p1/gpt-5").unwrap();
    assert_eq!(r.provider.id, "p1");
    assert_eq!(r.upstream_model, "gpt-5");
    assert_eq!(r.model.unwrap().id, "m1");
    assert!(r.route_mode.is_none());
}

#[test]
fn global_requires_protocol_support() {
    let db = Db::open_in_memory().unwrap();
    save(
        &db,
        &provider("p1", "oa", &[Protocol::OpenaiChat]),
        &[model("m1", "gpt-5")],
    );
    let snap = routes(&db).load();
    // No default provider, and p1 does not speak Anthropic.
    let err = resolve(&snap, Scope::Global, Protocol::Anthropic, "name-p1/gpt-5").unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)));
}

#[test]
fn global_falls_back_to_default_provider() {
    let db = Db::open_in_memory().unwrap();
    save(
        &db,
        &provider("p1", "oa", &[Protocol::OpenaiChat]),
        &[model("m1", "gpt-5")],
    );
    let mut d = provider("p2", "dd", &[Protocol::Anthropic]);
    d.is_default = true;
    save(&db, &d, &[]);
    let snap = routes(&db).load();
    let r = resolve(&snap, Scope::Global, Protocol::OpenaiChat, "whatever").unwrap();
    assert_eq!(r.provider.id, "p2");
    assert_eq!(r.upstream_model, "whatever");
    assert!(r.model.is_none());
    // A protocol mismatch on an exposed name also falls back, name unchanged.
    let r = resolve(&snap, Scope::Global, Protocol::Anthropic, "name-p1/gpt-5").unwrap();
    assert_eq!(r.provider.id, "p2");
    assert_eq!(r.upstream_model, "name-p1/gpt-5");
}

#[test]
fn disabled_provider_excluded() {
    let db = Db::open_in_memory().unwrap();
    let mut p = provider("p1", "oa", &[Protocol::OpenaiChat]);
    p.is_default = true;
    p.enabled = false;
    save(&db, &p, &[model("m1", "gpt-5")]);
    let snap = routes(&db).load();
    assert!(snap.providers.is_empty() && snap.by_exposed.is_empty() && snap.models.is_empty());
    assert!(snap.default_provider.is_none());
    let err = resolve(&snap, Scope::Global, Protocol::OpenaiChat, "name-p1/gpt-5").unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)));
}

#[test]
fn fixed_profile_resolves_model() {
    let db = Db::open_in_memory().unwrap();
    save(
        &db,
        &provider("p1", "oa", &[Protocol::Responses]),
        &[model("m1", "gpt-5"), model("m2", "gpt-6")],
    );
    db.with_conn(|c| {
        terminal::save_proxy(
            c,
            &fixed_profile(Client::Codex, Protocol::Responses, "p1", "name-p1/gpt-5"),
        )
    })
    .unwrap();
    let snap = routes(&db).load();
    let scope = Scope::Client(Client::Codex);
    let r = resolve(&snap, scope, Protocol::Responses, "name-p1/gpt-5").unwrap();
    assert_eq!(r.upstream_model, "gpt-5");
    assert_eq!(r.route_mode, Some(RouteMode::Fixed));
    // The bare upstream name is not an alias.
    let err = resolve(&snap, scope, Protocol::Responses, "gpt-5").unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)));
    // A model outside the profile is not routable even though it exists.
    let err = resolve(&snap, scope, Protocol::Responses, "gpt-6").unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)));
}

#[test]
fn shared_upstream_name_never_crosses_providers() {
    let db = Db::open_in_memory().unwrap();
    save(
        &db,
        &provider("pa", "a", &[Protocol::Responses]),
        &[model("ma", "x")],
    );
    save(
        &db,
        &provider("pb", "b", &[Protocol::Responses]),
        &[model("mb", "x")],
    );
    db.with_conn(|c| {
        terminal::save_proxy(
            c,
            &fixed_profile(Client::Codex, Protocol::Responses, "pa", "name-pa/x"),
        )?;
        // Entry whose exposed name belongs to another provider.
        terminal::save_proxy(
            c,
            &fixed_profile(Client::Pi, Protocol::Responses, "pa", "name-pb/x"),
        )
    })
    .unwrap();
    let snap = routes(&db).load();
    let r = resolve(
        &snap,
        Scope::Client(Client::Codex),
        Protocol::Responses,
        "name-pa/x",
    )
    .unwrap();
    assert_eq!(r.provider.id, "pa");
    let err = resolve(
        &snap,
        Scope::Client(Client::Codex),
        Protocol::Responses,
        "x",
    )
    .unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)));
    let err = resolve(
        &snap,
        Scope::Client(Client::Pi),
        Protocol::Responses,
        "name-pb/x",
    )
    .unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)));
}

#[test]
fn client_protocol_mismatch_is_validation() {
    let db = Db::open_in_memory().unwrap();
    save(
        &db,
        &provider("p1", "oa", &[Protocol::Responses, Protocol::Anthropic]),
        &[model("m1", "gpt-5")],
    );
    db.with_conn(|c| {
        terminal::save_proxy(
            c,
            &fixed_profile(Client::Codex, Protocol::Responses, "p1", "name-p1/gpt-5"),
        )
    })
    .unwrap();
    let snap = routes(&db).load();
    let err = resolve(
        &snap,
        Scope::Client(Client::Codex),
        Protocol::Anthropic,
        "name-p1/gpt-5",
    )
    .unwrap_err();
    assert!(matches!(err, CoreError::Validation(_)));
}

#[test]
fn client_without_profile_is_not_found() {
    let db = Db::open_in_memory().unwrap();
    save(
        &db,
        &provider("p1", "oa", &[Protocol::Responses]),
        &[model("m1", "gpt-5")],
    );
    let snap = routes(&db).load();
    let err = resolve(
        &snap,
        Scope::Client(Client::Codex),
        Protocol::Responses,
        "name-p1/gpt-5",
    )
    .unwrap_err();
    assert!(matches!(&err, CoreError::NotFound(m) if m.contains("尚未配置代理")));
}

#[test]
fn auto_without_engine_is_conflict() {
    let db = Db::open_in_memory().unwrap();
    db.with_conn(|c| {
        terminal::save_proxy(
            c,
            &ProxyProfile {
                client: Client::Codex,
                protocol: Protocol::Responses,
                route_mode: RouteMode::Auto,
                models: vec![],
                default_model: None,
            },
        )
    })
    .unwrap();
    let snap = routes(&db).load();
    let err = resolve(
        &snap,
        Scope::Client(Client::Codex),
        Protocol::Responses,
        "yi-token",
    )
    .unwrap_err();
    assert!(matches!(err, CoreError::Conflict(_)));
}

#[test]
fn refresh_swaps_snapshot() {
    let db = Db::open_in_memory().unwrap();
    let p = provider("p1", "oa", &[Protocol::OpenaiChat]);
    save(&db, &p, &[model("m1", "gpt-5")]);
    let routes = routes(&db);
    let old = routes.load();
    assert!(resolve(&old, Scope::Global, Protocol::OpenaiChat, "name-p1/gpt-5").is_ok());

    save(&db, &p, &[model("m1", "gpt-5"), model("m2", "gpt-6")]);
    // Not visible until refresh.
    assert!(!routes.load().by_exposed.contains_key("name-p1/gpt-6"));
    routes.refresh().unwrap();
    let new = routes.load();
    assert!(resolve(&new, Scope::Global, Protocol::OpenaiChat, "name-p1/gpt-6").is_ok());
    // The old Arc is untouched and still usable.
    assert!(!old.by_exposed.contains_key("name-p1/gpt-6"));
    assert!(resolve(&old, Scope::Global, Protocol::OpenaiChat, "name-p1/gpt-5").is_ok());
    assert!(!Arc::ptr_eq(&old, &new));
}

#[test]
fn deleting_a_provider_makes_fixed_resolve_not_found() {
    let db = Db::open_in_memory().unwrap();
    save(
        &db,
        &provider("p1", "oa", &[Protocol::Responses]),
        &[model("m1", "gpt-5")],
    );
    db.with_conn(|c| {
        terminal::save_proxy(
            c,
            &fixed_profile(Client::Codex, Protocol::Responses, "p1", "name-p1/gpt-5"),
        )
    })
    .unwrap();
    let routes = routes(&db);
    let scope = Scope::Client(Client::Codex);
    assert!(resolve(&routes.load(), scope, Protocol::Responses, "name-p1/gpt-5").is_ok());
    db.with_conn(|c| providers::delete(c, "p1")).unwrap();
    routes.refresh().unwrap();
    let err = resolve(&routes.load(), scope, Protocol::Responses, "name-p1/gpt-5").unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)));
}

fn app() -> (axum::Router, Arc<Services>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let services = Services::for_test(dir.path());
    let router = build(ProxyState::new(services.routes(), services.usage_sink()));
    (router, services, dir)
}

fn post_json(uri: &str, body: Vec<u8>) -> Request<Body> {
    Request::post(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap()
}

#[tokio::test]
async fn unknown_client_path_is_404() {
    let (router, _s, _d) = app();
    for (method, uri) in [
        ("GET", "/clients/nope/v1/models"),
        ("POST", "/clients/nope/v1/responses"),
        ("POST", "/clients/nope/responses"),
        ("POST", "/clients/nope/v1/messages"),
        ("POST", "/clients/nope/v1/chat/completions"),
    ] {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"model":"x"}"#))
            .unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{uri}");
    }
}

#[tokio::test]
async fn large_body_is_not_rejected() {
    let (router, _s, _d) = app();
    let body = serde_json::to_vec(&serde_json::json!({
        "model": "m",
        "input": "x".repeat(8 * 1024 * 1024),
    }))
    .unwrap();
    for uri in ["/clients/codex/v1/responses", "/v1/responses"] {
        let resp = router
            .clone()
            .oneshot(post_json(uri, body.clone()))
            .await
            .unwrap();
        assert_ne!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE, "{uri}");
    }
}

#[tokio::test]
async fn health_and_models_listing() {
    let (router, s, _d) = app();
    let resp = router
        .clone()
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    s.db()
        .with_tx(|tx| {
            providers::upsert(
                tx,
                &provider("p1", "oa", &[Protocol::Responses]),
                &[model("m1", "gpt-5")],
            )
        })
        .unwrap();
    s.routes().refresh().unwrap();
    let resp = router
        .oneshot(Request::get("/v1/models").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["object"], "list");
    assert!(
        v["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["id"] == "name-p1/gpt-5" && e["owned_by"] == "name-p1")
    );
}

#[tokio::test]
async fn client_models_lists_exposed_names() {
    let (router, s, _d) = app();
    s.db()
        .with_tx(|tx| {
            providers::upsert(
                tx,
                &provider("p1", "oa", &[Protocol::Responses]),
                &[model("m1", "gpt-5")],
            )
        })
        .unwrap();
    s.db()
        .with_conn(|c| {
            terminal::save_proxy(
                c,
                &fixed_profile(Client::Codex, Protocol::Responses, "p1", "name-p1/gpt-5"),
            )
        })
        .unwrap();
    s.routes().refresh().unwrap();
    let resp = router
        .oneshot(
            Request::get("/clients/codex/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let data = v["data"].as_array().unwrap();
    assert_eq!(data.len(), 2);
    assert_eq!(data[0]["id"], "yi-token");
    assert_eq!(data[0]["owned_by"], "yi-token");
    assert_eq!(data[1]["id"], "name-p1/gpt-5");
    assert_eq!(data[1]["owned_by"], "name-p1");
}
