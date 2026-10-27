use crate::domain::protocol::Protocol;
use crate::error::CoreError;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// A terminal client managed by yi-token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub enum Client {
    #[serde(rename = "codex")]
    #[ts(rename = "codex")]
    Codex,
    #[serde(rename = "claude-code")]
    #[ts(rename = "claude-code")]
    ClaudeCode,
    #[serde(rename = "opencode")]
    #[ts(rename = "opencode")]
    Opencode,
    #[serde(rename = "pi")]
    #[ts(rename = "pi")]
    Pi,
    #[serde(rename = "deepseek-harness")]
    #[ts(rename = "deepseek-harness")]
    DeepseekHarness,
}

const ALL_PROTOCOLS: &[Protocol] = &[
    Protocol::OpenaiChat,
    Protocol::Responses,
    Protocol::Anthropic,
];

impl Client {
    pub const ALL: [Client; 5] = [
        Self::Codex,
        Self::ClaudeCode,
        Self::Opencode,
        Self::Pi,
        Self::DeepseekHarness,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude-code",
            Self::Opencode => "opencode",
            Self::Pi => "pi",
            Self::DeepseekHarness => "deepseek-harness",
        }
    }

    /// Protocols this client can speak to the proxy; the first is the primary one.
    pub fn protocols(&self) -> &'static [Protocol] {
        match self {
            Self::Codex => &[Protocol::Responses],
            Self::ClaudeCode => &[Protocol::Anthropic],
            _ => ALL_PROTOCOLS,
        }
    }

    pub fn default_protocol(&self) -> Protocol {
        self.protocols()[0]
    }
}

impl FromStr for Client {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|c| c.as_str() == s)
            .ok_or_else(|| CoreError::Validation("未知终端类型".into()))
    }
}
