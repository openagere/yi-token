//! Anthropic SSE <-> IR stream events.

use super::inbound::usage_value;
use super::{ANTHROPIC_SIGNATURE, EVENT_PREFIX};
use crate::error::CoreResult;
use crate::protocol::Usage;
use crate::protocol::ir::{
    self, Content, ContentPart, FunctionCall, InputItem, Message, Reasoning, StreamEvent, TextPart,
};
use crate::protocol::sse::{self, SseFrame};
use bytes::Bytes;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};

// ------------------------------------------------------- upstream -> IR

#[derive(Debug, PartialEq)]
enum Kind {
    Text,
    Thinking,
    Redacted,
    Tool,
    Unknown,
}

#[derive(Debug)]
struct Blk {
    kind: Kind,
    text: String,
    signature: String,
    call_id: String,
    name: String,
    args: String,
}

impl Blk {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            text: String::new(),
            signature: String::new(),
            call_id: String::new(),
            name: String::new(),
            args: String::new(),
        }
    }
}

fn message_item(text: &str) -> InputItem {
    InputItem::Message(Message {
        kind: Some("message".into()),
        role: "assistant".into(),
        content: Content::Parts(if text.is_empty() {
            vec![]
        } else {
            vec![ContentPart::OutputText(TextPart {
                kind: "output_text".into(),
                text: text.into(),
                extra: Map::new(),
            })]
        }),
        extra: Map::new(),
    })
}

fn unknown_event(name: &str, data: &Value) -> StreamEvent {
    StreamEvent::Unknown {
        event: format!("{EVENT_PREFIX}{name}"),
        data: data.clone(),
    }
}

/// Anthropic upstream SSE -> IR (Responses) events.
#[derive(Debug, Default)]
pub struct UpstreamToIr {
    id: String,
    model: String,
    blocks: HashMap<u64, Blk>,
    done: BTreeMap<u64, InputItem>,
    usage: Usage,
    stop_reason: Option<String>,
}

impl UpstreamToIr {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn usage(&self) -> Usage {
        self.usage
    }

    fn absorb_usage(&mut self, u: &Value) {
        let get = |k: &str| u.get(k).and_then(Value::as_u64);
        if let Some(v) = get("input_tokens") {
            self.usage.input_tokens = v;
        }
        if let Some(v) = get("output_tokens") {
            self.usage.output_tokens = v;
        }
        if let Some(v) = get("cache_read_input_tokens") {
            self.usage.cached_tokens = v;
        }
    }

    fn response(&self, status: &str) -> ir::Response {
        let u = self.usage;
        let mut usage = usage_value(&json!({
            "input_tokens": u.input_tokens,
            "output_tokens": u.output_tokens,
            "cache_read_input_tokens": u.cached_tokens,
        }));
        usage["output_tokens_details"]["reasoning_tokens"] = json!(u.reasoning_tokens);
        let mut extra = Map::new();
        if status == "incomplete" {
            extra.insert(
                "incomplete_details".into(),
                json!({"reason": "max_output_tokens"}),
            );
        }
        ir::Response {
            id: Some(self.id.clone()),
            object: Some("response".into()),
            created_at: None,
            model: Some(self.model.clone()),
            status: Some(status.into()),
            output: self.done.values().cloned().collect(),
            usage: (status != "in_progress").then_some(usage),
            extra,
        }
    }

    pub fn on_frame(&mut self, frame: SseFrame) -> CoreResult<Vec<StreamEvent>> {
        if frame.data.trim() == "[DONE]" {
            return Ok(vec![]);
        }
        let Ok(data) = serde_json::from_str::<Value>(&frame.data) else {
            tracing::debug!("skipping non-JSON anthropic SSE frame");
            return Ok(vec![]);
        };
        let kind = data
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or(frame.event)
            .unwrap_or_default();
        let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
        let mut out = Vec::new();
        match kind.as_str() {
            "ping" => {}
            "message_start" => {
                let m = &data["message"];
                self.id = m["id"].as_str().unwrap_or_default().to_owned();
                self.model = m["model"].as_str().unwrap_or_default().to_owned();
                self.absorb_usage(&m["usage"]);
                out.push(StreamEvent::Created {
                    response: self.response("in_progress"),
                    extra: Map::new(),
                });
            }
            "content_block_start" => {
                let b = &data["content_block"];
                match b.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        self.blocks.insert(index, Blk::new(Kind::Text));
                        out.push(StreamEvent::OutputItemAdded {
                            output_index: index,
                            item: message_item(""),
                            extra: Map::new(),
                        });
                        if let Some(t) = b["text"].as_str().filter(|t| !t.is_empty()) {
                            self.blocks
                                .get_mut(&index)
                                .expect("inserted")
                                .text
                                .push_str(t);
                            out.push(StreamEvent::OutputTextDelta {
                                output_index: index,
                                content_index: Some(0),
                                delta: t.into(),
                                extra: Map::new(),
                            });
                        }
                    }
                    Some("thinking") => {
                        self.blocks.insert(index, Blk::new(Kind::Thinking));
                        out.push(StreamEvent::OutputItemAdded {
                            output_index: index,
                            item: InputItem::Reasoning(Reasoning {
                                kind: "reasoning".into(),
                                id: Some(format!("rs_{index}")),
                                summary: Some(vec![]),
                                encrypted_content: None,
                                extra: Map::new(),
                            }),
                            extra: Map::new(),
                        });
                    }
                    Some("redacted_thinking") => {
                        let mut blk = Blk::new(Kind::Redacted);
                        blk.signature = b["data"].as_str().unwrap_or_default().to_owned();
                        let item = redacted_item(index, &blk.signature);
                        self.blocks.insert(index, blk);
                        out.push(StreamEvent::OutputItemAdded {
                            output_index: index,
                            item,
                            extra: Map::new(),
                        });
                    }
                    Some("tool_use") => {
                        let mut blk = Blk::new(Kind::Tool);
                        blk.call_id = b["id"].as_str().unwrap_or_default().to_owned();
                        blk.name = b["name"].as_str().unwrap_or_default().to_owned();
                        out.push(StreamEvent::OutputItemAdded {
                            output_index: index,
                            item: InputItem::FunctionCall(FunctionCall {
                                kind: "function_call".into(),
                                call_id: Some(blk.call_id.clone()),
                                name: Some(blk.name.clone()),
                                arguments: Some(String::new()),
                                extra: Map::new(),
                            }),
                            extra: Map::new(),
                        });
                        self.blocks.insert(index, blk);
                    }
                    other => {
                        tracing::debug!(
                            block_type = other.unwrap_or("?"),
                            "unknown anthropic content block kept as IR Unknown event"
                        );
                        self.blocks.insert(index, Blk::new(Kind::Unknown));
                        out.push(unknown_event("content_block_start", &data));
                    }
                }
            }
            "content_block_delta" => {
                let d = &data["delta"];
                let kind = self.blocks.get(&index).map(|b| &b.kind);
                if kind == Some(&Kind::Unknown) {
                    out.push(unknown_event("content_block_delta", &data));
                } else {
                    match d.get("type").and_then(Value::as_str) {
                        Some("text_delta") => {
                            let t = d["text"].as_str().unwrap_or_default();
                            if let Some(b) = self.blocks.get_mut(&index) {
                                b.text.push_str(t);
                            }
                            out.push(StreamEvent::OutputTextDelta {
                                output_index: index,
                                content_index: Some(0),
                                delta: t.into(),
                                extra: Map::new(),
                            });
                        }
                        Some("thinking_delta") => {
                            let t = d["thinking"].as_str().unwrap_or_default();
                            if let Some(b) = self.blocks.get_mut(&index) {
                                b.text.push_str(t);
                            }
                            out.push(StreamEvent::ReasoningSummaryTextDelta {
                                output_index: index,
                                summary_index: Some(0),
                                delta: t.into(),
                                extra: Map::new(),
                            });
                        }
                        Some("signature_delta") => {
                            if let Some(b) = self.blocks.get_mut(&index) {
                                b.signature
                                    .push_str(d["signature"].as_str().unwrap_or_default());
                            }
                        }
                        Some("input_json_delta") => {
                            let t = d["partial_json"].as_str().unwrap_or_default();
                            if let Some(b) = self.blocks.get_mut(&index) {
                                b.args.push_str(t);
                            }
                            out.push(StreamEvent::FunctionCallArgumentsDelta {
                                output_index: index,
                                delta: t.into(),
                                extra: Map::new(),
                            });
                        }
                        other => {
                            tracing::debug!(
                                delta_type = other.unwrap_or("?"),
                                "unknown anthropic delta kept as IR Unknown event"
                            );
                            out.push(unknown_event("content_block_delta", &data));
                        }
                    }
                }
            }
            "content_block_stop" => {
                let Some(b) = self.blocks.remove(&index) else {
                    return Ok(out);
                };
                let item = match b.kind {
                    Kind::Unknown => {
                        out.push(unknown_event("content_block_stop", &data));
                        return Ok(out);
                    }
                    Kind::Text => message_item(&b.text),
                    Kind::Thinking => {
                        let signed = !b.signature.is_empty();
                        let mut extra = Map::new();
                        if signed {
                            extra.insert(ANTHROPIC_SIGNATURE.into(), Value::Bool(true));
                        }
                        InputItem::Reasoning(Reasoning {
                            kind: "reasoning".into(),
                            id: Some(format!("rs_{index}")),
                            summary: Some(vec![json!({"type": "summary_text", "text": b.text})]),
                            encrypted_content: signed.then_some(b.signature),
                            extra,
                        })
                    }
                    Kind::Redacted => redacted_item(index, &b.signature),
                    Kind::Tool => InputItem::FunctionCall(FunctionCall {
                        kind: "function_call".into(),
                        call_id: Some(b.call_id),
                        name: Some(b.name),
                        arguments: Some(if b.args.is_empty() {
                            "{}".into()
                        } else {
                            b.args
                        }),
                        extra: Map::new(),
                    }),
                };
                self.done.insert(index, item.clone());
                out.push(StreamEvent::OutputItemDone {
                    output_index: index,
                    item,
                    extra: Map::new(),
                });
            }
            "message_delta" => {
                if let Some(r) = data["delta"]["stop_reason"].as_str() {
                    self.stop_reason = Some(r.to_owned());
                }
                self.absorb_usage(&data["usage"]);
            }
            "message_stop" => {
                let incomplete = self.stop_reason.as_deref() == Some("max_tokens");
                let response = self.response(if incomplete {
                    "incomplete"
                } else {
                    "completed"
                });
                out.push(if incomplete {
                    StreamEvent::Incomplete {
                        response,
                        extra: Map::new(),
                    }
                } else {
                    StreamEvent::Completed {
                        response,
                        extra: Map::new(),
                    }
                });
            }
            "error" => out.push(StreamEvent::Error {
                message: data["error"]["message"].as_str().map(str::to_owned),
                extra: match data {
                    Value::Object(m) => m,
                    _ => Map::new(),
                },
            }),
            other => out.push(unknown_event(other, &data)),
        }
        Ok(out)
    }
}

fn redacted_item(index: u64, data: &str) -> InputItem {
    let mut extra = Map::new();
    extra.insert("redacted".into(), Value::Bool(true));
    extra.insert(ANTHROPIC_SIGNATURE.into(), Value::Bool(true));
    InputItem::Reasoning(Reasoning {
        kind: "reasoning".into(),
        id: Some(format!("rs_{index}")),
        summary: Some(vec![]),
        encrypted_content: Some(data.to_owned()),
        extra,
    })
}

// ------------------------------------------------------- IR -> client

#[derive(Debug, PartialEq, Clone, Copy)]
enum OpenKind {
    Text,
    Thinking,
    Tool,
    Redacted,
}

#[derive(Debug)]
struct Open {
    kind: OpenKind,
    output_index: u64,
    content_index: u64,
    saw_delta: bool,
}

/// IR (Responses) events -> Anthropic client SSE.
#[derive(Debug, Default)]
pub struct IrToClient {
    started: bool,
    finished: bool,
    next_index: u64,
    open: Option<Open>,
    tool_used: bool,
}

fn frame(name: &str, data: Value) -> Bytes {
    sse::encode(Some(name), &data.to_string())
}

impl IrToClient {
    pub fn new() -> Self {
        Self::default()
    }

    fn ensure_started(&mut self, response: Option<&ir::Response>, out: &mut Vec<Bytes>) {
        if self.started {
            return;
        }
        self.started = true;
        let id = response
            .and_then(|r| r.id.clone())
            .unwrap_or_else(|| "msg_yi".into());
        let model = response.and_then(|r| r.model.clone()).unwrap_or_default();
        out.push(frame(
            "message_start",
            json!({"type": "message_start", "message": {
                "id": id, "type": "message", "role": "assistant", "model": model,
                "content": [], "stop_reason": null, "stop_sequence": null,
                "usage": {"input_tokens": 0, "output_tokens": 0},
            }}),
        ));
    }

    fn close_open(&mut self, out: &mut Vec<Bytes>) {
        if self.open.take().is_some() {
            out.push(frame(
                "content_block_stop",
                json!({"type": "content_block_stop", "index": self.next_index - 1}),
            ));
        }
    }

    fn open_block(
        &mut self,
        kind: OpenKind,
        output_index: u64,
        content_index: u64,
        block: Value,
        out: &mut Vec<Bytes>,
    ) {
        self.close_open(out);
        out.push(frame(
            "content_block_start",
            json!({"type": "content_block_start", "index": self.next_index, "content_block": block}),
        ));
        self.next_index += 1;
        self.open = Some(Open {
            kind,
            output_index,
            content_index,
            saw_delta: false,
        });
    }

    fn delta(&mut self, delta: Value, out: &mut Vec<Bytes>) {
        if let Some(o) = &mut self.open {
            o.saw_delta = true;
        }
        out.push(frame(
            "content_block_delta",
            json!({"type": "content_block_delta", "index": self.next_index - 1, "delta": delta}),
        ));
    }

    fn open_text(&mut self, oi: u64, ci: u64, out: &mut Vec<Bytes>) {
        self.open_block(
            OpenKind::Text,
            oi,
            ci,
            json!({"type": "text", "text": ""}),
            out,
        );
    }

    fn error(&mut self, message: &str, out: &mut Vec<Bytes>) {
        self.close_open(out);
        out.push(frame(
            "error",
            json!({"type": "error", "error": {"type": "api_error", "message": message}}),
        ));
        self.finished = true;
    }

    pub fn on_event(&mut self, ev: &StreamEvent) -> Vec<Bytes> {
        let mut out = Vec::new();
        if self.finished {
            return out;
        }
        match ev {
            StreamEvent::Created { response, .. } => self.ensure_started(Some(response), &mut out),
            StreamEvent::OutputItemAdded {
                output_index, item, ..
            } => {
                self.ensure_started(None, &mut out);
                match item {
                    InputItem::Message(_) => self.open_text(*output_index, 0, &mut out),
                    InputItem::FunctionCall(c) => {
                        self.tool_used = true;
                        self.open_block(
                            OpenKind::Tool,
                            *output_index,
                            0,
                            json!({"type": "tool_use",
                                "id": c.call_id.clone().unwrap_or_default(),
                                "name": c.name.clone().unwrap_or_default(),
                                "input": {}}),
                            &mut out,
                        );
                        if let Some(a) = c.arguments.as_deref().filter(|a| !a.is_empty()) {
                            self.delta(
                                json!({"type": "input_json_delta", "partial_json": a}),
                                &mut out,
                            );
                        }
                    }
                    InputItem::Reasoning(r) => {
                        let redacted =
                            r.extra.get("redacted").and_then(Value::as_bool) == Some(true);
                        if redacted && !super::outbound::is_anthropic_origin(r) {
                            // Never forge redacted_thinking data from a foreign payload.
                            tracing::debug!("dropping foreign redacted reasoning item");
                        } else if redacted {
                            self.open_block(
                                OpenKind::Redacted,
                                *output_index,
                                0,
                                json!({"type": "redacted_thinking",
                                    "data": r.encrypted_content.clone().unwrap_or_default()}),
                                &mut out,
                            );
                        } else {
                            self.open_block(
                                OpenKind::Thinking,
                                *output_index,
                                0,
                                json!({"type": "thinking", "thinking": ""}),
                                &mut out,
                            );
                        }
                    }
                    other => tracing::debug!(
                        ?other,
                        "dropping output item with no anthropic stream equivalent"
                    ),
                }
            }
            StreamEvent::OutputTextDelta {
                output_index,
                content_index,
                delta,
                ..
            } => {
                self.ensure_started(None, &mut out);
                let ci = content_index.unwrap_or(0);
                let matches = matches!(&self.open, Some(o) if o.kind == OpenKind::Text
                    && o.output_index == *output_index && o.content_index == ci);
                if !matches {
                    self.open_text(*output_index, ci, &mut out);
                }
                self.delta(json!({"type": "text_delta", "text": delta}), &mut out);
            }
            StreamEvent::ReasoningSummaryTextDelta {
                output_index,
                delta,
                ..
            } => {
                self.ensure_started(None, &mut out);
                let matches = matches!(&self.open, Some(o) if o.kind == OpenKind::Thinking
                    && o.output_index == *output_index);
                if !matches {
                    self.open_block(
                        OpenKind::Thinking,
                        *output_index,
                        0,
                        json!({"type": "thinking", "thinking": ""}),
                        &mut out,
                    );
                }
                self.delta(
                    json!({"type": "thinking_delta", "thinking": delta}),
                    &mut out,
                );
            }
            StreamEvent::FunctionCallArgumentsDelta {
                output_index,
                delta,
                ..
            } => {
                if matches!(&self.open, Some(o) if o.kind == OpenKind::Tool && o.output_index == *output_index)
                {
                    self.delta(
                        json!({"type": "input_json_delta", "partial_json": delta}),
                        &mut out,
                    );
                }
            }
            StreamEvent::OutputItemDone {
                output_index, item, ..
            } => {
                let current = matches!(&self.open, Some(o) if o.output_index == *output_index);
                if current {
                    let saw = self.open.as_ref().is_some_and(|o| o.saw_delta);
                    match item {
                        InputItem::FunctionCall(c) if !saw => {
                            if let Some(a) = c.arguments.as_deref().filter(|a| !a.is_empty()) {
                                self.delta(
                                    json!({"type": "input_json_delta", "partial_json": a}),
                                    &mut out,
                                );
                            }
                        }
                        InputItem::Reasoning(r)
                            if self
                                .open
                                .as_ref()
                                .is_some_and(|o| o.kind == OpenKind::Thinking) =>
                        {
                            if let Some(sig) = r
                                .encrypted_content
                                .as_ref()
                                .filter(|_| super::outbound::is_anthropic_origin(r))
                            {
                                self.delta(
                                    json!({"type": "signature_delta", "signature": sig}),
                                    &mut out,
                                );
                            }
                        }
                        _ => {}
                    }
                    self.close_open(&mut out);
                }
            }
            StreamEvent::Completed { response, .. } | StreamEvent::Incomplete { response, .. } => {
                self.ensure_started(Some(response), &mut out);
                self.close_open(&mut out);
                let has_call = self.tool_used
                    || response
                        .output
                        .iter()
                        .any(|i| matches!(i, InputItem::FunctionCall(_)));
                let stop_reason = if has_call {
                    "tool_use"
                } else if matches!(ev, StreamEvent::Incomplete { .. }) {
                    "max_tokens"
                } else {
                    "end_turn"
                };
                let u = response.usage();
                let mut usage =
                    json!({"input_tokens": u.input_tokens, "output_tokens": u.output_tokens});
                if u.cached_tokens > 0 {
                    usage["cache_read_input_tokens"] = json!(u.cached_tokens);
                }
                out.push(frame(
                    "message_delta",
                    json!({"type": "message_delta",
                        "delta": {"stop_reason": stop_reason, "stop_sequence": null},
                        "usage": usage}),
                ));
                out.push(frame("message_stop", json!({"type": "message_stop"})));
                self.finished = true;
            }
            StreamEvent::Failed { response, .. } => {
                self.ensure_started(Some(response), &mut out);
                let msg = response
                    .extra
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("upstream response failed")
                    .to_owned();
                self.error(&msg, &mut out);
            }
            StreamEvent::Error { message, .. } => {
                self.ensure_started(None, &mut out);
                let msg = message.clone().unwrap_or_else(|| "upstream error".into());
                self.error(&msg, &mut out);
            }
            StreamEvent::Unknown { event, data } => {
                let Some(name) = event.strip_prefix(EVENT_PREFIX) else {
                    tracing::debug!(event = %event, "dropping stream event with no anthropic equivalent");
                    return out;
                };
                self.ensure_started(None, &mut out);
                if name == "content_block_start" {
                    self.close_open(&mut out);
                    if let Some(i) = data.get("index").and_then(Value::as_u64) {
                        self.next_index = self.next_index.max(i + 1);
                    }
                }
                out.push(frame(name, data.clone()));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ir::{InputItem, StreamEvent};
    use crate::protocol::sse::Decoder;

    fn frames(s: &str) -> Vec<SseFrame> {
        Decoder::default().push(s.as_bytes())
    }

    fn run(s: &str) -> (Vec<StreamEvent>, UpstreamToIr) {
        let mut up = UpstreamToIr::new();
        let mut out = Vec::new();
        for f in frames(s) {
            out.extend(up.on_frame(f).unwrap());
        }
        (out, up)
    }

    const TEXT_TOOL: &str = include_str!("../../../tests/fixtures/anthropic/stream_text_tool.sse");
    const UNKNOWN: &str =
        include_str!("../../../tests/fixtures/anthropic/stream_unknown_block.sse");

    #[test]
    fn stream_text_and_tool_use_to_ir() {
        let (evs, _) = run(TEXT_TOOL);
        assert!(
            matches!(&evs[0], StreamEvent::Created { response, .. } if response.id.as_deref() == Some("msg_1"))
        );
        let text: String = evs
            .iter()
            .filter_map(|e| match e {
                StreamEvent::OutputTextDelta { delta, .. } => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "Hello");
        let args: String = evs
            .iter()
            .filter_map(|e| match e {
                StreamEvent::FunctionCallArgumentsDelta { delta, .. } => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(args, r#"{"a":1}"#);
        // reasoning item done carries the signature as encrypted_content
        assert!(evs.iter().any(|e| matches!(e,
            StreamEvent::OutputItemDone { item: InputItem::Reasoning(r), .. }
            if r.encrypted_content.as_deref() == Some("SIG"))));
        let Some(StreamEvent::Completed { response, .. }) = evs.last() else {
            panic!("last event must be completed: {:?}", evs.last())
        };
        assert_eq!(response.output.len(), 3);
        assert!(
            matches!(&response.output[2], InputItem::FunctionCall(c) if c.call_id.as_deref() == Some("toolu_9"))
        );
    }

    #[test]
    fn stream_usage_extracted() {
        let (_, up) = run(TEXT_TOOL);
        let u = up.usage();
        assert_eq!(u.input_tokens, 25);
        assert_eq!(u.output_tokens, 42);
        assert_eq!(u.cached_tokens, 10);
    }

    #[test]
    fn stream_unknown_block_preserved() {
        let (evs, _) = run(UNKNOWN);
        let unknown: Vec<_> = evs
            .iter()
            .filter(|e| matches!(e, StreamEvent::Unknown { .. }))
            .collect();
        // start+delta+stop of the server_tool_use block, plus the citations_delta
        assert_eq!(unknown.len(), 4);
        // text after it is still translated
        assert!(
            evs.iter()
                .any(|e| matches!(e, StreamEvent::OutputTextDelta { delta, .. } if delta == "ok"))
        );
        // same-protocol re-emission is verbatim
        let mut down = IrToClient::new();
        let sse: String = evs
            .iter()
            .flat_map(|e| down.on_event(e))
            .map(|b| String::from_utf8(b.to_vec()).unwrap())
            .collect();
        assert!(sse.contains(r#""type":"server_tool_use""#));
        assert!(sse.contains(r#""type":"citations_delta""#));
        assert!(sse.contains("web_search_result_location"));
    }

    fn reasoning_stream_sse(tagged: bool, redacted: bool) -> String {
        let mut extra = Map::new();
        if tagged {
            extra.insert(ANTHROPIC_SIGNATURE.into(), Value::Bool(true));
        }
        if redacted {
            extra.insert("redacted".into(), Value::Bool(true));
        }
        let item = InputItem::Reasoning(Reasoning {
            kind: "reasoning".into(),
            id: Some("rs_0".into()),
            summary: Some(vec![json!({"type": "summary_text", "text": "hmm"})]),
            encrypted_content: Some("ENC".into()),
            extra,
        });
        let mut down = IrToClient::new();
        let mut out = String::new();
        for ev in [
            StreamEvent::OutputItemAdded {
                output_index: 0,
                item: item.clone(),
                extra: Map::new(),
            },
            StreamEvent::OutputItemDone {
                output_index: 0,
                item,
                extra: Map::new(),
            },
        ] {
            for b in down.on_event(&ev) {
                out.push_str(std::str::from_utf8(&b).unwrap());
            }
        }
        out
    }

    #[test]
    fn client_stream_never_forges_signature() {
        let foreign = reasoning_stream_sse(false, false);
        assert!(!foreign.contains("signature") && !foreign.contains("ENC"));
        let foreign_redacted = reasoning_stream_sse(false, true);
        assert!(
            !foreign_redacted.contains("redacted_thinking") && !foreign_redacted.contains("ENC")
        );
        let own = reasoning_stream_sse(true, false);
        assert!(own.contains(r#""type":"signature_delta""#) && own.contains("ENC"));
        let own_redacted = reasoning_stream_sse(true, true);
        assert!(own_redacted.contains("redacted_thinking") && own_redacted.contains("ENC"));
    }

    #[test]
    fn ir_events_to_anthropic_sse_order() {
        let (evs, _) = run(TEXT_TOOL);
        let mut down = IrToClient::new();
        let out: Vec<String> = evs
            .iter()
            .flat_map(|e| down.on_event(e))
            .map(|b| String::from_utf8(b.to_vec()).unwrap())
            .collect();
        let names: Vec<String> = out
            .iter()
            .map(|s| {
                s.lines()
                    .next()
                    .unwrap()
                    .trim_start_matches("event: ")
                    .to_owned()
            })
            .collect();
        assert_eq!(names.first().map(String::as_str), Some("message_start"));
        assert_eq!(
            &names[names.len() - 2..],
            ["message_delta".to_owned(), "message_stop".to_owned()]
        );
        // every start is followed by stop before the next start
        let mut open = false;
        for n in &names {
            match n.as_str() {
                "content_block_start" => {
                    assert!(!open);
                    open = true;
                }
                "content_block_stop" => {
                    assert!(open);
                    open = false;
                }
                "content_block_delta" => assert!(open),
                _ => assert!(!open, "{n} inside open block"),
            }
        }
        let joined = out.join("");
        assert!(joined.contains(r#""type":"thinking_delta""#));
        assert!(joined.contains(r#""type":"signature_delta""#));
        assert!(joined.contains(r#""stop_reason":"tool_use""#));
        assert!(joined.contains(r#""partial_json":"{\"a\"""#));
        // block indices are sequential 0,1,2
        for i in 0..3 {
            assert!(
                joined.contains(&format!(r#""type":"content_block_start","index":{i}"#))
                    || joined.contains(&format!(r#""index":{i},"type":"content_block_start""#)),
                "index {i}"
            );
        }
    }

    #[test]
    fn responses_sourced_events_produce_valid_anthropic_stream() {
        use serde_json::json;
        let ev = |v: serde_json::Value| {
            StreamEvent::from_frame(&SseFrame {
                event: None,
                data: v.to_string(),
            })
            .unwrap()
        };
        let events = [
            ev(json!({"type":"response.created","response":{"id":"resp_1","model":"gpt"}})),
            ev(
                json!({"type":"response.output_item.added","output_index":0,"item":{"type":"message","role":"assistant","content":[]}}),
            ),
            ev(
                json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"yo"}),
            ),
            ev(
                json!({"type":"response.output_item.done","output_index":0,"item":{"type":"message","role":"assistant","content":[]}}),
            ),
            ev(
                json!({"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","call_id":"c1","name":"f","arguments":""}}),
            ),
            ev(
                json!({"type":"response.function_call_arguments.delta","output_index":1,"delta":"{}"}),
            ),
            ev(
                json!({"type":"response.completed","response":{"id":"resp_1","usage":{"input_tokens":3,"output_tokens":4}}}),
            ),
        ];
        let mut down = IrToClient::new();
        let out: String = events
            .iter()
            .flat_map(|e| down.on_event(e))
            .map(|b| String::from_utf8(b.to_vec()).unwrap())
            .collect();
        assert!(out.starts_with("event: message_start"));
        assert!(out.contains(r#""type":"tool_use""#));
        assert!(out.contains(r#""stop_reason":"tool_use""#));
        assert!(out.contains(r#""output_tokens":4"#));
        assert!(out.trim_end().ends_with(r#"{"type":"message_stop"}"#));
    }
}
