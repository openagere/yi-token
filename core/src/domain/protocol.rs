use crate::error::CoreError;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Wire protocol spoken by a provider or a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/generated/")]
pub enum Protocol {
    Anthropic,
    OpenaiChat,
    Responses,
}

impl Protocol {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenaiChat => "openai_chat",
            Self::Responses => "responses",
        }
    }

    pub fn all() -> [Protocol; 3] {
        [Self::Anthropic, Self::OpenaiChat, Self::Responses]
    }
}

impl FromStr for Protocol {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::all()
            .into_iter()
            .find(|p| p.as_str() == s)
            .ok_or_else(|| CoreError::Validation("模型协议无效".into()))
    }
}
