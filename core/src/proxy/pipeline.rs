//! Request pipeline (spec 5.2 - 5.5): parse, resolve, prepare, send, relay.
//!
//! Large-body choice: an inference body can be many MiB (agent histories).
//! Parsing and rebuilding it is CPU work that would stall the async worker, so
//! bodies of [`OFFLOAD_THRESHOLD`] bytes or more are parsed, prepared and
//! response-translated on the blocking pool (`spawn_blocking`); small ones run
//! inline to avoid the hop. The body is not streamed into the parser because
//! the model name and `stream` flag are needed before routing.

use std::collections::HashSet;
use std::io;
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures::StreamExt;
use serde_json::Value;
use uuid::Uuid;

use super::capture::{CaptureTap, now_ms, stop_reason};
use super::headers;
use super::live::LiveRegistry;
use super::resolve::{Resolved, Scope, resolve};
use super::router::ProxyState;
use super::upstream::{endpoint, unreachable_error};
use crate::domain::{Client, LivePhase, Protocol, RouteMode, Thinking, UsageEvent};
use crate::error::{CoreError, CoreResult};
use crate::protocol::ir;
use crate::protocol::prepare::{self, PrepareCtx};
use crate::protocol::sse::Decoder;
use crate::protocol::{
    OutboundCtx, Usage, build_upstream, choose_upstream, stream_translator_with_usage,
    thinking as thinking_rules, to_ir, translate_response_parsed,
};
use crate::storage::usage_writer::UsageSink;
use crate::trajectory::keys::extract;
use crate::trajectory::probe_shape;
use crate::trajectory::record::{CaptureRecord, CaptureSink, ClientKeys};

/// Bodies at least this large are processed off the async worker threads.
pub const OFFLOAD_THRESHOLD: usize = 256 * 1024;

/// Marks `key` as logged and reports whether this call is the first to do so.
/// When `enabled` is false the probe event would be dropped, so the key is left
/// unmarked and a later call with the level raised can still log it.
fn claim_probe(seen: &Mutex<HashSet<String>>, key: String, enabled: bool) -> bool {
    if !enabled {
        return false;
    }
    match seen.lock() {
        Ok(mut set) => set.insert(key),
        Err(poisoned) => poisoned.into_inner().insert(key),
    }
}

/// Logs the request shape (names only) once per `(scope, inbound)` pair for the
/// life of the process. Diagnostic only; target `yi_token::probe`. The level is
/// runtime-changeable, so a pair is marked only when the event is enabled.
fn log_probe_once(scope: Scope, inbound: Protocol, headers: &HeaderMap, raw: &Value) {
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let enabled = tracing::enabled!(target: "yi_token::probe", tracing::Level::DEBUG);
    if !enabled {
        return;
    }
    let key = format!("{scope:?}/{inbound:?}");
    let seen = SEEN.get_or_init(|| Mutex::new(HashSet::new()));
    if claim_probe(seen, key, enabled) {
        tracing::debug!(
            target: "yi_token::probe",
            client = ?scope,
            inbound = ?inbound,
            shape = %probe_shape(headers, raw)
        );
    }
}

/// Per-request facts fixed when the request is routed.
struct RequestMeta {
    requested_at: i64,
    client: Option<Client>,
    route_mode: Option<RouteMode>,
    provider_name: String,
    inbound: Protocol,
    upstream: Protocol,
    upstream_model: String,
    /// Exposed model name, as written to usage and capture.
    model: String,
}

/// Owns one request's bookkeeping and is dropped exactly once when it ends
/// (normal end, upstream error, or client disconnect). On drop it records the
/// usage row, closes the live entry and submits the capture record.
struct RequestGuard {
    sink: UsageSink,
    /// Taken on drop, which is what keeps delivery to exactly once.
    meta: Option<RequestMeta>,
    usage: Usage,
    finished: bool,
    step_id: String,
    live: LiveRegistry,
    /// Set for streams; reports the first token and collects the response.
    tap: Option<CaptureTap>,
    first_token_at: Option<i64>,
    status: Option<u16>,
    error: Option<String>,
    /// Capture sink, taken for this request only (`None` when capture is off).
    capture: Option<Arc<dyn CaptureSink>>,
    keys: ClientKeys,
    request_body: Option<Bytes>,
    response: Option<ir::Response>,
    response_body: Option<Bytes>,
}

impl RequestGuard {
    fn finish(&mut self, usage: Usage) {
        self.usage = usage;
        self.finished = true;
    }

    /// Records why the request failed. Only the upstream error body is kept, so a
    /// failed request is distinguished from a client that disconnected.
    fn fail(&mut self, e: &CoreError) {
        if let CoreError::Upstream(u) = e {
            self.status = Some(u.status);
        }
        self.error = Some(truncate(e.message(), ERROR_LIMIT));
    }
}

/// Stored error text is cut to this many bytes.
const ERROR_LIMIT: usize = 4096;

/// Cuts `s` to at most `limit` bytes, on a character boundary.
fn truncate(mut s: String, limit: usize) -> String {
    if s.len() > limit {
        let mut end = limit;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

fn clamp(n: u64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Milliseconds from `from` to `to`, or `None` when out of range.
fn elapsed_ms(from: i64, to: i64) -> Option<u32> {
    u32::try_from(to.saturating_sub(from).max(0)).ok()
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        let Some(meta) = self.meta.take() else {
            return;
        };
        let finished_at = now_ms();
        let (tap_response, tap_first, tap_error) = match self.tap.take() {
            Some(tap) => tap.take(),
            None => (None, None, None),
        };
        let first_token_at = self.first_token_at.or(tap_first);
        let response = self.response.take().or(tap_response);
        let error = self.error.take().or(tap_error);
        let u = self.usage;
        let phase = if self.finished && error.is_none() && self.status.is_none_or(|s| s < 400) {
            LivePhase::Done
        } else {
            LivePhase::Error
        };
        let stop = stop_reason(
            response.as_ref(),
            self.status,
            self.finished,
            error.as_deref(),
        );

        let any = u.input_tokens + u.output_tokens + u.cached_tokens + u.reasoning_tokens > 0;
        if self.finished || any {
            self.sink.record(UsageEvent {
                requested_at: meta.requested_at,
                client: meta.client,
                route_mode: meta.route_mode,
                provider_name: meta.provider_name.clone(),
                protocol: meta.upstream,
                model: meta.model.clone(),
                input_tokens: clamp(u.input_tokens),
                output_tokens: clamp(u.output_tokens),
                cached_tokens: clamp(u.cached_tokens),
                reasoning_tokens: clamp(u.reasoning_tokens),
                step_id: Some(self.step_id.clone()),
                session_id: None,
                ttft_ms: first_token_at.and_then(|t| elapsed_ms(meta.requested_at, t)),
                duration_ms: elapsed_ms(meta.requested_at, finished_at),
                status: self.status,
            });
        }

        self.live
            .finish(&self.step_id, phase, self.status, u, Some(finished_at));

        if let Some(sink) = self.capture.take() {
            sink.submit(CaptureRecord {
                step_id: self.step_id.clone(),
                client: meta.client,
                keys: std::mem::take(&mut self.keys),
                inbound: meta.inbound,
                upstream: meta.upstream,
                provider_name: meta.provider_name,
                model: meta.model,
                upstream_model: meta.upstream_model,
                route_mode: meta.route_mode,
                requested_at: meta.requested_at,
                first_token_at,
                finished_at: self.finished.then_some(finished_at),
                status: self.status,
                stop_reason: stop,
                error,
                usage: u,
                request_body: self.request_body.take(),
                response,
                response_body: self.response_body.take(),
            });
        }
    }
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
    let client = match scope {
        Scope::Client(c) => Some(c),
        Scope::Global => None,
    };
    // The sink is read once: a sink swapped mid-request does not split it.
    let sink = state.capture.get().filter(|c| c.enabled_for(client));
    let request_body = sink.as_ref().map(|_| body.clone());
    let raw: Value = offload(len, move || {
        serde_json::from_slice(&body)
            .map_err(|_| CoreError::Validation("请求不是有效的 JSON".into()))
    })
    .await?;
    log_probe_once(scope, inbound, inbound_headers, &raw);
    let keys = if sink.is_some() {
        extract(client, inbound_headers, &raw)
    } else {
        ClientKeys::default()
    };
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
    let step_id = Uuid::new_v4().to_string();
    let requested_at = now_ms();
    state.live.begin(
        &step_id,
        client,
        &usage_model,
        &resolved.provider.name,
        requested_at,
    );
    let mut guard = RequestGuard {
        sink: state.usage.clone(),
        meta: Some(RequestMeta {
            requested_at,
            client,
            route_mode: resolved.route_mode,
            provider_name: resolved.provider.name.clone(),
            inbound,
            upstream: plan.upstream,
            upstream_model: resolved.upstream_model.clone(),
            model: usage_model,
        }),
        usage: Usage::default(),
        finished: false,
        step_id: step_id.clone(),
        live: state.live.clone(),
        tap: None,
        first_token_at: None,
        status: None,
        error: None,
        capture: sink,
        keys,
        request_body,
        response: None,
        response_body: None,
    };

    let resp = match state
        .http
        .send_with(
            &resolved.provider,
            endpoint(plan.upstream),
            plan.body.clone(),
            stream,
            headers::forward(inbound_headers, &resolved.provider, plan.upstream),
        )
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            guard.fail(&e);
            return Err(e);
        }
    };
    guard.status = Some(resp.status().as_u16());
    let mut out_headers = if plan.passthrough {
        headers::response(resp.headers())
    } else {
        HeaderMap::new()
    };

    if stream {
        // First-token timing is needed for every stream; the collector only
        // when capture is on.
        let tap = if guard.capture.is_some() {
            CaptureTap::new()
        } else {
            CaptureTap::timing_only()
        };
        let mut translator =
            stream_translator_with_usage(inbound, plan.upstream, plan.include_usage);
        translator.set_tap(tap.tap());
        guard.tap = Some(tap);
        let events = async_stream::stream! {
            // Owning `resp` here is what cancels the upstream request: when the
            // client goes away the body is dropped, and with it this generator.
            let mut up = Box::pin(resp.bytes_stream());
            let mut decoder = Decoder::default();
            while let Some(chunk) = up.next().await {
                let bytes = match chunk {
                    Ok(b) => b,
                    Err(e) => {
                        guard.error = Some(truncate(e.to_string(), ERROR_LIMIT));
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
                            guard.fail(&e);
                            guard.finish(translator.usage());
                            yield Err(io::Error::other(e.message()));
                            return;
                        }
                    }
                }
                guard.usage = translator.usage();
                if guard.first_token_at.is_none()
                    && let Some(at) = guard.tap.as_ref().and_then(CaptureTap::first_token_at)
                {
                    guard.first_token_at = Some(at);
                    guard.live.first_token(&guard.step_id, at);
                }
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
    let bytes = match resp.bytes().await.map_err(unreachable_error) {
        Ok(bytes) => bytes,
        Err(e) => {
            guard.fail(&e);
            return Err(e);
        }
    };
    let raw_for_capture = guard.capture.is_some().then(|| bytes.clone());
    let (upstream_protocol, passthrough) = (plan.upstream, plan.passthrough);
    let (out, usage, parsed) = match offload(bytes.len(), move || {
        let parsed = serde_json::from_slice::<Value>(&bytes);
        if passthrough {
            // Usage is best effort; the bytes go back exactly as received.
            let (usage, parsed) = parsed
                .ok()
                .and_then(|v| translate_response_parsed(inbound, upstream_protocol, v).ok())
                .map(|(_, usage, parsed)| (usage, parsed))
                .unwrap_or_default();
            return Ok((bytes, usage, parsed));
        }
        let value = parsed.map_err(|e| CoreError::Internal(format!("无法解析上游响应：{e}")))?;
        let (translated, usage, parsed) =
            translate_response_parsed(inbound, upstream_protocol, value)?;
        Ok((Bytes::from(serde_json::to_vec(&translated)?), usage, parsed))
    })
    .await
    {
        Ok(done) => done,
        Err(e) => {
            guard.fail(&e);
            return Err(e);
        }
    };
    guard.finish(usage);
    match parsed {
        Some(parsed) => guard.response = Some(parsed),
        None => guard.response_body = raw_for_capture,
    }
    out_headers
        .entry(header::CONTENT_TYPE)
        .or_insert(HeaderValue::from_static("application/json"));
    let mut response = Response::new(Body::from(out));
    *response.status_mut() = status;
    *response.headers_mut() = out_headers;
    Ok(response)
}

#[cfg(test)]
mod probe_gate_tests {
    use super::*;

    #[test]
    fn disabled_probe_does_not_consume_key() {
        let seen = Mutex::new(HashSet::new());
        assert!(!claim_probe(&seen, "k".into(), false));
        // Enabling later must still see the key as fresh.
        assert!(claim_probe(&seen, "k".into(), true));
    }

    #[test]
    fn enabled_probe_fires_once_per_key() {
        let seen = Mutex::new(HashSet::new());
        assert!(claim_probe(&seen, "k".into(), true));
        assert!(!claim_probe(&seen, "k".into(), true));
        assert!(claim_probe(&seen, "other".into(), true));
    }
}
