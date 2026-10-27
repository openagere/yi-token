use crate::domain::capabilities::Capabilities;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct Model {
    pub id: String,
    pub provider_id: String,
    pub standard_model_id: Option<String>,
    pub upstream_model: String,
    pub exposed_name: String,
    pub capabilities: Capabilities,
    /// The upstream implements only a subset of its declared wire protocol.
    pub non_standard: bool,
}
