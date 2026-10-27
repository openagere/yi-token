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
