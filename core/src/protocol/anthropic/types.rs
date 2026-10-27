//! Typed Anthropic Messages wire types (deserialization side).
//! Unknown fields are kept in flattened `extra` maps; unknown content blocks
//! are kept verbatim as [`Block::Unknown`] (never dropped silently).

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Deserialize)]
pub struct MessagesRequest {
    pub model: String,
    #[serde(default)]
    pub system: Option<System>,
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default, deserialize_with = "crate::protocol::ir::null_default")]
    pub tools: Vec<Value>,
    #[serde(default)]
    pub tool_choice: Option<Value>,
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    pub thinking: Option<Value>,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum System {
    Text(String),
    Blocks(Vec<Block>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: MessageContent,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<Block>),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum KnownBlock {
    #[serde(rename = "text")]
    Text {
        text: String,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    #[serde(rename = "image")]
    Image {
        source: Value,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: Value,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        #[serde(default)]
        content: Option<Value>,
        #[serde(default)]
        is_error: Option<bool>,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    #[serde(rename = "thinking")]
    Thinking {
        #[serde(default)]
        thinking: String,
        #[serde(default)]
        signature: Option<String>,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    #[serde(rename = "redacted_thinking")]
    RedactedThinking {
        data: String,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
}

#[derive(Debug, Clone)]
pub enum Block {
    Known(KnownBlock),
    Unknown(Value),
}

impl<'de> Deserialize<'de> for Block {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        Ok(match serde_json::from_value::<KnownBlock>(v.clone()) {
            Ok(k) => Block::Known(k),
            Err(e) => {
                const KNOWN: [&str; 6] = [
                    "text",
                    "image",
                    "tool_use",
                    "tool_result",
                    "thinking",
                    "redacted_thinking",
                ];
                let ty = v.get("type").and_then(Value::as_str).unwrap_or("?");
                if KNOWN.contains(&ty) {
                    tracing::warn!(block_type = ty, error = %e,
                        "malformed anthropic content block kept as unknown");
                }
                Block::Unknown(v)
            }
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MessagesResponse {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub content: Vec<Block>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub usage: Option<Value>,
}

pub fn from_value<T: DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| e.to_string())
}
