//! Responses IR -> Chat Completions request / client response.

use super::{ANTHROPIC_UNKNOWN_BLOCK, REASONING_CONTENT};
use crate::domain::Protocol;
use crate::domain::capabilities::effort_levels;
use crate::error::{CoreError, CoreResult};
use crate::protocol::ir::{Content, ContentPart, InputItem, Reasoning, Tool};
use crate::protocol::{OutboundCtx, Usage, ir, thinking};
use serde_json::{Map, Value, json};
use std::collections::HashSet;

/// IR `extra` keys that have a Chat Completions equivalent and are forwarded as-is.
const FORWARDED_EXTRA: [&str; 8] = [
    "parallel_tool_calls",
    "seed",
    "frequency_penalty",
    "presence_penalty",
    "response_format",
    "user",
    "logprobs",
    "top_logprobs",
];

pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Chat `finish_reason` for an incomplete response.
pub(crate) fn incomplete_finish_reason(resp: &ir::Response) -> &'static str {
    let reason = resp
        .extra
        .get("incomplete_details")
        .and_then(|d| d.get("reason"))
        .and_then(Value::as_str);
    if reason == Some("content_filter") {
        "content_filter"
    } else {
        "length"
    }
}

/// Chat `usage` object for a normalised [`Usage`].
pub(crate) fn chat_usage(u: Usage) -> Value {
    json!({
        "prompt_tokens": u.input_tokens,
        "completion_tokens": u.output_tokens,
        "total_tokens": u.input_tokens + u.output_tokens,
        "prompt_tokens_details": {"cached_tokens": u.cached_tokens},
        "completion_tokens_details": {"reasoning_tokens": u.reasoning_tokens},
    })
}

/// Controller-resolved thinking wins; otherwise the client's own effort when
/// Chat can express it.
fn reasoning_effort(req: &ir::Request, ctx: &OutboundCtx) -> Option<String> {
    match ctx.thinking {
        Some(t) => Some(thinking::effort_name(t).into()),
        None => req
            .reasoning
            .as_ref()
            .and_then(|r| r.effort.clone())
            .filter(|e| effort_levels(Protocol::OpenaiChat).contains(&e.as_str())),
    }
}

/// Text of an IR reasoning item: the `reasoning_content` extension, else the summary.
fn reasoning_text(r: &Reasoning) -> String {
    if let Some(t) = r.extra.get(REASONING_CONTENT).and_then(Value::as_str) {
        return t.to_owned();
    }
    r.summary
        .iter()
        .flatten()
        .filter_map(|s| s.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

fn part_text(p: &ContentPart) -> Option<&str> {
    match p {
        ContentPart::InputText(t) | ContentPart::OutputText(t) => {
            Some(t.text.as_str()).filter(|t| !t.is_empty())
        }
        _ => None,
    }
}

fn message_text(c: &Content) -> String {
    match c {
        Content::Text(t) => t.clone(),
        Content::Parts(ps) => ps
            .iter()
            .filter_map(|p| {
                let text = part_text(p);
                if text.is_none()
                    && !matches!(p, ContentPart::InputText(_) | ContentPart::OutputText(_))
                {
                    tracing::warn!("dropping non-text content part from a text-only chat message");
                }
                text
            })
            .collect::<Vec<_>>()
            .join(""),
    }
}

/// User content: a plain string, or typed parts when it carries images.
fn user_content(c: &Content) -> Option<Value> {
    let Content::Parts(parts) = c else {
        let t = message_text(c);
        return (!t.is_empty()).then_some(Value::String(t));
    };
    let mut out = Vec::new();
    let mut has_image = false;
    for p in parts {
        match p {
            ContentPart::InputText(_) | ContentPart::OutputText(_) => {
                out.extend(part_text(p).map(|t| json!({"type": "text", "text": t})));
            }
            ContentPart::InputImage(i) => match &i.image_url {
                Some(url) => {
                    has_image = true;
                    let mut image = json!({"url": url});
                    if let Some(d) = &i.detail {
                        image["detail"] = json!(d);
                    }
                    out.push(json!({"type": "image_url", "image_url": image}));
                }
                None => tracing::warn!("dropping input_image without image_url"),
            },
            ContentPart::Unknown(v) => {
                let part_type = v.get("type").and_then(Value::as_str).unwrap_or("?");
                tracing::warn!(part_type, "dropping content part with no chat equivalent");
            }
        }
    }
    if out.is_empty() {
        None
    } else if has_image {
        Some(Value::Array(out))
    } else {
        Some(Value::String(message_text(c)))
    }
}

/// function_call_output.output -> tool message text.
fn tool_text(output: Option<&Value>) -> String {
    match output {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => {
            let texts: Vec<&str> = parts
                .iter()
                .filter(|p| {
                    matches!(
                        p.get("type").and_then(Value::as_str),
                        Some("input_text" | "output_text")
                    )
                })
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect();
            if texts.len() == parts.len() {
                texts.join("\n")
            } else {
                Value::Array(parts.clone()).to_string()
            }
        }
        Some(other) => other.to_string(),
    }
}

/// The assistant turn being assembled: reasoning, text and tool calls fold
/// into one Chat message so tool messages directly follow their `tool_calls`.
#[derive(Default)]
struct Assistant {
    text: String,
    reasoning: Vec<String>,
    calls: Vec<Value>,
}

impl Assistant {
    fn has_output(&self) -> bool {
        !self.text.is_empty() || !self.calls.is_empty()
    }

    /// Emit the pending turn (if any). Taking the state also clears pending
    /// reasoning, so later tool messages never get a stray assistant message
    /// inserted between them and their `tool_calls`.
    fn flush(&mut self, messages: &mut Vec<Value>) {
        let a = std::mem::take(self);
        if a.text.is_empty() && a.calls.is_empty() && a.reasoning.is_empty() {
            return;
        }
        // Strict OpenAI-compatible upstreams reject an assistant message
        // without `content`; tool-call-only turns keep an empty string.
        let mut m = json!({"role": "assistant", "content": a.text});
        let reasoning = a.reasoning.join("\n");
        if !reasoning.is_empty() {
            m[REASONING_CONTENT] = json!(reasoning);
        }
        if !a.calls.is_empty() {
            m["tool_calls"] = Value::Array(a.calls);
        }
        messages.push(m);
    }
}

fn convert_tool_choice(tc: &Value) -> Option<Value> {
    match tc {
        Value::String(s) if matches!(s.as_str(), "auto" | "none" | "required") => Some(tc.clone()),
        Value::Object(o) if o.get("type").and_then(Value::as_str) == Some("function") => {
            Some(json!({"type": "function", "function": {"name": o.get("name")?}}))
        }
        _ => None,
    }
}

pub fn from_ir(req: &ir::Request, ctx: &OutboundCtx) -> CoreResult<Value> {
    let mut messages: Vec<Value> = Vec::new();
    if let Some(i) = req.instructions.as_deref().filter(|i| !i.is_empty()) {
        messages.push(json!({"role": "system", "content": i}));
    }
    let mut asst = Assistant::default();
    let mut call_ids: HashSet<&str> = HashSet::new();
    // Calls of the current assistant turn; cleared once another message intervenes.
    let mut open_ids: HashSet<&str> = HashSet::new();

    for item in &req.input {
        match item {
            InputItem::Message(m) => match m.role.as_str() {
                "assistant" => {
                    if matches!(&m.content, Content::Parts(ps)
                        if ps.iter().any(|p| matches!(p, ContentPart::InputImage(_))))
                    {
                        return Err(CoreError::Validation(
                            "chat completions only accepts images in user messages".into(),
                        ));
                    }
                    if asst.has_output() {
                        asst.flush(&mut messages);
                        open_ids.clear();
                    }
                    asst.text = message_text(&m.content);
                }
                role => {
                    asst.flush(&mut messages);
                    let (role, content) = if matches!(role, "system" | "developer") {
                        // Classic role set only: most Chat-compatible upstreams reject `developer`.
                        let t = message_text(&m.content);
                        ("system", (!t.is_empty()).then_some(Value::String(t)))
                    } else {
                        ("user", user_content(&m.content))
                    };
                    if let Some(content) = content {
                        messages.push(json!({"role": role, "content": content}));
                        // Calls announced before this message can no longer be answered.
                        open_ids.clear();
                    }
                }
            },
            InputItem::Reasoning(r) => {
                if asst.has_output() {
                    asst.flush(&mut messages);
                    open_ids.clear();
                }
                let t = reasoning_text(r);
                if !t.is_empty() {
                    asst.reasoning.push(t);
                }
            }
            InputItem::FunctionCall(c) => {
                let (Some(id), Some(name)) = (&c.call_id, &c.name) else {
                    return Err(CoreError::Validation(
                        "function_call is missing call_id or name".into(),
                    ));
                };
                call_ids.insert(id);
                open_ids.insert(id);
                asst.calls.push(json!({
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": c.arguments.as_deref().filter(|a| !a.trim().is_empty()).unwrap_or("{}"),
                    },
                }));
            }
            InputItem::FunctionCallOutput(o) => {
                let id = o.call_id.as_deref().ok_or_else(|| {
                    CoreError::Validation("function_call_output is missing call_id".into())
                })?;
                if !call_ids.contains(id) {
                    return Err(CoreError::Validation(format!(
                        "function_call_output `{id}` has no preceding function_call"
                    )));
                }
                if !open_ids.contains(id) {
                    return Err(CoreError::Validation(format!(
                        "工具输出必须紧跟对应的工具调用（call_id `{id}` 与其调用之间插入了其他消息）"
                    )));
                }
                asst.flush(&mut messages);
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": id,
                    "content": tool_text(o.output.as_ref()),
                }));
            }
            InputItem::Unknown(v) => {
                let ty = v.get("type").and_then(Value::as_str).unwrap_or("?");
                if ty == ANTHROPIC_UNKNOWN_BLOCK {
                    tracing::debug!("dropping foreign anthropic block");
                } else {
                    tracing::warn!(
                        item_type = ty,
                        "dropping input item with no chat equivalent"
                    );
                }
            }
        }
    }
    asst.flush(&mut messages);
    if messages.is_empty() {
        return Err(CoreError::Validation(
            "input is empty or entirely ignored".into(),
        ));
    }

    let mut out = Map::new();
    out.insert("model".into(), json!(ctx.upstream_model));
    out.insert("messages".into(), Value::Array(messages));
    if let Some(n) = req.max_output_tokens {
        out.insert("max_completion_tokens".into(), json!(n));
    }
    if let Some(t) = req.temperature {
        out.insert("temperature".into(), json!(t));
    }
    if let Some(t) = req.top_p {
        out.insert("top_p".into(), json!(t));
    }
    if let Some(e) = reasoning_effort(req, ctx) {
        out.insert("reasoning_effort".into(), json!(e));
    }

    let tools: Vec<Value> = req
        .tools
        .iter()
        .filter_map(|t| match t {
            Tool::Function(f) => {
                let mut def = json!({
                    "name": f.name,
                    "parameters": f.parameters.clone().unwrap_or_else(|| json!({"type": "object"})),
                });
                if let Some(d) = &f.description {
                    def["description"] = json!(d);
                }
                if let Some(s) = f.strict {
                    def["strict"] = json!(s);
                }
                Some(json!({"type": "function", "function": def}))
            }
            Tool::Unknown(v) => {
                let tool_type = v.get("type").and_then(Value::as_str).unwrap_or("?");
                tracing::warn!(tool_type, "dropping tool with no chat equivalent");
                None
            }
        })
        .collect();
    if !tools.is_empty() {
        out.insert("tools".into(), Value::Array(tools));
        if let Some(c) = req.tool_choice.as_ref().and_then(convert_tool_choice) {
            out.insert("tool_choice".into(), c);
        }
    }

    for k in FORWARDED_EXTRA {
        if let Some(v) = req.extra.get(k) {
            out.insert(k.into(), v.clone());
        }
    }
    if let Some(stop) = req
        .extra
        .get("stop_sequences")
        .or_else(|| req.extra.get("stop"))
    {
        out.insert("stop".into(), stop.clone());
    }
    // Chat `metadata` is a string -> string map.
    if let Some(m) = req.metadata.as_ref().filter(|m| {
        m.as_object()
            .is_some_and(|o| o.values().all(Value::is_string))
    }) {
        out.insert("metadata".into(), m.clone());
    }
    if let Some(s) = req.stream {
        out.insert("stream".into(), Value::Bool(s));
        if s {
            out.insert("stream_options".into(), json!({"include_usage": true}));
        }
    }
    Ok(Value::Object(out))
}

pub fn response_to_client(resp: &ir::Response) -> Value {
    let mut text = String::new();
    let mut reasoning = Vec::new();
    let mut calls = Vec::new();
    for item in &resp.output {
        match item {
            InputItem::Message(m) => text.push_str(&message_text(&m.content)),
            InputItem::Reasoning(r) => reasoning.push(reasoning_text(r)),
            InputItem::FunctionCall(c) => calls.push(json!({
                "id": c.call_id.clone().unwrap_or_default(),
                "type": "function",
                "function": {
                    "name": c.name.clone().unwrap_or_default(),
                    "arguments": c.arguments.clone().unwrap_or_else(|| "{}".into()),
                },
            })),
            InputItem::FunctionCallOutput(_) => {}
            InputItem::Unknown(v) => {
                let ty = v.get("type").and_then(Value::as_str).unwrap_or("?");
                if ty == ANTHROPIC_UNKNOWN_BLOCK {
                    tracing::debug!("dropping foreign anthropic block");
                } else {
                    tracing::warn!(
                        item_type = ty,
                        "dropping response item with no chat equivalent"
                    );
                }
            }
        }
    }
    reasoning.retain(|r| !r.is_empty());
    let finish_reason = if !calls.is_empty() {
        "tool_calls"
    } else if resp.status.as_deref() == Some("incomplete") {
        incomplete_finish_reason(resp)
    } else {
        "stop"
    };
    let mut message = Map::new();
    message.insert("role".into(), json!("assistant"));
    message.insert(
        "content".into(),
        if text.is_empty() && !calls.is_empty() {
            Value::Null
        } else {
            Value::String(text)
        },
    );
    if !reasoning.is_empty() {
        message.insert(REASONING_CONTENT.into(), json!(reasoning.join("\n")));
    }
    if !calls.is_empty() {
        message.insert("tool_calls".into(), Value::Array(calls));
    }
    json!({
        "id": resp.id.clone().unwrap_or_else(|| "chatcmpl-yi".into()),
        "object": "chat.completion",
        "created": resp.created_at.unwrap_or_else(now_secs),
        "model": resp.model.clone().unwrap_or_default(),
        "choices": [{"index": 0, "message": message, "finish_reason": finish_reason}],
        "usage": chat_usage(resp.usage()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Capabilities, Thinking};
    use crate::error::CoreError;
    use crate::protocol::chat::inbound;
    use serde_json::json;

    fn ctx(thinking: Option<Thinking>) -> OutboundCtx {
        OutboundCtx {
            upstream_model: "up-model".into(),
            capabilities: Capabilities::default(),
            thinking,
        }
    }

    fn ir_from(v: Value) -> ir::Request {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn ir_to_chat_sets_reasoning_effort() {
        let req = ir_from(json!({"model": "m", "input": "hi", "reasoning": {"effort": "high"}}));
        let v = from_ir(&req, &ctx(None)).unwrap();
        assert_eq!(v["reasoning_effort"], "high");
        assert_eq!(v["model"], "up-model");
        // controller-resolved thinking wins over the client's effort
        let v = from_ir(&req, &ctx(Some(Thinking::Low))).unwrap();
        assert_eq!(v["reasoning_effort"], "low");
        // effort the chat protocol cannot express is not sent
        let req = ir_from(json!({"model": "m", "input": "hi", "reasoning": {"effort": "bogus"}}));
        assert!(
            from_ir(&req, &ctx(None))
                .unwrap()
                .get("reasoning_effort")
                .is_none()
        );
    }

    #[test]
    fn stream_requests_include_usage() {
        let req = ir_from(json!({"model": "m", "input": "hi", "stream": true}));
        let v = from_ir(&req, &ctx(None)).unwrap();
        assert_eq!(v["stream"], true);
        assert_eq!(v["stream_options"], json!({"include_usage": true}));
        let req = ir_from(json!({"model": "m", "input": "hi"}));
        assert!(
            from_ir(&req, &ctx(None))
                .unwrap()
                .get("stream_options")
                .is_none()
        );
    }

    #[test]
    fn parallel_calls_fold_and_stay_paired() {
        let req = ir_from(json!({
            "model": "m", "instructions": "sys",
            "input": [
                {"role": "developer", "content": [{"type": "input_text", "text": "dev"}]},
                {"role": "user", "content": [{"type": "input_text", "text": "run"}]},
                {"type": "reasoning", "id": "rs", "summary": [{"type": "summary_text", "text": "think"}]},
                {"role": "assistant", "content": [{"type": "output_text", "text": "ok"}]},
                {"type": "function_call", "call_id": "c1", "name": "f", "arguments": "{}"},
                {"type": "function_call", "call_id": "c2", "name": "g", "arguments": "{}"},
                {"type": "function_call_output", "call_id": "c1", "output": "a"},
                {"type": "function_call_output", "call_id": "c2", "output": "b"},
                {"role": "user", "content": [{"type": "input_text", "text": "more"}]}
            ]
        }));
        let v = from_ir(&req, &ctx(None)).unwrap();
        let msgs = v["messages"].as_array().unwrap();
        let roles: Vec<&str> = msgs.iter().map(|m| m["role"].as_str().unwrap()).collect();
        assert_eq!(
            roles,
            [
                "system",
                "system",
                "user",
                "assistant",
                "tool",
                "tool",
                "user"
            ]
        );
        let a = &msgs[3];
        assert_eq!(a["content"], "ok");
        assert_eq!(a["reasoning_content"], "think");
        assert_eq!(a["tool_calls"].as_array().unwrap().len(), 2);
        assert_eq!(a["tool_calls"][0]["function"]["name"], "f");
        assert_eq!(msgs[4]["tool_call_id"], "c1");
        assert_eq!(msgs[5]["content"], "b");
    }

    #[test]
    fn reasoning_before_calls_does_not_split_tool_messages() {
        let req = ir_from(json!({
            "model": "m",
            "input": [
                {"role": "user", "content": [{"type": "input_text", "text": "run"}]},
                {"type": "reasoning", "id": "rs", "summary": [{"type": "summary_text", "text": "think"}]},
                {"type": "function_call", "call_id": "c1", "name": "f", "arguments": "{}"},
                {"type": "function_call_output", "call_id": "c1", "output": "a"},
                {"type": "function_call_output", "call_id": "c1", "output": "dup"}
            ]
        }));
        let v = from_ir(&req, &ctx(None)).unwrap();
        let roles: Vec<&str> = v["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["user", "assistant", "tool", "tool"]);
        // tool-call-only assistant message keeps an empty content string
        assert_eq!(v["messages"][1]["content"], "");
        assert_eq!(v["messages"][1]["reasoning_content"], "think");
    }

    #[test]
    fn interleaved_message_between_call_and_output_is_validation_error() {
        let req = ir_from(json!({
            "model": "m",
            "input": [
                {"type": "function_call", "call_id": "c1", "name": "f", "arguments": "{}"},
                {"role": "user", "content": [{"type": "input_text", "text": "hey"}]},
                {"type": "function_call_output", "call_id": "c1", "output": "a"}
            ]
        }));
        let err = from_ir(&req, &ctx(None)).unwrap_err();
        assert!(
            matches!(&err, CoreError::Validation(m) if m.contains("紧跟")),
            "{err:?}"
        );
    }

    #[test]
    fn unknown_tools_custom_items_and_parts_are_still_dropped() {
        let req = ir_from(json!({
            "model": "m",
            "tools": [{"type": "custom", "name": "apply_patch"}, {"type": "web_search"}],
            "input": [
                {"role": "user", "content": [
                    {"type": "input_text", "text": "q"},
                    {"type": "input_file", "file_id": "f1"}
                ]},
                {"type": "custom_tool_call", "call_id": "x", "name": "apply_patch", "input": "p"},
                {"type": "custom_tool_call_output", "call_id": "x", "output": "ok"}
            ]
        }));
        let v = from_ir(&req, &ctx(None)).unwrap();
        assert!(v.get("tools").is_none());
        assert_eq!(v["messages"].as_array().unwrap().len(), 1);
        assert_eq!(v["messages"][0]["content"], "q");
    }

    #[test]
    fn unpaired_function_call_output_is_validation_error() {
        let req = ir_from(json!({
            "model": "m",
            "input": [{"type": "function_call_output", "call_id": "ghost", "output": "x"}]
        }));
        assert!(matches!(
            from_ir(&req, &ctx(None)).unwrap_err(),
            CoreError::Validation(_)
        ));
    }

    #[test]
    fn tools_choice_and_scalars() {
        let req = ir_from(json!({
            "model": "m", "input": "hi", "max_output_tokens": 77, "temperature": 0.3,
            "tools": [
                {"type": "function", "name": "f", "description": "d", "parameters": {"type": "object"}, "strict": true},
                {"type": "web_search"}
            ],
            "tool_choice": {"type": "function", "name": "f"},
            "parallel_tool_calls": false,
            "stop_sequences": ["X"],
            "service_tier": "flex"
        }));
        let v = from_ir(&req, &ctx(None)).unwrap();
        assert_eq!(v["max_completion_tokens"], 77);
        assert_eq!(v["temperature"], 0.3);
        assert_eq!(v["tools"].as_array().unwrap().len(), 1);
        assert_eq!(v["tools"][0]["function"]["name"], "f");
        assert_eq!(v["tools"][0]["function"]["strict"], true);
        assert_eq!(
            v["tool_choice"],
            json!({"type": "function", "function": {"name": "f"}})
        );
        assert_eq!(v["parallel_tool_calls"], false);
        assert_eq!(v["stop"], json!(["X"]));
        assert!(v.get("service_tier").is_none());
    }

    #[test]
    fn user_images_become_image_url_parts() {
        let req = ir_from(json!({"model": "m", "input": [
            {"role": "user", "content": [
                {"type": "input_text", "text": "see"},
                {"type": "input_image", "image_url": "data:image/png;base64,AA", "detail": "high"}
            ]}
        ]}));
        let v = from_ir(&req, &ctx(None)).unwrap();
        assert_eq!(
            v["messages"][0]["content"],
            json!([
                {"type": "text", "text": "see"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,AA", "detail": "high"}}
            ])
        );
    }

    #[test]
    fn roundtrip_chat_to_ir_to_chat_keeps_reasoning_and_pairing() {
        let body = json!({"model": "m", "messages": [
            {"role": "user", "content": "q"},
            {"role": "assistant", "content": "a", "reasoning_content": "deep",
             "tool_calls": [{"id": "c", "type": "function", "function": {"name": "f", "arguments": "{}"}}]},
            {"role": "tool", "tool_call_id": "c", "content": "res"}
        ]});
        let v = from_ir(&inbound::to_ir(body).unwrap(), &ctx(None)).unwrap();
        let msgs = v["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[1]["reasoning_content"], "deep");
        assert_eq!(msgs[1]["content"], "a");
        assert_eq!(msgs[1]["tool_calls"][0]["id"], "c");
        assert_eq!(msgs[2]["tool_call_id"], "c");
    }

    #[test]
    fn foreign_anthropic_unknown_block_item_is_dropped() {
        let req = ir_from(json!({"model": "m", "input": [
            {"role": "user", "content": [{"type": "input_text", "text": "q"}]},
            {"type": "anthropic.unknown_block", "block": {"type": "server_tool_use", "id": "srv"}},
        ]}));
        let v = from_ir(&req, &ctx(None)).unwrap();
        assert_eq!(v["messages"].as_array().unwrap().len(), 1);
        assert!(!v.to_string().contains("server_tool_use"));
    }

    #[test]
    fn response_to_client_shapes_completion() {
        let r = inbound::response_to_ir(json!({
            "id": "chatcmpl-1", "model": "m",
            "choices": [{"index": 0, "finish_reason": "tool_calls", "message": {
                "role": "assistant", "content": null, "reasoning_content": "why",
                "tool_calls": [{"id": "c", "type": "function", "function": {"name": "f", "arguments": "{\"a\":1}"}}]}}],
            "usage": {"prompt_tokens": 9, "completion_tokens": 4,
                "prompt_tokens_details": {"cached_tokens": 3}}
        }))
        .unwrap();
        let v = response_to_client(&r);
        assert_eq!(v["object"], "chat.completion");
        assert_eq!(v["id"], "chatcmpl-1");
        assert_eq!(v["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(v["choices"][0]["message"]["reasoning_content"], "why");
        assert_eq!(v["choices"][0]["message"]["content"], Value::Null);
        assert_eq!(
            v["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"],
            "{\"a\":1}"
        );
        assert_eq!(v["usage"]["prompt_tokens"], 9);
        assert_eq!(v["usage"]["total_tokens"], 13);
        assert_eq!(v["usage"]["prompt_tokens_details"]["cached_tokens"], 3);
    }

    #[test]
    fn response_to_client_drops_foreign_anthropic_blocks_and_maps_length() {
        let r = ir::Response {
            status: Some("incomplete".into()),
            output: vec![
                serde_json::from_value(json!({"type": "anthropic.unknown_block",
                    "block": {"type": "web_search_tool_result"}}))
                .unwrap(),
                serde_json::from_value(json!({"role": "assistant", "content": [
                    {"type": "output_text", "text": "partial"}]}))
                .unwrap(),
            ],
            ..Default::default()
        };
        let v = response_to_client(&r);
        assert_eq!(v["choices"][0]["message"]["content"], "partial");
        assert_eq!(v["choices"][0]["finish_reason"], "length");
        assert!(!v.to_string().contains("web_search_tool_result"));
    }
}
