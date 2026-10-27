//! Responses IR -> Anthropic Messages request / client response.

use super::{ANTHROPIC_SIGNATURE, UNKNOWN_BLOCK, type_name};
use crate::error::CoreResult;
use crate::protocol::ir::{Content, ContentPart, InputItem, Reasoning, Tool};
use crate::protocol::{OutboundCtx, ir, thinking};
use serde_json::{Map, Value, json};

/// Anthropic requires `max_tokens`; used when neither the client nor the
/// model capabilities provide one.
const DEFAULT_MAX_TOKENS: u64 = 16384;

fn text_block(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

/// `data:<mt>;base64,<data>` / http URL -> Anthropic image block.
fn image_block(url: &str) -> Option<Value> {
    let source = if let Some(rest) = url.strip_prefix("data:") {
        let (meta, data) = rest.split_once(',')?;
        let media_type = meta.strip_suffix(";base64")?;
        json!({"type": "base64", "media_type": media_type, "data": data})
    } else {
        json!({"type": "url", "url": url})
    };
    Some(json!({"type": "image", "source": source}))
}

fn part_to_block(part: &ContentPart) -> Option<Value> {
    match part {
        ContentPart::InputText(t) | ContentPart::OutputText(t) => {
            (!t.text.is_empty()).then(|| text_block(&t.text))
        }
        ContentPart::InputImage(i) => i.image_url.as_deref().and_then(image_block),
        ContentPart::Unknown(v) => {
            tracing::debug!(
                part = type_name(v),
                "dropping content part with no anthropic equivalent"
            );
            None
        }
    }
}

/// function_call_output.output -> tool_result content.
fn tool_result_content(output: Option<&Value>) -> Value {
    match output {
        None | Some(Value::Null) => Value::String(String::new()),
        Some(Value::String(s)) => Value::String(s.clone()),
        Some(Value::Array(parts)) => Value::Array(
            parts
                .iter()
                .map(|p| match p.get("type").and_then(Value::as_str) {
                    Some("input_text") | Some("output_text") => {
                        text_block(p.get("text").and_then(Value::as_str).unwrap_or_default())
                    }
                    Some("input_image") => p
                        .get("image_url")
                        .and_then(Value::as_str)
                        .and_then(image_block)
                        .unwrap_or_else(|| p.clone()),
                    _ => p.clone(),
                })
                .collect(),
        ),
        Some(other) => Value::String(other.to_string()),
    }
}

/// A reasoning item is signed only when tagged `ANTHROPIC_SIGNATURE`; a signature is
/// never forged from foreign `encrypted_content`.
pub(super) fn is_anthropic_origin(r: &Reasoning) -> bool {
    r.extra.get(ANTHROPIC_SIGNATURE).and_then(Value::as_bool) == Some(true)
}

fn reasoning_text(r: &Reasoning) -> String {
    r.summary
        .iter()
        .flatten()
        .filter_map(|s| s.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(
            "
",
        )
}

/// `for_request`: upstream request path; untagged (foreign) items are dropped.
/// Client path: untagged items become an unsigned `thinking` block when they
/// have text, otherwise they are dropped.
fn reasoning_block(r: &Reasoning, for_request: bool) -> Option<Value> {
    if !is_anthropic_origin(r) {
        if for_request || r.extra.get("redacted").and_then(Value::as_bool) == Some(true) {
            return None;
        }
        let text = reasoning_text(r);
        return (!text.is_empty()).then(|| json!({"type": "thinking", "thinking": text}));
    }
    let signature = r.encrypted_content.as_deref()?;
    if r.extra.get("redacted").and_then(Value::as_bool) == Some(true) {
        return Some(json!({"type": "redacted_thinking", "data": signature}));
    }
    Some(json!({"type": "thinking", "thinking": reasoning_text(r), "signature": signature}))
}

#[derive(Default)]
struct Turns(Vec<(&'static str, Vec<Value>)>);

impl Turns {
    fn push(&mut self, role: &'static str, block: Value) {
        if self.0.last().map(|(r, _)| *r) != Some(role) {
            self.0.push((role, Vec::new()));
        }
        let blocks = &mut self.0.last_mut().expect("turn exists").1;
        if block["type"] == "tool_result" {
            // tool_result blocks must lead the user turn.
            let at = blocks
                .iter()
                .take_while(|b| b["type"] == "tool_result")
                .count();
            blocks.insert(at, block);
        } else {
            blocks.push(block);
        }
    }
}

fn convert_tool_choice(tc: &Value, parallel: Option<bool>) -> Option<Value> {
    let mut v = match tc {
        Value::String(s) => match s.as_str() {
            "auto" => json!({"type": "auto"}),
            "required" => json!({"type": "any"}),
            "none" => json!({"type": "none"}),
            _ => return None,
        },
        Value::Object(o) if o.get("type").and_then(Value::as_str) == Some("function") => {
            json!({"type": "tool", "name": o.get("name")?})
        }
        _ => return None,
    };
    if parallel == Some(false) && v["type"] != "none" {
        v["disable_parallel_tool_use"] = Value::Bool(true);
    }
    Some(v)
}

/// Effort levels Anthropic's `output_config.effort` accepts.
const ANTHROPIC_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max", "ultracode"];

pub fn from_ir(req: &ir::Request, ctx: &OutboundCtx) -> CoreResult<Value> {
    let mut system: Vec<String> = req.instructions.iter().cloned().collect();
    let mut turns = Turns::default();

    for item in &req.input {
        match item {
            InputItem::Message(m) => {
                if matches!(m.role.as_str(), "system" | "developer") {
                    match &m.content {
                        Content::Text(t) => system.push(t.clone()),
                        Content::Parts(ps) => {
                            for p in ps {
                                if let ContentPart::InputText(t) | ContentPart::OutputText(t) = p {
                                    system.push(t.text.clone());
                                }
                            }
                        }
                    }
                    continue;
                }
                let role = if m.role == "assistant" {
                    "assistant"
                } else {
                    "user"
                };
                match &m.content {
                    Content::Text(t) => {
                        if !t.is_empty() {
                            turns.push(role, text_block(t));
                        }
                    }
                    Content::Parts(ps) => {
                        for b in ps.iter().filter_map(part_to_block) {
                            turns.push(role, b);
                        }
                    }
                }
            }
            InputItem::FunctionCall(c) => {
                let (Some(id), Some(name)) = (&c.call_id, &c.name) else {
                    tracing::debug!("dropping function_call without call_id/name");
                    continue;
                };
                let input = c
                    .arguments
                    .as_deref()
                    .filter(|a| !a.trim().is_empty())
                    .map(|a| serde_json::from_str(a).unwrap_or_else(|_| json!({})))
                    .unwrap_or_else(|| json!({}));
                turns.push(
                    "assistant",
                    json!({"type": "tool_use", "id": id, "name": name, "input": input}),
                );
            }
            InputItem::FunctionCallOutput(o) => {
                let Some(id) = &o.call_id else {
                    tracing::debug!("dropping function_call_output without call_id");
                    continue;
                };
                let mut block = json!({
                    "type": "tool_result",
                    "tool_use_id": id,
                    "content": tool_result_content(o.output.as_ref()),
                });
                if o.extra.get("is_error").and_then(Value::as_bool) == Some(true) {
                    block["is_error"] = Value::Bool(true);
                }
                turns.push("user", block);
            }
            InputItem::Reasoning(r) => match reasoning_block(r, true) {
                Some(b) => turns.push("assistant", b),
                None => tracing::debug!("dropping reasoning item without an anthropic signature"),
            },
            InputItem::Unknown(v) => {
                if v.get("type").and_then(Value::as_str) == Some(UNKNOWN_BLOCK) {
                    turns.push("assistant", v["block"].clone());
                } else {
                    tracing::debug!(
                        item_type = type_name(v),
                        "dropping input item with no anthropic equivalent"
                    );
                }
            }
        }
    }

    // Same rule as `thinking::apply_raw` (spec 5.5): the budget is limited below
    // the client's `max_tokens`; when it cannot fit, thinking is skipped.
    let max_tokens = req
        .max_output_tokens
        .or(ctx.capabilities.max_output_tokens.map(u64::from))
        .unwrap_or(DEFAULT_MAX_TOKENS);
    let thinking_budget = ctx
        .thinking
        .and_then(thinking::anthropic_budget)
        .and_then(|b| thinking::clamp_budget(b, max_tokens));

    let mut out = Map::new();
    out.insert("model".into(), json!(ctx.upstream_model));
    if let Some(b) = thinking_budget {
        out.insert(
            "thinking".into(),
            json!({"type": "enabled", "budget_tokens": b}),
        );
    } else {
        // The client's own effort, when Anthropic has a matching level.
        if let Some(effort) = req
            .reasoning
            .as_ref()
            .and_then(|r| r.effort.as_deref())
            .filter(|e| ANTHROPIC_EFFORTS.contains(e))
        {
            out.insert("output_config".into(), json!({"effort": effort}));
        }
        // temperature/top_p are rejected by Anthropic when thinking is enabled.
        if let Some(t) = req.temperature {
            out.insert("temperature".into(), json!(t));
        }
        if let Some(t) = req.top_p {
            out.insert("top_p".into(), json!(t));
        }
    }
    out.insert("max_tokens".into(), json!(max_tokens));
    system.retain(|s| !s.is_empty());
    if !system.is_empty() {
        out.insert("system".into(), json!(system.join("\n\n")));
    }
    out.insert(
        "messages".into(),
        Value::Array(
            turns
                .0
                .into_iter()
                .filter(|(_, b)| !b.is_empty())
                .map(|(role, content)| json!({"role": role, "content": content}))
                .collect(),
        ),
    );

    let tools: Vec<Value> = req
        .tools
        .iter()
        .filter_map(|t| match t {
            Tool::Function(f) => {
                let mut o = json!({
                    "name": f.name,
                    "input_schema": f.parameters.clone().unwrap_or_else(|| json!({"type": "object"})),
                });
                if let Some(d) = &f.description {
                    o["description"] = json!(d);
                }
                if let Some(cc) = f.extra.get("cache_control") {
                    o["cache_control"] = cc.clone();
                }
                Some(o)
            }
            Tool::Unknown(v) => {
                tracing::debug!(
                    tool_type = type_name(v),
                    "dropping tool with no anthropic equivalent"
                );
                None
            }
        })
        .collect();
    if !tools.is_empty() {
        out.insert("tools".into(), Value::Array(tools));
        let parallel = req
            .extra
            .get("parallel_tool_calls")
            .and_then(Value::as_bool);
        let choice = match &req.tool_choice {
            Some(tc) => convert_tool_choice(tc, parallel),
            None if parallel == Some(false) => convert_tool_choice(&json!("auto"), parallel),
            None => None,
        };
        if let Some(c) = choice {
            out.insert("tool_choice".into(), c);
        }
    }
    if let Some(s) = req.stream {
        out.insert("stream".into(), Value::Bool(s));
    }
    for k in ["stop_sequences", "top_k"] {
        if let Some(v) = req.extra.get(k) {
            out.insert(k.into(), v.clone());
        }
    }
    if let Some(uid) = req.metadata.as_ref().and_then(|m| m.get("user_id")) {
        out.insert("metadata".into(), json!({"user_id": uid}));
    }
    Ok(Value::Object(out))
}

pub fn response_to_client(resp: &ir::Response) -> Value {
    let mut content = Vec::new();
    let mut has_tool = false;
    for item in &resp.output {
        match item {
            InputItem::Message(m) => match &m.content {
                Content::Text(t) => {
                    if !t.is_empty() {
                        content.push(text_block(t));
                    }
                }
                Content::Parts(ps) => content.extend(ps.iter().filter_map(part_to_block)),
            },
            InputItem::FunctionCall(c) => {
                has_tool = true;
                let input = c
                    .arguments
                    .as_deref()
                    .and_then(|a| serde_json::from_str(a).ok())
                    .unwrap_or_else(|| json!({}));
                content.push(json!({
                    "type": "tool_use",
                    "id": c.call_id.clone().unwrap_or_default(),
                    "name": c.name.clone().unwrap_or_default(),
                    "input": input,
                }));
            }
            InputItem::Reasoning(r) => content.extend(reasoning_block(r, false)),
            InputItem::Unknown(v) => {
                if v.get("type").and_then(Value::as_str) == Some(UNKNOWN_BLOCK) {
                    content.push(v["block"].clone());
                } else {
                    tracing::debug!("dropping response output item with no anthropic equivalent");
                }
            }
            InputItem::FunctionCallOutput(_) => {}
        }
    }
    let stop_reason = if has_tool {
        "tool_use"
    } else if resp.status.as_deref() == Some("incomplete") {
        "max_tokens"
    } else {
        "end_turn"
    };
    let u = resp.usage();
    let mut usage = json!({"input_tokens": u.input_tokens, "output_tokens": u.output_tokens});
    if u.cached_tokens > 0 {
        usage["cache_read_input_tokens"] = json!(u.cached_tokens);
    }
    json!({
        "id": resp.id.clone().unwrap_or_else(|| "msg_yi".into()),
        "type": "message",
        "role": "assistant",
        "model": resp.model.clone().unwrap_or_default(),
        "content": content,
        "stop_reason": stop_reason,
        "stop_sequence": null,
        "usage": usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Capabilities, Thinking};
    use crate::protocol::anthropic::inbound;
    use serde_json::json;

    fn ctx(thinking: Option<Thinking>) -> OutboundCtx {
        OutboundCtx {
            upstream_model: "claude-up".into(),
            capabilities: Capabilities::default(),
            thinking,
        }
    }

    fn ir_req() -> ir::Request {
        inbound::to_ir(
            serde_json::from_str(include_str!(
                "../../../tests/fixtures/anthropic/messages_with_tools.json"
            ))
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn client_effort_becomes_output_config_effort() {
        let mut req = ir_req();
        req.reasoning = Some(ir::ReasoningConfig {
            effort: Some("high".into()),
            summary: None,
            extra: serde_json::Map::new(),
        });
        let v = from_ir(&req, &ctx(None)).unwrap();
        assert_eq!(v["output_config"], json!({"effort": "high"}));
        assert!(v.get("thinking").is_none());
        // controller-resolved thinking wins; unknown levels are not forwarded
        let v = from_ir(&req, &ctx(Some(Thinking::Low))).unwrap();
        assert!(v.get("output_config").is_none());
        req.reasoning.as_mut().unwrap().effort = Some("minimal".into());
        assert!(
            from_ir(&req, &ctx(None))
                .unwrap()
                .get("output_config")
                .is_none()
        );
    }

    #[test]
    fn thinking_budget_is_clamped_below_client_max_tokens() {
        let mut req = ir_req();
        req.max_output_tokens = Some(5000);
        let v = from_ir(&req, &ctx(Some(Thinking::Medium))).unwrap();
        assert_eq!(v["max_tokens"], 5000);
        assert_eq!(v["thinking"]["budget_tokens"], 4999);
        // cannot fit above the 1024 minimum: thinking is skipped, max_tokens untouched
        req.max_output_tokens = Some(1000);
        let v = from_ir(&req, &ctx(Some(Thinking::Medium))).unwrap();
        assert_eq!(v["max_tokens"], 1000);
        assert!(v.get("thinking").is_none());
    }

    #[test]
    fn ir_to_messages_with_thinking_budget() {
        let v = from_ir(&ir_req(), &ctx(Some(Thinking::High))).unwrap();
        assert_eq!(v["model"], "claude-up");
        assert_eq!(v["system"], "be brief");
        assert_eq!(v["thinking"]["type"], "enabled");
        let budget = v["thinking"]["budget_tokens"].as_u64().unwrap();
        assert!(budget >= 1024);
        assert!(v["max_tokens"].as_u64().unwrap() > budget);
        assert!(
            v.get("temperature").is_none(),
            "temperature invalid with thinking"
        );
        assert_eq!(v["stop_sequences"], json!(["END"]));
        assert_eq!(v["tool_choice"], json!({"type": "auto"}));
        assert_eq!(v["tools"][0]["name"], "get_weather");
        assert_eq!(v["tools"][0]["input_schema"]["type"], "object");
        let msgs = v["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(
            msgs[0],
            json!({"role": "user", "content": [{"type": "text", "text": "weather in Paris?"}]})
        );
        let a: Vec<_> = msgs[1]["content"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["type"].as_str().unwrap())
            .collect();
        assert_eq!(a, vec!["thinking", "text", "tool_use"]);
        assert_eq!(msgs[1]["content"][0]["signature"], "sig-abc");
        assert_eq!(msgs[1]["content"][2]["input"], json!({"city": "Paris"}));
        // tool_result first in the user turn, pairing intact
        assert_eq!(msgs[2]["content"][0]["type"], "tool_result");
        assert_eq!(msgs[2]["content"][0]["tool_use_id"], "toolu_1");
        assert_eq!(msgs[2]["content"][1]["type"], "text");
    }

    fn reasoning_req(tagged: bool, redacted: bool) -> ir::Request {
        let mut req: ir::Request = serde_json::from_value(json!({
            "model": "m",
            "input": [{"role": "user", "content": "hi"},
                      {"role": "assistant", "content": "ok"}]
        }))
        .unwrap();
        req.input.insert(
            1,
            ir::InputItem::Reasoning(reasoning_item(tagged, redacted)),
        );
        req
    }

    fn assistant_types(v: &Value) -> Vec<String> {
        v["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["role"] == "assistant")
            .flat_map(|m| m["content"].as_array().unwrap().clone())
            .map(|b| b["type"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn foreign_encrypted_content_is_not_sent_as_signature() {
        let v = from_ir(&reasoning_req(false, false), &ctx(None)).unwrap();
        assert!(!v.to_string().contains("ENC"));
        assert!(!assistant_types(&v).iter().any(|t| t.contains("thinking")));
    }

    #[test]
    fn anthropic_origin_signature_is_sent() {
        let v = from_ir(&reasoning_req(true, false), &ctx(None)).unwrap();
        let block = &v["messages"][1]["content"][0];
        assert_eq!(block["type"], "thinking");
        assert_eq!(block["signature"], "ENC");
        // redacted blocks keep their payload too
        let v = from_ir(&reasoning_req(true, true), &ctx(None)).unwrap();
        assert_eq!(v["messages"][1]["content"][0]["type"], "redacted_thinking");
    }

    #[test]
    fn inbound_thinking_is_tagged_anthropic_origin() {
        let ir = inbound::to_ir(
            serde_json::from_str(include_str!(
                "../../../tests/fixtures/anthropic/messages_with_tools.json"
            ))
            .unwrap(),
        )
        .unwrap();
        let tagged = ir.input.iter().any(|i| matches!(i,
            ir::InputItem::Reasoning(r) if r.extra.get(ANTHROPIC_SIGNATURE) == Some(&Value::Bool(true))));
        assert!(tagged);
    }

    fn reasoning_item(tagged: bool, redacted: bool) -> ir::Reasoning {
        let mut extra = Map::new();
        if tagged {
            extra.insert(ANTHROPIC_SIGNATURE.into(), Value::Bool(true));
        }
        if redacted {
            extra.insert("redacted".into(), Value::Bool(true));
        }
        ir::Reasoning {
            kind: "reasoning".into(),
            id: None,
            summary: Some(vec![json!({"type": "summary_text", "text": "hmm"})]),
            encrypted_content: Some("ENC".into()),
            extra,
        }
    }

    fn client_content(r: ir::Reasoning) -> Vec<Value> {
        let resp = ir::Response {
            output: vec![ir::InputItem::Reasoning(r)],
            ..Default::default()
        };
        response_to_client(&resp)["content"]
            .as_array()
            .unwrap()
            .clone()
    }

    #[test]
    fn client_response_never_forges_signature() {
        let c = client_content(reasoning_item(false, false));
        assert_eq!(c, vec![json!({"type": "thinking", "thinking": "hmm"})]);
        assert!(client_content(reasoning_item(false, true)).is_empty());
        let c = client_content(reasoning_item(true, false));
        assert_eq!(c[0]["signature"], "ENC");
        let c = client_content(reasoning_item(true, true));
        assert_eq!(c[0]["type"], "redacted_thinking");
    }

    #[test]
    fn forged_origin_tag_from_responses_client_is_stripped() {
        let req: ir::Request = serde_json::from_value(json!({
            "model": "m",
            "input": [{"role": "user", "content": "hi"},
                {"type": "reasoning", "summary": [], "encrypted_content": "ENC",
                 "anthropic_signature": true},
                {"role": "assistant", "content": "ok"}]
        }))
        .unwrap();
        let ir::InputItem::Reasoning(r) = &req.input[1] else {
            panic!("reasoning expected")
        };
        assert!(!r.extra.contains_key(ANTHROPIC_SIGNATURE));
        let v = from_ir(&req, &ctx(None)).unwrap();
        assert!(!v.to_string().contains("ENC"));
    }

    #[test]
    fn no_thinking_without_ctx() {
        let v = from_ir(&ir_req(), &ctx(None)).unwrap();
        assert!(v.get("thinking").is_none());
        assert_eq!(v["temperature"], 0.5);
        assert_eq!(v["max_tokens"], 2048);
    }

    #[test]
    fn parallel_tool_results_merge_into_one_user_turn() {
        let ir = inbound::to_ir(
            serde_json::from_str(include_str!(
                "../../../tests/fixtures/anthropic/tool_result_pairs.json"
            ))
            .unwrap(),
        )
        .unwrap();
        let v = from_ir(&ir, &ctx(None)).unwrap();
        let msgs = v["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        let last = msgs[2]["content"].as_array().unwrap();
        assert_eq!(last.len(), 2);
        assert_eq!(last[0]["content"], json!("ra"));
        assert_eq!(last[1]["is_error"], true);
    }

    #[test]
    fn response_to_client_blocks_and_stop_reason() {
        let r = inbound::response_to_ir(json!({
            "id": "msg_1", "model": "m", "stop_reason": "tool_use",
            "content": [
                {"type": "thinking", "thinking": "t", "signature": "s"},
                {"type": "text", "text": "hi"},
                {"type": "web_search_tool_result", "tool_use_id": "x", "content": []},
                {"type": "tool_use", "id": "t1", "name": "f", "input": {"a": 1}}
            ],
            "usage": {"input_tokens": 7, "output_tokens": 3}
        }))
        .unwrap();
        let v = response_to_client(&r);
        assert_eq!(v["type"], "message");
        assert_eq!(v["stop_reason"], "tool_use");
        assert_eq!(v["content"][0]["signature"], "s");
        assert_eq!(v["content"][2]["type"], "web_search_tool_result");
        assert_eq!(v["content"][3]["input"], json!({"a": 1}));
        assert_eq!(v["usage"]["input_tokens"], 7);
    }
}
