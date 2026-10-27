//! Anthropic Messages request/response -> Responses IR.

use super::types::{Block, KnownBlock, MessageContent, MessagesRequest, MessagesResponse, System};
use super::{ANTHROPIC_SIGNATURE, UNKNOWN_BLOCK, type_name};
use crate::error::{CoreError, CoreResult};
use crate::protocol::ir::{
    self, Content, ContentPart, FunctionCall, FunctionCallOutput, FunctionTool, ImagePart,
    InputItem, Message, Reasoning, ReasoningConfig, TextPart, Tool,
};
use serde_json::{Map, Value, json};

fn text_part(kind: &str, text: String) -> TextPart {
    TextPart {
        kind: kind.into(),
        text,
        extra: Map::new(),
    }
}

/// `{"type":"base64",...}` / `{"type":"url",...}` image source -> image URL.
fn image_url(source: &Value) -> Option<String> {
    match source.get("type").and_then(Value::as_str) {
        Some("base64") => Some(format!(
            "data:{};base64,{}",
            source.get("media_type").and_then(Value::as_str)?,
            source.get("data").and_then(Value::as_str)?
        )),
        Some("url") => source.get("url").and_then(Value::as_str).map(str::to_owned),
        _ => None,
    }
}

fn image_part(source: &Value) -> Option<ContentPart> {
    Some(ContentPart::InputImage(ImagePart {
        kind: "input_image".into(),
        image_url: Some(image_url(source)?),
        file_id: None,
        detail: None,
        extra: Map::new(),
    }))
}

/// tool_result content -> IR function_call_output `output`.
fn tool_result_output(content: Option<Value>) -> Value {
    match content {
        None | Some(Value::Null) => Value::String(String::new()),
        Some(Value::Array(blocks)) => {
            let only_text = blocks
                .iter()
                .all(|b| b.get("type").and_then(Value::as_str) == Some("text"));
            if only_text {
                Value::String(
                    blocks
                        .iter()
                        .filter_map(|b| b.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            } else {
                Value::Array(
                    blocks
                        .into_iter()
                        .map(|b| match b.get("type").and_then(Value::as_str) {
                            Some("text") => json!({"type": "input_text", "text": b["text"]}),
                            Some("image") => match image_url(&b["source"]) {
                                Some(url) => json!({"type": "input_image", "image_url": url}),
                                None => b,
                            },
                            _ => b,
                        })
                        .collect(),
                )
            }
        }
        Some(other) => other,
    }
}

struct Pending {
    role: String,
    parts: Vec<ContentPart>,
}

fn flush(pending: &mut Option<Pending>, out: &mut Vec<InputItem>) {
    if let Some(p) = pending.take()
        && !p.parts.is_empty()
    {
        out.push(InputItem::Message(Message {
            kind: None,
            role: p.role,
            content: Content::Parts(p.parts),
            extra: Map::new(),
        }));
    }
}

fn convert_message(role: &str, content: MessageContent, out: &mut Vec<InputItem>) {
    let assistant = role == "assistant";
    let text_for_role = |t: String| {
        if assistant {
            ContentPart::OutputText(text_part("output_text", t))
        } else {
            ContentPart::InputText(text_part("input_text", t))
        }
    };
    let blocks = match content {
        MessageContent::Text(t) => {
            out.push(InputItem::Message(Message {
                kind: None,
                role: role.into(),
                content: Content::Parts(vec![text_for_role(t)]),
                extra: Map::new(),
            }));
            return;
        }
        MessageContent::Blocks(b) => b,
    };
    let mut pending: Option<Pending> = None;
    let push_part = |part: ContentPart, pending: &mut Option<Pending>| {
        pending
            .get_or_insert_with(|| Pending {
                role: role.into(),
                parts: Vec::new(),
            })
            .parts
            .push(part);
    };
    for block in blocks {
        let known = match block {
            Block::Known(k) => k,
            Block::Unknown(v) => {
                tracing::debug!(
                    block_type = type_name(&v),
                    "skipping unknown anthropic content block on cross-protocol path"
                );
                continue;
            }
        };
        match known {
            KnownBlock::Text { text, .. } => {
                if !text.is_empty() {
                    push_part(text_for_role(text), &mut pending);
                }
            }
            KnownBlock::Image { source, .. } => match image_part(&source) {
                Some(p) => push_part(p, &mut pending),
                None => tracing::debug!("skipping anthropic image with unsupported source"),
            },
            KnownBlock::ToolUse {
                id, name, input, ..
            } => {
                flush(&mut pending, out);
                out.push(InputItem::FunctionCall(FunctionCall {
                    kind: "function_call".into(),
                    call_id: Some(id),
                    name: Some(name),
                    arguments: Some(if input.is_null() {
                        "{}".into()
                    } else {
                        input.to_string()
                    }),
                    extra: Map::new(),
                }));
            }
            KnownBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
                ..
            } => {
                flush(&mut pending, out);
                let mut extra = Map::new();
                if is_error == Some(true) {
                    extra.insert("is_error".into(), Value::Bool(true));
                }
                out.push(InputItem::FunctionCallOutput(FunctionCallOutput {
                    kind: "function_call_output".into(),
                    call_id: Some(tool_use_id),
                    output: Some(tool_result_output(content)),
                    extra,
                }));
            }
            KnownBlock::Thinking {
                thinking,
                signature,
                ..
            } => {
                flush(&mut pending, out);
                let mut extra = Map::new();
                if signature.is_some() {
                    extra.insert(ANTHROPIC_SIGNATURE.into(), Value::Bool(true));
                }
                out.push(InputItem::Reasoning(Reasoning {
                    kind: "reasoning".into(),
                    id: None,
                    summary: Some(vec![json!({"type": "summary_text", "text": thinking})]),
                    encrypted_content: signature,
                    extra,
                }));
            }
            KnownBlock::RedactedThinking { data, .. } => {
                flush(&mut pending, out);
                let mut extra = Map::new();
                extra.insert("redacted".into(), Value::Bool(true));
                extra.insert(ANTHROPIC_SIGNATURE.into(), Value::Bool(true));
                out.push(InputItem::Reasoning(Reasoning {
                    kind: "reasoning".into(),
                    id: None,
                    summary: Some(vec![]),
                    encrypted_content: Some(data),
                    extra,
                }));
            }
        }
    }
    flush(&mut pending, out);
}

fn convert_tool(tool: Value) -> Option<Tool> {
    let kind = tool.get("type").and_then(Value::as_str);
    if !matches!(kind, None | Some("custom")) || tool.get("input_schema").is_none() {
        tracing::debug!(
            tool_type = kind.unwrap_or("?"),
            "skipping anthropic server-side tool on cross-protocol path"
        );
        return None;
    }
    let mut extra = Map::new();
    if let Some(cc) = tool.get("cache_control") {
        extra.insert("cache_control".into(), cc.clone());
    }
    Some(Tool::Function(FunctionTool {
        kind: "function".into(),
        name: tool.get("name")?.as_str()?.to_owned(),
        description: tool
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned),
        parameters: tool.get("input_schema").cloned(),
        strict: None,
        extra,
    }))
}

fn convert_tool_choice(tc: &Value, extra: &mut Map<String, Value>) -> Option<Value> {
    if tc.get("disable_parallel_tool_use").and_then(Value::as_bool) == Some(true) {
        extra.insert("parallel_tool_calls".into(), Value::Bool(false));
    }
    match tc.get("type").and_then(Value::as_str)? {
        "auto" => Some(json!("auto")),
        "any" => Some(json!("required")),
        "none" => Some(json!("none")),
        "tool" => Some(json!({"type": "function", "name": tc.get("name")?})),
        _ => None,
    }
}

/// Legacy thinking budget -> effort heuristic.
fn budget_to_effort(budget: u64) -> &'static str {
    match budget {
        0..8192 => "low",
        8192..20000 => "medium",
        _ => "high",
    }
}

pub fn to_ir(body: Value) -> CoreResult<ir::Request> {
    let req: MessagesRequest = serde_json::from_value(body)
        .map_err(|e| CoreError::Validation(format!("invalid anthropic request: {e}")))?;

    let instructions = match req.system {
        None => None,
        Some(System::Text(t)) => Some(t),
        Some(System::Blocks(blocks)) => {
            let texts: Vec<String> = blocks
                .into_iter()
                .filter_map(|b| match b {
                    Block::Known(KnownBlock::Text { text, .. }) => Some(text),
                    other => {
                        tracing::debug!(
                            block = ?other,
                            "dropping non-text anthropic system block"
                        );
                        None
                    }
                })
                .collect();
            (!texts.is_empty()).then(|| texts.join("\n\n"))
        }
    }
    .filter(|s| !s.is_empty());

    let mut input = Vec::new();
    for m in req.messages {
        convert_message(&m.role, m.content, &mut input);
    }

    let mut extra = req.extra;
    let tool_choice = req
        .tool_choice
        .as_ref()
        .and_then(|tc| convert_tool_choice(tc, &mut extra));

    // An explicit `output_config.effort` wins over the legacy thinking budget.
    let explicit_effort = extra
        .get_mut("output_config")
        .and_then(Value::as_object_mut)
        .and_then(|o| o.remove("effort"))
        .and_then(|e| e.as_str().map(str::to_owned));
    if extra
        .get("output_config")
        .and_then(Value::as_object)
        .is_some_and(Map::is_empty)
    {
        extra.remove("output_config");
    }
    let effort = explicit_effort.or_else(|| {
        let t = req.thinking.as_ref()?;
        if t.get("type").and_then(Value::as_str) != Some("enabled") {
            return None;
        }
        let budget = t.get("budget_tokens").and_then(Value::as_u64)?;
        Some(budget_to_effort(budget).to_owned())
    });
    let reasoning = effort.map(|effort| ReasoningConfig {
        effort: Some(effort),
        summary: None,
        extra: Map::new(),
    });

    Ok(ir::Request {
        model: req.model,
        input,
        instructions,
        tools: req.tools.into_iter().filter_map(convert_tool).collect(),
        tool_choice,
        reasoning,
        max_output_tokens: req.max_tokens,
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

/// Anthropic usage object -> Responses-shaped usage JSON.
pub(crate) fn usage_value(u: &Value) -> Value {
    let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
    let (i, o) = (n("input_tokens"), n("output_tokens"));
    json!({
        "input_tokens": i,
        "output_tokens": o,
        "total_tokens": i + o,
        "input_tokens_details": {"cached_tokens": n("cache_read_input_tokens")},
        "output_tokens_details": {"reasoning_tokens": 0},
    })
}

pub fn response_to_ir(body: Value) -> CoreResult<ir::Response> {
    if !body.is_object() {
        return Err(CoreError::Validation(
            "anthropic response body must be a JSON object".into(),
        ));
    }
    let resp: MessagesResponse = serde_json::from_value(body)
        .map_err(|e| CoreError::Validation(format!("invalid anthropic response: {e}")))?;
    let mut output = Vec::new();
    let mut tmp = Vec::new();
    for block in resp.content {
        match block {
            Block::Unknown(v) => output.push(InputItem::Unknown(
                json!({"type": UNKNOWN_BLOCK, "block": v}),
            )),
            Block::Known(k) => {
                convert_message(
                    "assistant",
                    MessageContent::Blocks(vec![Block::Known(k)]),
                    &mut tmp,
                );
                output.append(&mut tmp);
            }
        }
    }
    let incomplete = resp.stop_reason.as_deref() == Some("max_tokens");
    Ok(ir::Response {
        id: resp.id,
        object: Some("response".into()),
        created_at: None,
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
        usage: resp.usage.as_ref().map(usage_value),
        extra: Map::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ir::{Content, ContentPart, InputItem, Tool};
    use serde_json::json;

    fn fixture(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn client_effort_and_budget_map_to_reasoning_effort() {
        let base = json!({"model": "m", "max_tokens": 100, "messages": [{"role": "user", "content": "hi"}]});
        let effort = |extra: Value| {
            let mut b = base.clone();
            b.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            to_ir(b).unwrap().reasoning.and_then(|r| r.effort)
        };
        assert_eq!(
            effort(json!({"output_config": {"effort": "max"}})).as_deref(),
            Some("max")
        );
        let thinking = |n: u64| json!({"thinking": {"type": "enabled", "budget_tokens": n}});
        assert_eq!(effort(thinking(1024)).as_deref(), Some("low"));
        assert_eq!(effort(thinking(10000)).as_deref(), Some("medium"));
        assert_eq!(effort(thinking(32000)).as_deref(), Some("high"));
        // explicit effort wins over the budget; the consumed key does not leak
        let mut b = base.clone();
        b["thinking"] = json!({"type": "enabled", "budget_tokens": 1024});
        b["output_config"] = json!({"effort": "high"});
        let ir = to_ir(b).unwrap();
        assert_eq!(ir.reasoning.unwrap().effort.as_deref(), Some("high"));
        assert!(!ir.extra.contains_key("output_config"));
        assert_eq!(effort(json!({"thinking": {"type": "disabled"}})), None);
    }

    #[test]
    fn messages_with_tools_to_ir() {
        let req = to_ir(fixture(include_str!(
            "../../../tests/fixtures/anthropic/messages_with_tools.json"
        )))
        .unwrap();
        assert_eq!(req.model, "claude-sonnet-4");
        assert_eq!(req.instructions.as_deref(), Some("be brief"));
        assert_eq!(req.max_output_tokens, Some(2048));
        assert_eq!(req.temperature, Some(0.5));
        assert_eq!(req.tool_choice, Some(json!("auto")));
        assert_eq!(req.extra["stop_sequences"], json!(["END"]));
        // server-side tool is skipped on the cross-protocol path
        assert_eq!(req.tools.len(), 1);
        assert!(
            matches!(&req.tools[0], Tool::Function(f) if f.name == "get_weather"
            && f.parameters.as_ref().unwrap()["type"] == "object")
        );
        // user, reasoning, assistant text, function_call (server_tool_use skipped),
        // function_call_output, user text
        assert_eq!(req.input.len(), 6);
        assert!(matches!(&req.input[1], InputItem::Reasoning(r)
            if r.encrypted_content.as_deref() == Some("sig-abc")));
        assert!(
            matches!(&req.input[2], InputItem::Message(m) if m.role == "assistant"
            && matches!(&m.content, Content::Parts(p) if matches!(&p[0], ContentPart::OutputText(t) if t.text == "Checking.")))
        );
        assert!(matches!(&req.input[3], InputItem::FunctionCall(c)
            if c.call_id.as_deref() == Some("toolu_1")
            && c.name.as_deref() == Some("get_weather")
            && c.arguments.as_deref() == Some(r#"{"city":"Paris"}"#)));
        assert!(matches!(&req.input[4], InputItem::FunctionCallOutput(o)
            if o.call_id.as_deref() == Some("toolu_1") && o.output == Some(json!("sunny"))));
        assert!(matches!(&req.input[5], InputItem::Message(m) if m.role == "user"));
    }

    #[test]
    fn system_blocks_become_instructions() {
        let req = to_ir(fixture(include_str!(
            "../../../tests/fixtures/anthropic/system_blocks.json"
        )))
        .unwrap();
        assert_eq!(
            req.instructions.as_deref(),
            Some("You are helpful.\n\nAnswer in French.")
        );
        assert_eq!(req.input.len(), 1);
    }

    #[test]
    fn tool_result_pairs_preserved() {
        let req = to_ir(fixture(include_str!(
            "../../../tests/fixtures/anthropic/tool_result_pairs.json"
        )))
        .unwrap();
        let calls: Vec<_> = req
            .input
            .iter()
            .filter_map(|i| match i {
                InputItem::FunctionCall(c) => c.call_id.clone(),
                _ => None,
            })
            .collect();
        let outs: Vec<_> = req
            .input
            .iter()
            .filter_map(|i| match i {
                InputItem::FunctionCallOutput(o) => Some(o),
                _ => None,
            })
            .collect();
        assert_eq!(calls, vec!["toolu_a", "toolu_b"]);
        assert_eq!(outs.len(), 2);
        assert_eq!(outs[0].call_id.as_deref(), Some("toolu_a"));
        assert_eq!(outs[0].output, Some(json!("ra")));
        assert_eq!(outs[1].call_id.as_deref(), Some("toolu_b"));
        assert_eq!(outs[1].extra["is_error"], json!(true));
    }

    #[test]
    fn malformed_known_block_is_skipped_rest_intact() {
        let req = to_ir(json!({
            "model": "m", "max_tokens": 10,
            "messages": [
                {"role": "user", "content": "q"},
                {"role": "assistant", "content": [
                    {"type": "tool_use", "name": "no_id", "input": {}},
                    {"type": "text", "text": "kept"}
                ]}
            ]
        }))
        .unwrap();
        assert_eq!(req.input.len(), 2);
        assert!(matches!(&req.input[1], InputItem::Message(m)
            if matches!(&m.content, Content::Parts(p) if p.len() == 1)));
    }

    #[test]
    fn response_to_ir_rejects_non_object() {
        assert!(response_to_ir(json!("oops")).is_err());
        assert!(response_to_ir(json!([1])).is_err());
    }

    #[test]
    fn tool_use_without_input_gets_empty_object_arguments() {
        let req = to_ir(json!({
            "model": "m", "max_tokens": 10,
            "messages": [{"role": "assistant", "content": [
                {"type": "tool_use", "id": "t", "name": "f"}
            ]}]
        }))
        .unwrap();
        assert!(matches!(&req.input[0], InputItem::FunctionCall(c)
            if c.arguments.as_deref() == Some("{}")));
    }

    #[test]
    fn response_to_ir_keeps_unknown_blocks_and_usage() {
        let r = response_to_ir(json!({
            "id": "msg_1", "model": "m", "stop_reason": "max_tokens",
            "content": [
                {"type": "text", "text": "hi"},
                {"type": "web_search_tool_result", "tool_use_id": "s", "content": []},
                {"type": "tool_use", "id": "t1", "name": "f", "input": {"a": 1}}
            ],
            "usage": {"input_tokens": 7, "output_tokens": 3, "cache_read_input_tokens": 2}
        }))
        .unwrap();
        assert_eq!(r.status.as_deref(), Some("incomplete"));
        assert_eq!(r.output.len(), 3);
        assert!(matches!(&r.output[1], InputItem::Unknown(_)));
        let u = r.usage();
        assert_eq!(
            (u.input_tokens, u.output_tokens, u.cached_tokens),
            (7, 3, 2)
        );
    }
}
