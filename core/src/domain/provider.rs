use crate::domain::protocol::Protocol;
use crate::domain::thinking::Thinking;
use crate::error::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct Provider {
    pub id: String,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub provider_type: Protocol,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub enabled: bool,
    pub thinking: Option<Thinking>,
    pub is_default: bool,
    #[ts(type = "number")]
    pub sort_order: i64,
    #[ts(type = "Record<string, unknown>")]
    pub extra: serde_json::Value,
    /// Protocols the provider serves; always includes `provider_type`.
    pub protocols: Vec<Protocol>,
}

impl Provider {
    pub fn validate(&self) -> CoreResult<()> {
        if super::provider_slug(&self.name).is_empty() {
            return Err(CoreError::Validation("请填写 Provider 名称".into()));
        }
        if !(self.base_url.starts_with("http://") || self.base_url.starts_with("https://")) {
            return Err(CoreError::Validation(
                "Base URL 必须以 http:// 或 https:// 开头".into(),
            ));
        }
        if !self.protocols.contains(&self.provider_type) {
            return Err(CoreError::Validation(
                "支持的协议必须包含服务商的原生协议".into(),
            ));
        }
        Ok(())
    }
}
