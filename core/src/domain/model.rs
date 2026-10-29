use crate::domain::capabilities::Capabilities;
use crate::domain::protocol::Protocol;
use crate::domain::provider::Provider;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct Model {
    pub id: String,
    pub provider_id: String,
    pub standard_model_id: Option<String>,
    pub upstream_model: String,
    pub exposed_name: String,
    /// Upstream wire protocol when it differs from the provider's type; `None` follows the provider.
    pub protocol: Option<Protocol>,
    pub capabilities: Capabilities,
    /// The upstream implements only a subset of its declared wire protocol.
    pub non_standard: bool,
}

impl Model {
    /// The wire protocol this model is called with upstream.
    pub fn upstream_protocol(&self, provider: &Provider) -> Protocol {
        self.protocol.unwrap_or(provider.provider_type)
    }
}
