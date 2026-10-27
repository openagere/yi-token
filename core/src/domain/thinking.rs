use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum Thinking {
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
}
