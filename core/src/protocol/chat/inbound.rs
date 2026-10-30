//! Chat Completions request/response -> Responses IR.

use super::REASONING_CONTENT;
use super::types::{ChatContent, ChatMessage, ChatRequest, ChatResponse, ToolCall, parse_usage};
use crate::error::{CoreError, CoreResult};
use crate::protocol::Usage;
use crate::protocol::ir::{
    self, Content, ContentPart, FunctionCall, FunctionCallOutput, FunctionTool, ImagePart,
    InputItem, Message, Reasoning, ReasoningConfig, TextPart, Tool,
};
use serde_json::{Map, Value, json};
use std::collections::HashSet;

/// `image_url` is either a bare URL string or `{url, detail}`.
fn image_part(v: &Value) -> Option<ContentPart> {
    let (url, detail) = match v.get("image_url")? {
        Value::String(u) => (u.clone(), None),
        o => (
            o.get("url")?.as_str()?.to_owned(),
            o.get("detail").and_then(Value::as_str).map(str::to_owned),
        ),
    };
    Some(ContentPart::InputImage(ImagePart {
        kind: "input_image".into(),
        image_url: Some(url),
        file_id: None,
        detail,
        extra: Map::new(),
    }))
}

fn convert_parts(parts: Vec<Value>, assistant: bool) -> Vec<ContentPart> {
    parts
        .into_iter()
        .filter_map(|p| match p.get("type").and_then(Value::as_str) {
            Some("text") => {
                let text = p.get("text").and_then(Value::as_str)?.to_owned();
                (!text.is_empty()).then(|| {
                    if assistant {
                        ContentPart::OutputText(TextPart::new("output_text", text))
                    } else {
                        ContentPart::InputText(TextPart::new("input_text", text))
                    }
                })
            }
            Some("image_url") => Some(image_part(&p).unwrap_or(ContentPart::Unknown(p))),
            // Unknown parts are kept verbatim, never dropped.
            _ => Some(ContentPart::Unknown(p)),
        })
        .collect()
}

fn convert_content(content: Option<ChatContent>, assistant: bool) -> Vec<ContentPart> {
    match content {
        None => vec![],
        Some(ChatContent::Text(t)) if t.is_empty() => vec![],
        Some(ChatContent::Text(t)) => {
            convert_parts(vec![json!({"type": "text", "text": t})], assistant)
        }
        Some(ChatContent::Parts(p)) => convert_parts(p, assistant),
    }
}

/// Tool message content -> `function_call_output.output`.
fn tool_output(content: Option<ChatContent>) -> Value {
    match content {
        None => Value::String(String::new()),
        Some(ChatContent::Text(t)) => Value::String(t),
        Some(ChatContent::Parts(parts)) => Value::String(
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
    }
}

fn function_call(c: ToolCall) -> InputItem {
    InputItem::FunctionCall(FunctionCall {
        kind: "function_call".into(),
        call_id: Some(c.id),
        name: Some(c.function.name),
        arguments: Some(if c.function.arguments.trim().is_empty() {
            "{}".into()
        } else {
            c.function.arguments
        }),
        extra: Map::new(),
    })
}

/// Reasoning item carrying the `reasoning_content` vendor extension.
fn reasoning_item(text: &str) -> InputItem {
    let mut extra = Map::new();
    extra.insert(REASONING_CONTENT.into(), Value::String(text.to_owned()));
    InputItem::Reasoning(Reasoning {
        kind: "reasoning".into(),
        id: None,
        summary: Some(vec![json!({"type": "summary_text", "text": text})]),
        encrypted_content: None,
        extra,
    })
}

/// Assistant message (returned by a Chat upstream or replayed by a client) ->
/// IR items: reasoning, text, then tool calls.
fn assistant_items(m: ChatMessage, out: &mut Vec<InputItem>) {
    if let Some(r) = m.reasoning_text() {
        out.push(reasoning_item(&r));
    }
    let parts = convert_content(m.content, true);
    if !parts.is_empty() {
        out.push(InputItem::Message(Message {
            kind: None,
            role: "assistant".into(),
            content: Content::Parts(parts),
            extra: Map::new(),
        }));
    }
    out.extend(m.tool_calls.into_iter().map(function_call));
}

fn convert_tool(tool: Value) -> Tool {
    if tool.get("type").and_then(Value::as_str) == Some("function")
        && let Some(f) = tool.get("function")
        && let Some(name) = f.get("name").and_then(Value::as_str)
    {
        return Tool::Function(FunctionTool {
            kind: "function".into(),
            name: name.to_owned(),
            description: f
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_owned),
            parameters: f.get("parameters").cloned(),
            strict: f.get("strict").and_then(Value::as_bool),
            extra: Map::new(),
        });
    }
    Tool::Unknown(tool)
}

fn convert_tool_choice(tc: Value) -> Value {
    if tc.get("type").and_then(Value::as_str) == Some("function")
        && let Some(name) = tc.pointer("/function/name")
    {
        return json!({"type": "function", "name": name});
    }
    tc
}

pub fn to_ir(body: Value) -> CoreResult<ir::Request> {
    let req: ChatRequest = serde_json::from_value(body)
        .map_err(|e| CoreError::Validation(format!("invalid chat completions request: {e}")))?;

    let mut input = Vec::new();
    // Tool-call ids announced by assistant messages so far; a `tool` message
    // must answer one of them.
    let mut call_ids: HashSet<String> = HashSet::new();
    for m in req.messages {
        match m.role.as_str() {
            "tool" => {
                let id = m.tool_call_id.clone().ok_or_else(|| {
                    CoreError::Validation("tool message is missing tool_call_id".into())
                })?;
                if !call_ids.contains(&id) {
                    return Err(CoreError::Validation(format!(
                        "tool message answers tool_call_id `{id}` that no preceding assistant message announced"
                    )));
                }
                input.push(InputItem::FunctionCallOutput(FunctionCallOutput {
                    kind: "function_call_output".into(),
                    call_id: Some(id),
                    output: Some(tool_output(m.content)),
                    extra: Map::new(),
                }));
            }
            "assistant" => {
                call_ids.extend(m.tool_calls.iter().map(|c| c.id.clone()));
                assistant_items(m, &mut input);
            }
            _ => {
                let parts = convert_content(m.content, false);
                if parts.is_empty() {
                    continue;
                }
                input.push(InputItem::Message(Message {
                    kind: None,
                    role: m.role,
                    content: Content::Parts(parts),
                    extra: Map::new(),
                }));
            }
        }
    }

    let mut extra = req.extra;
    // The upstream request sets its own stream_options (see outbound::from_ir).
    extra.remove("stream_options");
    if let Some(stop) = req.stop {
        let list = match stop {
            Value::String(s) => vec![Value::String(s)],
            Value::Array(a) => a,
            _ => vec![],
        };
        if !list.is_empty() {
            extra.insert("stop_sequences".into(), Value::Array(list));
        }
    }

    Ok(ir::Request {
        model: req.model,
        input,
        instructions: None,
        tools: req.tools.into_iter().map(convert_tool).collect(),
        tool_choice: req.tool_choice.map(convert_tool_choice),
        reasoning: req.reasoning_effort.map(|e| ReasoningConfig {
            effort: Some(e),
            summary: None,
            extra: Map::new(),
        }),
        max_output_tokens: req.max_completion_tokens.or(req.max_tokens),
        temperature: req.temperature,
        top_p: req.top_p,
        stream: req.stream,
        store: None,
        previous_response_id: None,
        include: None,
        metadata: req.metadata,
        extra,
    })
}

/// Normalised usage -> Responses-shaped usage JSON.
pub(crate) fn usage_value(u: Usage) -> Value {
    json!({
        "input_tokens": u.input_tokens,
        "output_tokens": u.output_tokens,
        "total_tokens": u.input_tokens + u.output_tokens,
        "input_tokens_details": {"cached_tokens": u.cached_tokens},
        "output_tokens_details": {"reasoning_tokens": u.reasoning_tokens},
    })
}

/// Whether a Chat `finish_reason` means the output was cut short.
pub(crate) fn is_incomplete(finish_reason: Option<&str>) -> bool {
    matches!(finish_reason, Some("length" | "content_filter"))
}

pub(crate) fn incomplete_details(finish_reason: Option<&str>) -> Value {
    let reason = if finish_reason == Some("content_filter") {
        "content_filter"
    } else {
        "max_output_tokens"
    };
    json!({"reason": reason})
}

pub fn response_to_ir(body: Value) -> CoreResult<ir::Response> {
    if !body.is_object() {
        return Err(CoreError::Validation(
            "chat completions response body must be a JSON object".into(),
        ));
    }
    let resp: ChatResponse = serde_json::from_value(body)
        .map_err(|e| CoreError::Validation(format!("invalid chat completions response: {e}")))?;
    let choice = resp.choices.into_iter().next();
    let finish = choice.as_ref().and_then(|c| c.finish_reason.clone());
    let mut output = Vec::new();
    if let Some(c) = choice {
        assistant_items(c.message, &mut output);
    }
    let incomplete = is_incomplete(finish.as_deref());
    let mut extra = Map::new();
    if incomplete {
        extra.insert(
            "incomplete_details".into(),
            incomplete_details(finish.as_deref()),
        );
    }
    Ok(ir::Response {
        id: resp.id,
        object: Some("response".into()),
        created_at: resp.created,
        model: resp.model,
        status: Some(
            if incomplete {
                "incomplete"
            } else {
                "completed"
            }
            .into(),
        ),
        output,
        usage: resp.usage.as_ref().map(|u| usage_value(parse_usage(u))),
        extra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ir::{Content, ContentPart, InputItem, Tool};
    use serde_json::json;

    fn req() -> Value {
        json!({
            "model": "gpt-x",
            "max_completion_tokens": 256,
            "temperature": 0.2,
            "stream": true,
            "stop": "END",
            "parallel_tool_calls": false,
            "reasoning_effort": "high",
            "stream_options": {"include_usage": true},
            "messages": [
                {"role": "system", "content": "be brief"},
                {"role": "user", "content": [
                    {"type": "text", "text": "look"},
                    {"type": "image_url", "image_url": {"url": "https://x/y.png", "detail": "low"}},
                    {"type": "input_audio", "input_audio": {"data": "zz", "format": "wav"}}
                ]},
                {"role": "assistant", "content": "checking", "tool_calls": [
                    {"id": "call_1", "type": "function", "function": {"name": "f", "arguments": "{\"a\":1}"}},
                    {"id": "call_2", "type": "function", "function": {"name": "g", "arguments": ""}}
                ]},
                {"role": "tool", "tool_call_id": "call_1", "content": "r1"},
                {"role": "tool", "tool_call_id": "call_2", "content": [{"type": "text", "text": "r2"}]}
            ],
            "tools": [
                {"type": "function", "function": {"name": "f", "description": "d", "parameters": {"type": "object"}, "strict": true}},
                {"type": "web_search_preview"}
            ],
            "tool_choice": {"type": "function", "function": {"name": "f"}}
        })
    }

    #[test]
    fn chat_messages_to_ir() {
        let r = to_ir(req()).unwrap();
        assert_eq!(r.model, "gpt-x");
        assert_eq!(r.max_output_tokens, Some(256));
        assert_eq!(r.temperature, Some(0.2));
        assert_eq!(r.stream, Some(true));
        assert_eq!(
            r.reasoning.as_ref().unwrap().effort.as_deref(),
            Some("high")
        );
        assert_eq!(r.extra["stop_sequences"], json!(["END"]));
        assert_eq!(r.extra["parallel_tool_calls"], json!(false));
        assert!(!r.extra.contains_key("stream_options"));
        assert_eq!(
            r.tool_choice,
            Some(json!({"type": "function", "name": "f"}))
        );
        assert!(matches!(&r.tools[0], Tool::Function(f) if f.name == "f"
            && f.strict == Some(true) && f.description.as_deref() == Some("d")));
        assert!(matches!(&r.tools[1], Tool::Unknown(_)));

        // system, user, assistant text, call_1, call_2, out_1, out_2
        assert_eq!(r.input.len(), 7);
        assert!(matches!(&r.input[0], InputItem::Message(m) if m.role == "system"));
        let InputItem::Message(u) = &r.input[1] else {
            panic!("user")
        };
        let Content::Parts(parts) = &u.content else {
            panic!("parts")
        };
        assert!(matches!(&parts[0], ContentPart::InputText(t) if t.text == "look"));
        assert!(matches!(&parts[1], ContentPart::InputImage(i)
            if i.image_url.as_deref() == Some("https://x/y.png") && i.detail.as_deref() == Some("low")));
        assert!(matches!(&parts[2], ContentPart::Unknown(v) if v["type"] == "input_audio"));
        assert!(
            matches!(&r.input[2], InputItem::Message(m) if m.role == "assistant"
            && matches!(&m.content, Content::Parts(p) if matches!(&p[0], ContentPart::OutputText(t) if t.text == "checking")))
        );
        assert!(matches!(&r.input[3], InputItem::FunctionCall(c)
            if c.call_id.as_deref() == Some("call_1") && c.arguments.as_deref() == Some("{\"a\":1}")));
        assert!(matches!(&r.input[4], InputItem::FunctionCall(c)
            if c.call_id.as_deref() == Some("call_2") && c.arguments.as_deref() == Some("{}")));
    }

    #[test]
    fn tool_messages_stay_paired() {
        let r = to_ir(req()).unwrap();
        assert!(matches!(&r.input[5], InputItem::FunctionCallOutput(o)
            if o.call_id.as_deref() == Some("call_1") && o.output == Some(json!("r1"))));
        assert!(matches!(&r.input[6], InputItem::FunctionCallOutput(o)
            if o.call_id.as_deref() == Some("call_2") && o.output == Some(json!("r2"))));
    }

    #[test]
    fn unpaired_tool_message_is_validation_error() {
        let err = to_ir(json!({"model": "m", "messages": [
            {"role": "user", "content": "q"},
            {"role": "tool", "tool_call_id": "ghost", "content": "x"}
        ]}))
        .unwrap_err();
        assert!(matches!(err, CoreError::Validation(_)), "{err:?}");
        let err = to_ir(json!({"model": "m", "messages": [
            {"role": "assistant", "content": null, "tool_calls": [
                {"id": "a", "type": "function", "function": {"name": "f", "arguments": "{}"}}]},
            {"role": "tool", "content": "no id"}
        ]}))
        .unwrap_err();
        assert!(matches!(err, CoreError::Validation(_)), "{err:?}");
    }

    #[test]
    fn tool_message_before_its_call_is_unpaired() {
        let err = to_ir(json!({"model": "m", "messages": [
            {"role": "tool", "tool_call_id": "a", "content": "x"},
            {"role": "assistant", "content": null, "tool_calls": [
                {"id": "a", "type": "function", "function": {"name": "f", "arguments": "{}"}}]}
        ]}))
        .unwrap_err();
        assert!(matches!(err, CoreError::Validation(_)));
    }

    #[test]
    fn reasoning_content_preserved() {
        let r = to_ir(json!({"model": "m", "messages": [
            {"role": "user", "content": "q"},
            {"role": "assistant", "content": "a", "reasoning_content": "thought"}
        ]}))
        .unwrap();
        assert_eq!(r.input.len(), 3);
        let InputItem::Reasoning(rs) = &r.input[1] else {
            panic!("reasoning item first")
        };
        assert_eq!(rs.extra["reasoning_content"], "thought");
        assert_eq!(rs.summary.as_ref().unwrap()[0]["text"], "thought");
        assert!(matches!(&r.input[2], InputItem::Message(m) if m.role == "assistant"));
    }

    #[test]
    fn invalid_request_is_validation_error() {
        assert!(matches!(
            to_ir(json!({"messages": []})).unwrap_err(),
            CoreError::Validation(_)
        ));
    }

    #[test]
    fn response_to_ir_maps_message_tools_reasoning_usage() {
        let r = response_to_ir(json!({
            "id": "chatcmpl-1", "model": "m", "created": 5,
            "choices": [{"index": 0, "finish_reason": "length", "message": {
                "role": "assistant", "content": "hi", "reasoning_content": "why",
                "tool_calls": [{"id": "c", "type": "function", "function": {"name": "f", "arguments": "{}"}}]
            }}],
            "usage": {"prompt_tokens": 9, "completion_tokens": 4,
                "prompt_tokens_details": {"cached_tokens": 3},
                "completion_tokens_details": {"reasoning_tokens": 2}}
        }))
        .unwrap();
        assert_eq!(r.id.as_deref(), Some("chatcmpl-1"));
        assert_eq!(r.status.as_deref(), Some("incomplete"));
        assert_eq!(r.output.len(), 3);
        assert!(matches!(&r.output[0], InputItem::Reasoning(_)));
        assert!(matches!(&r.output[1], InputItem::Message(_)));
        assert!(
            matches!(&r.output[2], InputItem::FunctionCall(c) if c.call_id.as_deref() == Some("c"))
        );
        let u = r.usage();
        assert_eq!(
            (
                u.input_tokens,
                u.output_tokens,
                u.cached_tokens,
                u.reasoning_tokens
            ),
            (9, 4, 3, 2)
        );
        assert!(response_to_ir(json!("x")).is_err());
    }
}
