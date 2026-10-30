use crate::domain::protocol::Protocol;
use crate::error::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum Modality {
    Text,
    Image,
    Audio,
    Video,
    Pdf,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/generated/")]
pub enum Support {
    #[default]
    Unknown,
    Unsupported,
    Supported,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(default, deny_unknown_fields)]
#[ts(export, export_to = "../../src/generated/")]
pub struct Effort {
    pub support: Support,
    pub levels: Vec<String>,
    pub default: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(default, deny_unknown_fields)]
#[ts(export, export_to = "../../src/generated/")]
pub struct Capabilities {
    pub input_modalities: Option<Vec<Modality>>,
    pub output_modalities: Option<Vec<Modality>>,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub effort: Effort,
}

/// Effort levels each wire protocol can express.
pub fn effort_levels(protocol: Protocol) -> &'static [&'static str] {
    match protocol {
        Protocol::Anthropic => &["low", "medium", "high", "xhigh", "max", "ultracode"],
        Protocol::Responses | Protocol::OpenaiChat => {
            &["low", "medium", "high", "xhigh", "max", "ultra"]
        }
    }
}

fn has_duplicates<T: std::hash::Hash + Eq>(values: &[T]) -> bool {
    values.iter().collect::<HashSet<_>>().len() != values.len()
}

impl Capabilities {
    pub fn validate(&self, protocol: Protocol) -> CoreResult<()> {
        let invalid = |m: String| Err(CoreError::Validation(m));
        for (name, values) in [
            ("输入模态", &self.input_modalities),
            ("输出模态", &self.output_modalities),
        ] {
            if let Some(values) = values
                && (values.is_empty() || has_duplicates(values))
            {
                return invalid(format!("{name}必须至少选择一项且不能重复"));
            }
        }
        for (name, value) in [
            ("上下文容量", self.context_window),
            ("最大输出", self.max_output_tokens),
        ] {
            if value.is_some_and(|v| v == 0 || v > 100_000_000) {
                return invalid(format!("{name}必须是 1 到 100000000 之间的整数 Token 数"));
            }
        }
        if let (Some(context), Some(output)) = (self.context_window, self.max_output_tokens)
            && output > context
        {
            return invalid("最大输出不能超过上下文容量".into());
        }
        let allowed = effort_levels(protocol);
        if self.effort.support == Support::Supported {
            if self.effort.levels.is_empty()
                || has_duplicates(&self.effort.levels)
                || self
                    .effort
                    .levels
                    .iter()
                    .any(|level| !allowed.contains(&level.as_str()))
            {
                return invalid("Effort 档位为空、重复或不符合上游协议".into());
            }
            if self
                .effort
                .default
                .as_ref()
                .is_some_and(|d| !self.effort.levels.contains(d))
            {
                return invalid("默认 Effort 必须在支持的档位中".into());
            }
        } else if !self.effort.levels.is_empty() || self.effort.default.is_some() {
            return invalid("只有支持 Effort 的模型可以配置档位和默认值".into());
        }
        Ok(())
    }

    /// Effort levels usable when a `client`-protocol request is served by an `upstream` model.
    pub fn efforts_for(&self, client: Protocol, upstream: Protocol) -> Vec<String> {
        if self.effort.support != Support::Supported {
            return Vec::new();
        }
        self.effort
            .levels
            .iter()
            .filter(|level| {
                effort_levels(client).contains(&level.as_str())
                    && effort_levels(upstream).contains(&level.as_str())
            })
            .cloned()
            .collect()
    }

    /// Whether the model is known to expose reasoning effort control.
    pub fn supports_reasoning(&self) -> bool {
        self.effort.support == Support::Supported && !self.effort.levels.is_empty()
    }
}
