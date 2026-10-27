use crate::domain::capabilities::Capabilities;
use crate::domain::protocol::Protocol;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct StandardModel {
    pub id: String,
    pub name: String,
    pub protocol: Protocol,
    pub brand: String,
    pub capabilities: Capabilities,
}
