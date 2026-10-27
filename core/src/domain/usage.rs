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
