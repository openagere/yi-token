//! Request pipeline (spec 5.2 - 5.5): parse, resolve, prepare, send, relay.
//!
//! Large-body choice: an inference body can be many MiB (agent histories).
//! Parsing and rebuilding it is CPU work that would stall the async worker, so
//! bodies of [`OFFLOAD_THRESHOLD`] bytes or more are parsed, prepared and
//! response-translated on the blocking pool (`spawn_blocking`); small ones run
//! inline to avoid the hop. The body is not streamed into the parser because
//! the model name and `stream` flag are needed before routing.

use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures::StreamExt;
use serde_json::Value;

use super::headers;
use super::resolve::{Resolved, Scope, resolve};
use super::router::ProxyState;
use super::upstream::{endpoint, unreachable_error};
use crate::domain::{Client, Protocol, RouteMode, Thinking, UsageEvent};
use crate::error::{CoreError, CoreResult};
use crate::protocol::prepare::{self, PrepareCtx};
use crate::protocol::sse::Decoder;
use crate::protocol::{
    OutboundCtx, Usage, build_upstream, choose_upstream, stream_translator_with_usage,
    thinking as thinking_rules, to_ir, translate_response,
};
use crate::storage::usage_writer::UsageSink;

/// Bodies at least this large are processed off the async worker threads.
pub const OFFLOAD_THRESHOLD: usize = 256 * 1024;

/// Records exactly one usage event per request. A request that finished
/// (normally or with an upstream error mid-stream) always records; one that was
/// dropped by the client records only when some tokens were already counted.
struct UsageGuard {
    sink: UsageSink,
    meta: Option<UsageMeta>,
    usage: Usage,
    finished: bool,
}

struct UsageMeta {
    requested_at: i64,
    client: Option<Client>,
    route_mode: Option<RouteMode>,
    provider_name: String,
    protocol: Protocol,
    model: String,
}

impl UsageGuard {
    fn finish(&mut self, usage: Usage) {
        self.usage = usage;
        self.finished = true;
    }
}

fn clamp(n: u64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

impl Drop for UsageGuard {
    fn drop(&mut self) {
        let u = self.usage;
        let any = u.input_tokens + u.output_tokens + u.cached_tokens + u.reasoning_tokens > 0;
        if !(self.finished || any) {
            return;
        }
        if let Some(m) = self.meta.take() {
            self.sink.record(UsageEvent {
                requested_at: m.requested_at,
                client: m.client,
                route_mode: m.route_mode,
                provider_name: m.provider_name,
                protocol: m.protocol,
                model: m.model,
                input_tokens: clamp(u.input_tokens),
                output_tokens: clamp(u.output_tokens),
                cached_tokens: clamp(u.cached_tokens),
                reasoning_tokens: clamp(u.reasoning_tokens),
            });
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

async fn offload<T: Send + 'static>(
    len: usize,
    f: impl FnOnce() -> CoreResult<T> + Send + 'static,
) -> CoreResult<T> {
    if len < OFFLOAD_THRESHOLD {
        return f();
    }
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| CoreError::Internal(format!("后台任务失败：{e}")))?
}

/// What to send upstream.
struct Plan {
    upstream: Protocol,
    passthrough: bool,
    body: Bytes,
    include_usage: bool,
}

/// Builds the upstream request body. `raw` is the client's body as sent.
fn prepare(raw: Value, inbound: Protocol, r: &Resolved) -> CoreResult<Plan> {
    let provider = &r.provider;
    let (upstream, passthrough) =
        choose_upstream(inbound, &provider.protocols, provider.provider_type);
    let capabilities = r
        .model
        .as_ref()
        .map(|m| m.capabilities.clone())
        .unwrap_or_default();
    let fallback = provider.thinking.unwrap_or(Thinking::Medium);
    // `to_ir` strips stream_options, so the client's wish is read from the raw body.
    let include_usage = inbound == Protocol::OpenaiChat
        && raw
            .pointer("/stream_options/include_usage")
            .and_then(Value::as_bool)
            == Some(true);
    let body = if passthrough {
        let mut raw = raw;
        let ctx = PrepareCtx {
            inbound,
            upstream,
            upstream_model: r.upstream_model.clone(),
            capabilities,
            // `apply_raw` itself leaves a request that has its own reasoning alone.
            thinking: Some(fallback),
            non_standard: r.model.as_ref().is_some_and(|m| m.non_standard),
        };
        prepare::passthrough(&mut raw, &ctx)?;
        serde_json::to_vec(&raw)?
    } else {
        // A client that chose its own reasoning effort keeps it.
        let thinking = if thinking_rules::has_reasoning(inbound, &raw) {
            None
        } else {
            Some(fallback)
        };
        let ir = to_ir(inbound, raw)?;
        prepare::validate_for_translation(&ir, upstream)?;
        let ctx = OutboundCtx {
            upstream_model: r.upstream_model.clone(),
            capabilities,
            thinking,
        };
        serde_json::to_vec(&build_upstream(&ir, upstream, &ctx)?)?
    };
    Ok(Plan {
        upstream,
        passthrough,
        body: Bytes::from(body),
        include_usage,
    })
}

pub async fn handle(
    state: &ProxyState,
    scope: Scope,
    inbound: Protocol,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match run(state, scope, inbound, &headers, body).await {
        Ok(resp) => resp,
        Err(e) => e.into_response(),
    }
}

async fn run(
    state: &ProxyState,
    scope: Scope,
    inbound: Protocol,
    inbound_headers: &HeaderMap,
    body: Bytes,
) -> CoreResult<Response> {
    let len = body.len();
    let raw: Value = offload(len, move || {
        serde_json::from_slice(&body)
            .map_err(|_| CoreError::Validation("请求不是有效的 JSON".into()))
    })
    .await?;
    let model = raw
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| CoreError::Validation("请求缺少 model 字段".into()))?;
    let stream = raw.get("stream").and_then(Value::as_bool).unwrap_or(false);

    // The snapshot is read once: everything below uses this resolution, so an
    // Auto engine switch or a provider edit mid-request cannot split it.
    let resolved = resolve(&state.routes.load(), scope, inbound, &model)?;
    let plan = {
        let r = resolved.clone();
        offload(len, move || prepare(raw, inbound, &r)).await?
    };

    let usage_model = resolved.model.as_ref().map_or_else(
        || resolved.upstream_model.clone(),
        |m| m.exposed_name.clone(),
    );
    let mut guard = UsageGuard {
        sink: state.usage.clone(),
        meta: Some(UsageMeta {
            requested_at: now_ms(),
            client: match scope {
                Scope::Client(c) => Some(c),
                Scope::Global => None,
            },
            route_mode: resolved.route_mode,
            provider_name: resolved.provider.name.clone(),
            protocol: plan.upstream,
            model: usage_model,
        }),
        usage: Usage::default(),
        finished: false,
    };

    let resp = state
        .http
        .send_with(
            &resolved.provider,
            endpoint(plan.upstream),
            plan.body.clone(),
            stream,
            headers::forward(inbound_headers, &resolved.provider, plan.upstream),
        )
        .await?;
    let mut out_headers = if plan.passthrough {
        headers::response(resp.headers())
    } else {
        HeaderMap::new()
    };

    if stream {
        let mut translator =
            stream_translator_with_usage(inbound, plan.upstream, plan.include_usage);
        let events = async_stream::stream! {
            // Owning `resp` here is what cancels the upstream request: when the
            // client goes away the body is dropped, and with it this generator.
            let mut up = Box::pin(resp.bytes_stream());
            let mut decoder = Decoder::default();
            while let Some(chunk) = up.next().await {
                let bytes = match chunk {
                    Ok(b) => b,
                    Err(e) => {
                        guard.finish(translator.usage());
                        yield Err(io::Error::other(e));
                        return;
                    }
                };
                for frame in decoder.push(&bytes) {
                    match translator.on_frame(frame) {
                        Ok(out) => {
                            for b in out {
                                yield Ok::<Bytes, io::Error>(b);
                            }
                        }
                        Err(e) => {
                            guard.finish(translator.usage());
                            yield Err(io::Error::other(e.message()));
                            return;
                        }
                    }
                }
                guard.usage = translator.usage();
            }
            for b in translator.finish() {
                yield Ok(b);
            }
            guard.finish(translator.usage());
        };
        out_headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        );
        out_headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        let mut response = Body::from_stream(events).into_response();
        *response.headers_mut() = out_headers;
        return Ok(response);
    }

    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::OK);
    let bytes = resp.bytes().await.map_err(unreachable_error)?;
    let (upstream_protocol, passthrough) = (plan.upstream, plan.passthrough);
    let (out, usage) = offload(bytes.len(), move || {
        let parsed = serde_json::from_slice::<Value>(&bytes);
        if passthrough {
            // Usage is best effort; the bytes go back exactly as received.
            let usage = parsed
                .ok()
                .and_then(|v| translate_response(inbound, upstream_protocol, v).ok())
                .map(|(_, u)| u)
                .unwrap_or_default();
            return Ok((bytes, usage));
        }
        let value = parsed.map_err(|e| CoreError::Internal(format!("无法解析上游响应：{e}")))?;
        let (translated, usage) = translate_response(inbound, upstream_protocol, value)?;
        Ok((Bytes::from(serde_json::to_vec(&translated)?), usage))
    })
    .await?;
    guard.finish(usage);
    out_headers
        .entry(header::CONTENT_TYPE)
        .or_insert(HeaderValue::from_static("application/json"));
    let mut response = Response::new(Body::from(out));
    *response.status_mut() = status;
    *response.headers_mut() = out_headers;
    Ok(response)
}
