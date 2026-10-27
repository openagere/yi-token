//! Route table (spec 5.1).

use std::str::FromStr;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::HeaderMap;
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, get, post};
use serde_json::{Value, json};

use super::monitor::Monitor;
use super::pipeline;
use super::request_log;
use super::resolve::{Routes, Scope, resolve};
use super::upstream::UpstreamClient;
use crate::domain::{AUTO_MODEL, Client, Protocol, RouteMode};
use crate::error::CoreError;
use crate::storage::usage_writer::UsageSink;

/// Shared handler state.
#[derive(Clone)]
pub struct ProxyState {
    pub routes: Arc<Routes>,
    pub usage: UsageSink,
    pub http: UpstreamClient,
    pub monitor: Monitor,
}

impl ProxyState {
    pub fn new(routes: Arc<Routes>, usage: UsageSink) -> Self {
        Self::with_monitor(routes, usage, Monitor::default())
    }

    pub fn with_monitor(routes: Arc<Routes>, usage: UsageSink, monitor: Monitor) -> Self {
        Self::with_http(routes, usage, UpstreamClient::new(), monitor)
    }

    pub fn with_http(
        routes: Arc<Routes>,
        usage: UsageSink,
        http: UpstreamClient,
        monitor: Monitor,
    ) -> Self {
        Self {
            routes,
            usage,
            http,
            monitor,
        }
    }

    pub fn in_flight(&self) -> usize {
        self.monitor.totals().active_requests as usize
    }
}

/// An inference route. Inference routes carry agent-sized histories, so the
/// body limit is disabled.
fn global_infer(protocol: Protocol) -> MethodRouter<ProxyState> {
    post(
        move |State(s): State<ProxyState>, headers: HeaderMap, body: Bytes| async move {
            pipeline::handle(&s, Scope::Global, protocol, headers, body).await
        },
    )
    .layer(DefaultBodyLimit::disable())
}

fn client_infer(protocol: Protocol) -> MethodRouter<ProxyState> {
    post(
        move |State(s): State<ProxyState>,
              Path(client): Path<String>,
              headers: HeaderMap,
              body: Bytes| async move {
            match parse_client(&client) {
                Ok(c) => pipeline::handle(&s, Scope::Client(c), protocol, headers, body).await,
                Err(e) => e.into_response(),
            }
        },
    )
    .layer(DefaultBodyLimit::disable())
}

pub fn build(state: ProxyState) -> Router {
    let monitor = state.monitor.clone();
    Router::new()
        .route("/health", get(|| async { Json(json!({"status": "ok"})) }))
        .route("/v1/models", get(global_models))
        .route("/v1/messages", global_infer(Protocol::Anthropic))
        .route("/v1/chat/completions", global_infer(Protocol::OpenaiChat))
        .route("/v1/responses", global_infer(Protocol::Responses))
        .route("/responses", global_infer(Protocol::Responses))
        .route("/clients/{client}/v1/models", get(client_models))
        .route(
            "/clients/{client}/v1/messages",
            client_infer(Protocol::Anthropic),
        )
        .route(
            "/clients/{client}/v1/chat/completions",
            client_infer(Protocol::OpenaiChat),
        )
        .route(
            "/clients/{client}/v1/responses",
            client_infer(Protocol::Responses),
        )
        .route(
            "/clients/{client}/responses",
            client_infer(Protocol::Responses),
        )
        .layer(middleware::from_fn(move |request, next| {
            request_log::log_request(monitor.clone(), request, next)
        }))
        .with_state(state)
}

fn parse_client(s: &str) -> Result<Client, CoreError> {
    Client::from_str(s).map_err(|_| CoreError::NotFound(format!("未知终端 {s}")))
}

fn model_entry(id: &str, owner: &str) -> Value {
    json!({"id": id, "object": "model", "created": 0, "owned_by": owner})
}

async fn global_models(State(s): State<ProxyState>) -> Response {
    let snap = s.routes.load();
    let mut entries: Vec<(&String, &str)> = snap
        .by_exposed
        .iter()
        .filter_map(|(name, (pid, _))| {
            snap.providers
                .get(pid)
                .map(|r| (name, r.provider.name.as_str()))
        })
        .collect();
    entries.sort();
    let data: Vec<_> = entries
        .into_iter()
        .map(|(n, o)| model_entry(n, o))
        .collect();
    Json(json!({"object": "list", "data": data})).into_response()
}

async fn client_models(State(s): State<ProxyState>, Path(client): Path<String>) -> Response {
    let client = match parse_client(&client) {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    let snap = s.routes.load();
    let Some(profile) = snap.proxy_profiles.get(&client) else {
        return CoreError::NotFound(format!("终端 {} 尚未配置代理", client.as_str()))
            .into_response();
    };
    let mut data: Vec<Value> = vec![model_entry(AUTO_MODEL, "yi-token")];
    if profile.route_mode == RouteMode::Fixed {
        data.extend(profile.models.iter().filter_map(|pm| {
            resolve(&snap, Scope::Client(client), profile.protocol, &pm.model)
                .ok()
                .and_then(|r| {
                    r.model
                        .map(|m| model_entry(&m.exposed_name, &r.provider.name))
                })
        }));
    }
    Json(json!({"object": "list", "data": data})).into_response()
}
