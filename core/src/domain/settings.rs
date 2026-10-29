use crate::domain::client::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct Settings {
    pub host: String,
    pub port: u16,
    pub log_level: LogLevel,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 11435,
            log_level: LogLevel::Info,
        }
    }
}

/// Session trajectory recording settings (stored on the settings row).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct TrajectorySettings {
    pub enabled: bool,
    pub disabled_clients: Vec<Client>,
    /// Whether requests on the global route (`api`, no terminal client) are recorded.
    pub disabled_global: bool,
    pub retention_days: u32,
    #[ts(type = "number")]
    pub max_bytes: u64,
    pub notice_seen: bool,
}

impl Default for TrajectorySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            disabled_clients: Vec::new(),
            disabled_global: false,
            retention_days: 30,
            max_bytes: 2_147_483_648,
            notice_seen: false,
        }
    }
}
