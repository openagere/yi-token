//! Frontend-visible view types. These are the only wire shapes the frontend reads and writes;
//! the Rust structs are the source of the generated TypeScript types.

use crate::domain::{
    Capabilities, Client, Company, DirectProfile, LogLevel, ProfileModel, Protocol, RouteMode,
    StandardModel, Thinking, UsageGroupBy, UsageGroupRow, UsageReport, UsageTotals,
};
use crate::proxy::ProxyPhase;
use serde::{Deserialize, Serialize};

pub use crate::catalog::CatalogInfo;

fn default_thinking() -> Thinking {
    Thinking::Medium
}

fn empty_object() -> serde_json::Value {
    serde_json::json!({})
}

/// The provider fields exchanged with the frontend (input of `save_provider`,
/// and the flattened head of `ProviderView`). An empty `id` means "new".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct ProviderInput {
    #[serde(default)]
    pub id: String,
    pub provider_type: Protocol,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_thinking")]
    pub thinking: Thinking,
    #[serde(default = "empty_object")]
    #[ts(type = "Record<string, unknown>")]
    pub extra: serde_json::Value,
    #[serde(default)]
    pub is_default: bool,
}

/// A provider model as the frontend sees it: `route_id` is the exposed name,
/// `name` the upstream model. An empty `id` means "new".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct ModelMapping {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub route_id: String,
    pub name: String,
    #[serde(default)]
    pub standard_model_id: Option<String>,
    /// Upstream protocol override; `null` follows the provider's type.
    #[serde(default)]
    pub protocol: Option<Protocol>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub capabilities: Option<Capabilities>,
    #[serde(default)]
    pub non_standard: bool,
}

/// Input form of a model mapping in `save_provider`.
pub type ModelInput = ModelMapping;

/// Outcome of `test_provider`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct TestResult {
    pub ok: bool,
    #[ts(type = "number")]
    pub latency_ms: u64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct ProviderView {
    #[serde(flatten)]
    #[ts(flatten)]
    pub provider: ProviderInput,
    pub models: Vec<ModelMapping>,
}

/// A standard model plus the number of provider models that use it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct StandardModelView {
    pub id: String,
    pub name: String,
    pub company: Company,
    #[serde(default)]
    pub brand: String,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default)]
    pub provider_count: u32,
}

impl StandardModelView {
    pub fn new(model: StandardModel, provider_count: u32) -> Self {
        Self {
            id: model.id,
            name: model.name,
            company: model.company,
            brand: model.brand,
            capabilities: model.capabilities,
            provider_count,
        }
    }

    pub fn into_model(self) -> StandardModel {
        StandardModel {
            id: self.id,
            name: self.name,
            company: self.company,
            brand: self.brand,
            capabilities: self.capabilities,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct SettingsView {
    pub host: String,
    pub port: u16,
    pub log_level: LogLevel,
}

fn fixed_route_mode() -> RouteMode {
    RouteMode::Fixed
}

/// The current Auto engine of one terminal, resolved for display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct ActiveEngineView {
    pub client: Client,
    pub model_id: String,
    pub exposed_name: String,
    pub provider_name: String,
}

/// A terminal proxy profile as the frontend exchanges it. `protocol: null`
/// means the client's only protocol (and is always null in responses).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct TerminalProfileView {
    pub client: Client,
    pub models: Vec<ProfileModel>,
    #[serde(default)]
    pub default_model: String,
    #[serde(default)]
    pub protocol: Option<Protocol>,
    #[serde(default = "fixed_route_mode")]
    pub route_mode: RouteMode,
    #[serde(default)]
    pub active_engine: Option<ActiveEngineView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum TerminalAccessMode {
    None,
    Proxy,
    Direct,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct TerminalStatusView {
    pub client: Client,
    pub config_path: String,
    pub exists: bool,
    pub active: bool,
    pub active_mode: TerminalAccessMode,
    pub profile: TerminalProfileView,
    pub direct_profile: Option<DirectProfile>,
    pub config_error: Option<String>,
}

/// One status per managed client, in `Client::ALL` order.
pub type TerminalProfilesView = Vec<TerminalStatusView>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct TerminalApplyResult {
    pub config_path: String,
    pub endpoint: String,
    pub backup_paths: Vec<String>,
    #[ts(type = "number")]
    pub model_count: usize,
}

/// Live listener status plus the saved listen settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct ProxyRuntimeView {
    pub phase: ProxyPhase,
    pub desired_running: bool,
    /// Unix milliseconds.
    #[ts(type = "number | null")]
    pub started_at: Option<u64>,
    pub address: Option<String>,
    pub last_error: Option<String>,
    #[ts(type = "number")]
    pub active_requests: u64,
    #[ts(type = "number")]
    pub active_connections: u64,
    pub settings: SettingsView,
}

/// Usage report: the stored aggregate (flattened, unchanged fields) plus the
/// window `totals` and the `groups` for the requested `group_by`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct UsageReportView {
    #[serde(flatten)]
    #[ts(flatten)]
    pub report: UsageReport,
    pub totals: UsageTotals,
    pub groups: Vec<UsageGroupRow>,
}

/// Filter of `get_usage`. Dates are local `YYYY-MM-DD`; both or neither.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct UsageQueryInput {
    pub days: Option<u32>,
    pub model: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub client: Option<Client>,
    /// Defaults to `Model` when absent.
    pub group_by: Option<UsageGroupBy>,
}

/// Tail of the log file, oldest line first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct LogsView {
    pub lines: Vec<String>,
}
