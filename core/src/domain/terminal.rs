use crate::domain::client::Client;
use crate::domain::protocol::Protocol;
use crate::error::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};

/// Model name a client sends to let the proxy pick the engine (spec §7.1).
pub const AUTO_MODEL: &str = "yi-token";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum RouteMode {
    Fixed,
    Auto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct ProfileModel {
    pub provider_id: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct ProxyProfile {
    pub client: Client,
    pub protocol: Protocol,
    pub route_mode: RouteMode,
    pub models: Vec<ProfileModel>,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ModelSource {
    Native,
    Provider,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct DirectProfile {
    pub client: Client,
    pub provider_id: String,
    pub model_source: ModelSource,
    pub model: Option<String>,
}

impl ProxyProfile {
    pub fn validate(&self) -> CoreResult<()> {
        if !self.client.protocols().contains(&self.protocol) {
            return Err(CoreError::Validation("该终端不支持所选协议".into()));
        }
        if self.route_mode == RouteMode::Fixed {
            let default = self.default_model.as_deref().unwrap_or(AUTO_MODEL);
            let default_ok =
                default == AUTO_MODEL || self.models.iter().any(|m| m.model == default);
            if !default_ok {
                return Err(CoreError::Validation("默认模型必须在已选模型中".into()));
            }
        }
        Ok(())
    }
}
