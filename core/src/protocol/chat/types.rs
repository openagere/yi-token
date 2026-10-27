//! Typed Chat Completions wire types (deserialization side).
//! Unknown fields are kept in flattened `extra` maps; unknown content parts
//! and tools are kept verbatim as raw JSON values.

use crate::protocol::Usage;
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Clone, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
    #[serde(default, deserialize_with = "crate::protocol::ir::null_default")]
    pub tools: Vec<Value>,
    #[serde(default)]
    pub tool_choice: Option<Value>,
    #[serde(default)]
    pub max_completion_tokens: Option<u64>,
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    pub stop: Option<Value>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ChatContent {
    Text(String),
    /// Parts stay raw so unknown part types survive.
    Parts(Vec<Value>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default)]
    pub content: Option<ChatContent>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl ChatMessage {
    /// `reasoning_content`, or the `reasoning` spelling some providers use.
    pub fn reasoning_text(&self) -> Option<String> {
        self.reasoning_content
            .clone()
            .or_else(|| {
                self.extra
                    .get("reasoning")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .filter(|s| !s.is_empty())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub function: ToolCallFunction,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    #[serde(default)]
    pub arguments: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatResponse {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub created: Option<i64>,
    #[serde(default)]
    pub choices: Vec<Choice>,
    #[serde(default)]
    pub usage: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Choice {
    pub message: ChatMessage,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

// ---------------------------------------------------------------- streaming

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Chunk {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub choices: Vec<ChunkChoice>,
    #[serde(default)]
    pub usage: Option<Value>,
    #[serde(default)]
    pub error: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChunkChoice {
    #[serde(default)]
    pub index: u64,
    #[serde(default)]
    pub delta: Delta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Delta {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolCallDelta {
    #[serde(default)]
    pub index: u64,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<FunctionDelta>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FunctionDelta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

/// Chat `usage` object -> normalised [`Usage`].
pub fn parse_usage(u: &Value) -> Usage {
    let n = |ptr: &str| u.pointer(ptr).and_then(Value::as_u64);
    Usage {
        input_tokens: n("/prompt_tokens").unwrap_or(0),
        output_tokens: n("/completion_tokens").unwrap_or(0),
        cached_tokens: n("/prompt_tokens_details/cached_tokens")
            .or_else(|| n("/prompt_cache_hit_tokens"))
            .unwrap_or(0),
        reasoning_tokens: n("/completion_tokens_details/reasoning_tokens").unwrap_or(0),
    }
}
