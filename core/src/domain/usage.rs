use crate::domain::client::Client;
use crate::domain::protocol::Protocol;
use crate::domain::terminal::RouteMode;
use serde::{Deserialize, Serialize};

/// One finished request waiting to be persisted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct UsageEvent {
    /// UTC milliseconds.
    #[ts(type = "number")]
    pub requested_at: i64,
    pub client: Option<Client>,
    pub route_mode: Option<RouteMode>,
    pub provider_name: String,
    pub protocol: Protocol,
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cached_tokens: u32,
    pub reasoning_tokens: u32,
    /// Trajectory step this request produced, when trajectory recording is on.
    pub step_id: Option<String>,
    /// Trajectory session this request belongs to.
    pub session_id: Option<String>,
    /// Milliseconds from request start to the first upstream token.
    pub ttft_ms: Option<u32>,
    /// Milliseconds from request start to completion.
    pub duration_ms: Option<u32>,
    /// Upstream HTTP status code.
    pub status: Option<u16>,
}

/// One persisted request, as listed in the usage report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct UsageRecord {
    pub id: String,
    #[ts(type = "number")]
    pub requested_at: i64,
    pub client: Option<Client>,
    pub route_mode: Option<RouteMode>,
    pub provider_name: String,
    pub protocol: Protocol,
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cached_tokens: u32,
    pub reasoning_tokens: u32,
    pub step_id: Option<String>,
    pub session_id: Option<String>,
    pub ttft_ms: Option<u32>,
    pub duration_ms: Option<u32>,
    pub status: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct UsageDay {
    pub day: String,
    #[ts(type = "number")]
    pub input_tokens: u64,
    #[ts(type = "number")]
    pub output_tokens: u64,
    #[ts(type = "number")]
    pub cached_tokens: u64,
    #[ts(type = "number")]
    pub reasoning_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct UsageDailyModel {
    pub day: String,
    pub model: String,
    #[ts(type = "number")]
    pub tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct UsageModel {
    pub model: String,
    #[ts(type = "number")]
    pub requests: u64,
    #[ts(type = "number")]
    pub input_tokens: u64,
    #[ts(type = "number")]
    pub output_tokens: u64,
    #[ts(type = "number")]
    pub cached_tokens: u64,
    #[ts(type = "number")]
    pub reasoning_tokens: u64,
}

/// Aggregate per terminal client; `client` is `None` for global-route requests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct UsageClient {
    pub client: Option<Client>,
    #[ts(type = "number")]
    pub requests: u64,
    #[ts(type = "number")]
    pub input_tokens: u64,
    #[ts(type = "number")]
    pub output_tokens: u64,
    #[ts(type = "number")]
    pub cached_tokens: u64,
    #[ts(type = "number")]
    pub reasoning_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct UsageReport {
    #[ts(type = "number")]
    pub total_requests: u64,
    #[ts(type = "number")]
    pub input_tokens: u64,
    #[ts(type = "number")]
    pub output_tokens: u64,
    #[ts(type = "number")]
    pub cached_tokens: u64,
    #[ts(type = "number")]
    pub reasoning_tokens: u64,
    pub days: Vec<UsageDay>,
    pub daily_models: Vec<UsageDailyModel>,
    pub models: Vec<UsageModel>,
    pub by_client: Vec<UsageClient>,
    /// Most recent requests first, capped at 100.
    pub recent: Vec<UsageRecord>,
}

/// Dimension the report's `groups` are aggregated by. Defaults to `Model`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/generated/")]
pub enum UsageGroupBy {
    #[default]
    Model,
    Client,
    Provider,
}

/// Totals over the whole filtered window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct UsageTotals {
    #[ts(type = "number")]
    pub requests: u64,
    #[ts(type = "number")]
    pub input_tokens: u64,
    #[ts(type = "number")]
    pub output_tokens: u64,
    #[ts(type = "number")]
    pub cached_tokens: u64,
    #[ts(type = "number")]
    pub reasoning_tokens: u64,
    /// `cached_tokens / input_tokens`; `None` when there are no input tokens.
    pub cache_hit_rate: Option<f64>,
    /// Mean time to first token over rows that recorded it; `None` if none did.
    pub avg_ttft_ms: Option<f64>,
    /// Mean request duration over rows that recorded it; `None` if none did.
    pub avg_duration_ms: Option<f64>,
    /// Rows with any timing recorded (the rows that feed the averages).
    #[ts(type = "number")]
    pub timed_requests: u64,
}

/// One bucket of the report, keyed by the chosen `UsageGroupBy` dimension.
/// For `Client`, an empty key means the global route (no terminal client).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct UsageGroupRow {
    pub key: String,
    pub label: String,
    #[ts(type = "number")]
    pub requests: u64,
    #[ts(type = "number")]
    pub input_tokens: u64,
    #[ts(type = "number")]
    pub output_tokens: u64,
    #[ts(type = "number")]
    pub cached_tokens: u64,
    pub cache_hit_rate: Option<f64>,
    /// Mean duration over rows in this group that recorded it.
    pub avg_duration_ms: Option<f64>,
}
