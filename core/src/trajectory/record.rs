//! The per-request record handed from the proxy pipeline to the capture sink.

use std::sync::Arc;

use arc_swap::ArcSwapOption;
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::domain::{Client, Protocol, RouteMode};
use crate::protocol::Usage;
use crate::protocol::ir;

/// Why a step ended (spec 4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    End,
    ToolUse,
    MaxTokens,
    Refusal,
    Error,
    Cancelled,
    Unknown,
}

/// How much of a step is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureLevel {
    Full,
    MetadataOnly,
}

/// Client-supplied keys, before any prefix-chain matching.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientKeys {
    /// `<client>:<session>`, or `None` when the client sent no session identifier.
    pub session_key: Option<String>,
    pub turn_id: Option<String>,
}

/// One request's capture, submitted once when the request ends.
#[derive(Debug, Clone)]
pub struct CaptureRecord {
    /// UUID, equal to `usage.step_id`.
    pub step_id: String,
    pub client: Option<Client>,
    pub keys: ClientKeys,
    pub inbound: Protocol,
    pub upstream: Protocol,
    pub provider_name: String,
    /// Exposed model name, equal to `usage.model`.
    pub model: String,
    pub upstream_model: String,
    pub route_mode: Option<RouteMode>,
    pub requested_at: i64,
    pub first_token_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub status: Option<u16>,
    pub stop_reason: StopReason,
    /// Truncated to 4 KiB.
    pub error: Option<String>,
    pub usage: Usage,
    /// The client's request body as received.
    pub request_body: Option<Bytes>,
    /// Collector result for streams, or the parsed body for non-streams.
    pub response: Option<ir::Response>,
    /// Raw upstream response when it was not parsed.
    pub response_body: Option<Bytes>,
}

/// Receives capture records. `submit` must not block the request path.
pub trait CaptureSink: Send + Sync {
    fn enabled_for(&self, client: Option<Client>) -> bool;
    fn submit(&self, record: CaptureRecord);
}

/// Wraps the trait object so it can live inside an `ArcSwap`, which needs a sized type.
pub struct CaptureHandle(pub Arc<dyn CaptureSink>);

/// A capture sink that can be replaced while the proxy runs.
#[derive(Clone, Default)]
pub struct CaptureSlot(Arc<ArcSwapOption<CaptureHandle>>);

impl CaptureSlot {
    pub fn set(&self, sink: Option<Arc<dyn CaptureSink>>) {
        self.0.store(sink.map(|s| Arc::new(CaptureHandle(s))));
    }

    pub fn get(&self) -> Option<Arc<dyn CaptureSink>> {
        self.0.load_full().map(|h| h.0.clone())
    }
}
