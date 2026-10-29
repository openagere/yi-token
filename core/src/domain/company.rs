//! Model vendors that own standard model definitions.

use crate::domain::protocol::Protocol;
use crate::error::CoreError;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Vendor of a standard model. Chat Completions and Responses both use OpenAI's
/// catalog; Anthropic Messages uses Anthropic's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/generated/")]
pub enum Company {
    Openai,
    Anthropic,
}

impl Company {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
        }
    }

    pub fn all() -> [Company; 2] {
        [Self::Openai, Self::Anthropic]
    }

    /// The company whose standard models a wire protocol uses.
    pub fn of(protocol: Protocol) -> Self {
        match protocol {
            Protocol::Anthropic => Self::Anthropic,
            Protocol::OpenaiChat | Protocol::Responses => Self::Openai,
        }
    }

    /// A representative protocol, for rules that are shared by the whole company
    /// (such as the effort vocabulary of its standard models).
    pub fn protocol(self) -> Protocol {
        match self {
            Self::Openai => Protocol::Responses,
            Self::Anthropic => Protocol::Anthropic,
        }
    }
}

impl FromStr for Company {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::all()
            .into_iter()
            .find(|c| c.as_str() == s)
            .ok_or_else(|| CoreError::Validation("标准模型公司无效".into()))
    }
}
