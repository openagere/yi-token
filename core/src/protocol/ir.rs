//! Typed Responses-API representation. Unknown fields are preserved through
//! flattened `extra` maps; unknown item/part/tool/event types are kept verbatim.

use super::sse::SseFrame;
use super::usage::Usage;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

fn typed<T: DeserializeOwned>(v: &Value) -> Option<T> {
    serde_json::from_value(v.clone()).ok()
}

fn type_of(v: &Value) -> Option<&str> {
    v.get("type").and_then(Value::as_str)
}

// ---------------------------------------------------------------- content

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ContentPart {
    InputText(TextPart),
    OutputText(TextPart),
    InputImage(ImagePart),
    Unknown(Value),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextPart {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImagePart {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl<'de> Deserialize<'de> for ContentPart {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        let known = match type_of(&v) {
            Some("input_text") => typed(&v).map(ContentPart::InputText),
            Some("output_text") => typed(&v).map(ContentPart::OutputText),
            Some("input_image") => typed(&v).map(ContentPart::InputImage),
            _ => None,
        };
        Ok(known.unwrap_or(ContentPart::Unknown(v)))
    }
}

/// Message content: a plain string or a list of parts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Parts(Vec<ContentPart>),
}

// ------------------------------------------------------------------ items

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum InputItem {
    Message(Message),
    FunctionCall(FunctionCall),
    FunctionCallOutput(FunctionCallOutput),
    Reasoning(Reasoning),
    Unknown(Value),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// Absent on "easy" messages (`{"role":..,"content":..}`).
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub role: String,
    pub content: Content,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionCall {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionCallOutput {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reasoning {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<Vec<Value>>,
    /// Opaque upstream signature; must be passed through unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_content: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl<'de> Deserialize<'de> for InputItem {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        let known = match type_of(&v) {
            Some("message") => typed(&v).map(InputItem::Message),
            None if v.get("role").is_some() => typed(&v).map(InputItem::Message),
            Some("function_call") => typed(&v).map(InputItem::FunctionCall),
            Some("function_call_output") => typed(&v).map(InputItem::FunctionCallOutput),
            Some("reasoning") => typed::<Reasoning>(&v).map(|mut r| {
                // Reserved origin tag: only the Anthropic inbound/stream may set it.
                r.extra.remove("anthropic_signature");
                InputItem::Reasoning(r)
            }),
            _ => None,
        };
        Ok(known.unwrap_or(InputItem::Unknown(v)))
    }
}

// ------------------------------------------------------------------ tools

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Tool {
    Function(FunctionTool),
    Unknown(Value),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionTool {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl<'de> Deserialize<'de> for Tool {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        let known = match type_of(&v) {
            Some("function") => typed(&v).map(Tool::Function),
            _ => None,
        };
        Ok(known.unwrap_or(Tool::Unknown(v)))
    }
}

// ---------------------------------------------------------------- request

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReasoningConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Clients commonly send `"field": null` for an absent optional list; treat it
/// like a missing field instead of rejecting the request.
pub(crate) fn null_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// `input` may be a bare string; it becomes a single user message.
fn input_items<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<InputItem>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Input {
        Text(String),
        Items(Vec<InputItem>),
    }
    Ok(match Input::deserialize(d)? {
        Input::Text(text) => vec![InputItem::Message(Message {
            kind: None,
            role: "user".into(),
            content: Content::Text(text),
            extra: Map::new(),
        })],
        Input::Items(items) => items,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub model: String,
    #[serde(default, deserialize_with = "input_items")]
    pub input: Vec<InputItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub tools: Vec<Tool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

// --------------------------------------------------------------- response

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Response {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default)]
    pub output: Vec<InputItem>,
    /// Kept as raw JSON so provider-specific usage details survive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Response {
    pub fn usage(&self) -> Usage {
        let Some(u) = &self.usage else {
            return Usage::default();
        };
        let n = |ptr: &str| u.pointer(ptr).and_then(Value::as_u64).unwrap_or(0);
        Usage {
            input_tokens: n("/input_tokens"),
            output_tokens: n("/output_tokens"),
            cached_tokens: n("/input_tokens_details/cached_tokens"),
            reasoning_tokens: n("/output_tokens_details/reasoning_tokens"),
        }
    }
}

// ----------------------------------------------------------------- stream

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StreamEvent {
    #[serde(rename = "response.created")]
    Created {
        response: Response,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    #[serde(rename = "response.output_item.added")]
    OutputItemAdded {
        output_index: u64,
        item: InputItem,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    #[serde(rename = "response.output_item.done")]
    OutputItemDone {
        output_index: u64,
        item: InputItem,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta {
        output_index: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content_index: Option<u64>,
        delta: String,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    #[serde(rename = "response.reasoning_summary_text.delta")]
    ReasoningSummaryTextDelta {
        output_index: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary_index: Option<u64>,
        delta: String,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    #[serde(rename = "response.function_call_arguments.delta")]
    FunctionCallArgumentsDelta {
        output_index: u64,
        delta: String,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    #[serde(rename = "response.completed")]
    Completed {
        response: Response,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    #[serde(rename = "response.incomplete")]
    Incomplete {
        response: Response,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    #[serde(rename = "response.failed")]
    Failed {
        response: Response,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    #[serde(rename = "error")]
    Error {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    /// Any event this crate does not model; kept verbatim.
    #[serde(skip)]
    Unknown { event: String, data: Value },
}

impl StreamEvent {
    /// Interpret an SSE frame. Returns `None` for the `[DONE]` sentinel.
    pub fn from_frame(frame: &SseFrame) -> Option<StreamEvent> {
        if frame.data.trim() == "[DONE]" {
            return None;
        }
        let name = frame.event.clone().unwrap_or_default();
        let Ok(data) = serde_json::from_str::<Value>(&frame.data) else {
            return Some(StreamEvent::Unknown {
                event: name,
                data: Value::String(frame.data.clone()),
            });
        };
        let mut tagged = data.clone();
        if type_of(&tagged).is_none()
            && !name.is_empty()
            && let Value::Object(m) = &mut tagged
        {
            m.insert("type".into(), Value::String(name.clone()));
        }
        Some(typed(&tagged).unwrap_or_else(|| StreamEvent::Unknown {
            event: type_of(&data).map(str::to_owned).unwrap_or(name),
            data,
        }))
    }

    /// Event name suitable for the SSE `event:` line.
    pub fn name(&self) -> String {
        match self {
            StreamEvent::Unknown { event, .. } => event.clone(),
            other => serde_json::to_value(other)
                .ok()
                .and_then(|v| type_of(&v).map(str::to_owned))
                .unwrap_or_default(),
        }
    }

    /// JSON payload for the SSE `data:` line.
    pub fn data(&self) -> Value {
        match self {
            StreamEvent::Unknown { data, .. } => data.clone(),
            other => serde_json::to_value(other).unwrap_or(Value::Null),
        }
    }
}

impl TextPart {
    /// A text part of the given `kind` with no vendor extension fields.
    pub fn new(kind: &str, text: String) -> Self {
        Self {
            kind: kind.into(),
            text,
            extra: Map::new(),
        }
    }
}

/// An assistant message with one `output_text` part, or no parts when `text` is empty.
pub(crate) fn assistant_message(text: &str) -> InputItem {
    InputItem::Message(Message {
        kind: Some("message".into()),
        role: "assistant".into(),
        content: Content::Parts(if text.is_empty() {
            vec![]
        } else {
            vec![ContentPart::OutputText(TextPart::new("output_text", text.into()))]
        }),
        extra: Map::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn roundtrip<T: Serialize + DeserializeOwned>(v: Value) -> T {
        let t: T = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(serde_json::to_value(&t).unwrap(), v);
        t
    }

    #[test]
    fn unknown_input_item_roundtrips() {
        let v = json!({"type":"web_search_call","id":"ws_1","action":{"q":"x"}});
        let item: InputItem = roundtrip(v);
        assert!(matches!(item, InputItem::Unknown(_)));
    }

    #[test]
    fn null_tools_are_treated_as_absent() {
        let r: Request =
            serde_json::from_value(json!({"model": "m", "input": "hi", "tools": null})).unwrap();
        assert!(r.tools.is_empty());
        let c = crate::protocol::chat::inbound::to_ir(
            json!({"model": "m", "messages": [{"role": "user", "content": "hi"}], "tools": null}),
        )
        .unwrap();
        assert!(c.tools.is_empty());
        let a = crate::protocol::anthropic::inbound::to_ir(
            json!({"model": "m", "max_tokens": 5, "messages": [{"role": "user", "content": "hi"}], "tools": null}),
        )
        .unwrap();
        assert!(a.tools.is_empty());
    }

    #[test]
    fn extra_fields_are_preserved() {
        let v = json!({
            "model": "gpt-5",
            "input": [
                {"role": "user", "content": [
                    {"type": "input_text", "text": "hi", "cache_control": {"a": 1}},
                    {"type": "input_audio", "data": "zzz"}
                ], "phase": "x"},
                {"type": "function_call", "call_id": "c1", "name": "f", "arguments": "{}", "id": "fc_1"},
                {"type": "function_call_output", "call_id": "c1", "output": "ok"}
            ],
            "tools": [
                {"type": "function", "name": "f", "parameters": {"type": "object"}, "strict": true, "x": 1},
                {"type": "web_search", "filters": {}}
            ],
            "reasoning": {"effort": "high", "summary": "auto", "generate_summary": "x"},
            "stream": true,
            "service_tier": "flex",
            "parallel_tool_calls": false
        });
        let req: Request = roundtrip(v);
        assert_eq!(req.extra["service_tier"], "flex");
        assert!(matches!(req.tools[1], Tool::Unknown(_)));
        let InputItem::Message(m) = &req.input[0] else {
            panic!("message")
        };
        assert_eq!(m.extra["phase"], "x");
        let Content::Parts(parts) = &m.content else {
            panic!("parts")
        };
        assert!(matches!(parts[1], ContentPart::Unknown(_)));
    }

    #[test]
    fn string_input_becomes_user_message() {
        let req: Request = serde_json::from_value(json!({"model":"m","input":"hello"})).unwrap();
        assert_eq!(req.input.len(), 1);
        assert!(
            matches!(&req.input[0], InputItem::Message(m) if m.role == "user" && m.content == Content::Text("hello".into()))
        );
    }

    #[test]
    fn empty_response_output_is_serialized() {
        let v = serde_json::to_value(Response::default()).unwrap();
        assert_eq!(v["output"], json!([]));
    }

    #[test]
    fn encrypted_content_survives_roundtrip() {
        let v = json!({
            "type": "reasoning", "id": "rs_1", "summary": [{"type":"summary_text","text":"t"}],
            "encrypted_content": "gAAAA==", "signature": "sig"
        });
        let item: InputItem = roundtrip(v);
        let InputItem::Reasoning(r) = item else {
            panic!("reasoning")
        };
        assert_eq!(r.encrypted_content.as_deref(), Some("gAAAA=="));
        assert_eq!(r.extra["signature"], "sig");
    }

    #[test]
    fn stream_event_unknown_and_known() {
        let f = |event: Option<&str>, data: &str| SseFrame {
            event: event.map(str::to_owned),
            data: data.to_owned(),
        };
        let ev = StreamEvent::from_frame(&f(None, r#"{"type":"response.future.thing","a":1}"#));
        assert_eq!(
            ev,
            Some(StreamEvent::Unknown {
                event: "response.future.thing".into(),
                data: json!({"type":"response.future.thing","a":1})
            })
        );
        let ev = StreamEvent::from_frame(&f(
            Some("response.output_text.delta"),
            r#"{"output_index":0,"delta":"x","sequence_number":3}"#,
        ))
        .unwrap();
        assert!(matches!(ev, StreamEvent::OutputTextDelta { .. }));
        assert_eq!(ev.data()["sequence_number"], 3);
        assert_eq!(ev.name(), "response.output_text.delta");
        assert_eq!(StreamEvent::from_frame(&f(None, "[DONE]")), None);
    }

    #[test]
    fn response_usage_extraction() {
        let r: Response = serde_json::from_value(json!({
            "usage": {"input_tokens": 10, "output_tokens": 5,
                "input_tokens_details": {"cached_tokens": 4},
                "output_tokens_details": {"reasoning_tokens": 2}}
        }))
        .unwrap();
        assert_eq!(
            r.usage(),
            Usage {
                input_tokens: 10,
                output_tokens: 5,
                cached_tokens: 4,
                reasoning_tokens: 2
            }
        );
    }
}
