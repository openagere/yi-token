//! Chat Completions SSE <-> IR stream events.

use super::inbound::{incomplete_details, is_incomplete, usage_value};
use super::outbound::{chat_usage, incomplete_finish_reason, now_secs};
use super::types::{Chunk, parse_usage};
use super::{ANTHROPIC_EVENT_PREFIX, REASONING_CONTENT};
use crate::error::CoreResult;
use crate::protocol::Usage;
use crate::protocol::ir::{
    self, Content, ContentPart, FunctionCall, InputItem, Message, Reasoning, StreamEvent, TextPart,
};
use crate::protocol::sse::{self, SseFrame};
use bytes::Bytes;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

// ------------------------------------------------------- upstream -> IR

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

fn reasoning_item(output: u64, text: &str) -> InputItem {
    let mut extra = Map::new();
    if !text.is_empty() {
        extra.insert(REASONING_CONTENT.into(), Value::String(text.to_owned()));
    }
    InputItem::Reasoning(Reasoning {
        kind: "reasoning".into(),
        id: Some(format!("rs_{output}")),
        summary: Some(if text.is_empty() {
            vec![]
        } else {
            vec![json!({"type": "summary_text", "text": text})]
        }),
        encrypted_content: None,
        extra,
    })
}

fn call_item(t: &ToolSlot) -> InputItem {
    InputItem::FunctionCall(FunctionCall {
        kind: "function_call".into(),
        call_id: Some(t.call_id.clone()),
        name: Some(t.name.clone()),
        arguments: Some(t.args.clone()),
        extra: Map::new(),
    })
}

#[derive(Debug)]
struct ToolSlot {
    output: u64,
    call_id: String,
    name: String,
    args: String,
}

/// Chat Completions upstream SSE -> IR (Responses) events.
///
/// Completion is deferred until `[DONE]` (or [`UpstreamToIr::on_end`]) because
/// `stream_options.include_usage` sends the usage-only chunk *after* the chunk
/// carrying `finish_reason`.
#[derive(Debug, Default)]
pub struct UpstreamToIr {
    created: bool,
    completed: bool,
    id: String,
    model: String,
    next_output: u64,
    reasoning: Option<(u64, String)>,
    text: Option<(u64, String)>,
    /// Keyed by the upstream tool-call `index`.
    tools: BTreeMap<u64, ToolSlot>,
    done: BTreeMap<u64, InputItem>,
    finish: Option<String>,
    usage: Usage,
}

impl UpstreamToIr {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn usage(&self) -> Usage {
        self.usage
    }

    fn alloc(&mut self) -> u64 {
        let n = self.next_output;
        self.next_output += 1;
        n
    }

    fn response(&self, status: &str) -> ir::Response {
        let mut extra = Map::new();
        if status == "incomplete" {
            extra.insert(
                "incomplete_details".into(),
                incomplete_details(self.finish.as_deref()),
            );
        }
        ir::Response {
            id: Some(self.id.clone()),
            object: Some("response".into()),
            created_at: None,
            model: Some(self.model.clone()),
            status: Some(status.into()),
            output: self.done.values().cloned().collect(),
            usage: (status != "in_progress").then(|| usage_value(self.usage)),
            extra,
        }
    }

    /// Close every open item, in output order.
    fn close_all(&mut self, out: &mut Vec<StreamEvent>) {
        let mut items: Vec<(u64, InputItem)> = Vec::new();
        if let Some((o, t)) = self.reasoning.take() {
            items.push((o, reasoning_item(o, &t)));
        }
        if let Some((o, t)) = self.text.take() {
            items.push((o, message_item(&t)));
        }
        items.extend(
            std::mem::take(&mut self.tools)
                .into_values()
                .map(|t| (t.output, call_item(&t))),
        );
        items.sort_by_key(|(o, _)| *o);
        for (output_index, item) in items {
            self.done.insert(output_index, item.clone());
            out.push(StreamEvent::OutputItemDone {
                output_index,
                item,
                extra: Map::new(),
            });
        }
    }

    fn terminal(&mut self) -> Vec<StreamEvent> {
        let mut out = Vec::new();
        if self.completed {
            return out;
        }
        self.completed = true;
        self.close_all(&mut out);
        let incomplete = is_incomplete(self.finish.as_deref());
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
        out
    }

    pub fn on_frame(&mut self, frame: SseFrame) -> CoreResult<Vec<StreamEvent>> {
        if frame.data.trim() == "[DONE]" {
            if !self.created && self.finish.is_none() {
                return Ok(vec![]);
            }
            return Ok(self.terminal());
        }
        let Ok(data) = serde_json::from_str::<Value>(&frame.data) else {
            tracing::debug!("skipping non-JSON chat SSE frame");
            return Ok(vec![]);
        };
        if let Some(err) = data.get("error").filter(|e| !e.is_null()) {
            self.completed = true;
            return Ok(vec![StreamEvent::Error {
                message: err
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                extra: match data {
                    Value::Object(m) => m,
                    _ => Map::new(),
                },
            }]);
        }
        let chunk: Chunk = match serde_json::from_value(data) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "skipping malformed chat SSE chunk");
                return Ok(vec![]);
            }
        };
        let mut out = Vec::new();
        if let Some(id) = chunk.id.filter(|i| !i.is_empty())
            && self.id.is_empty()
        {
            self.id = id;
        }
        if let Some(m) = chunk.model.filter(|m| !m.is_empty())
            && self.model.is_empty()
        {
            self.model = m;
        }
        if !self.created {
            self.created = true;
            out.push(StreamEvent::Created {
                response: self.response("in_progress"),
                extra: Map::new(),
            });
        }
        if let Some(u) = chunk.usage.as_ref().filter(|u| u.is_object()) {
            self.usage = parse_usage(u);
        }
        for choice in chunk.choices {
            if choice.index != 0 {
                tracing::debug!(index = choice.index, "ignoring non-primary chat choice");
                continue;
            }
            let d = choice.delta;
            if let Some(r) = d
                .reasoning_content
                .or(d.reasoning)
                .filter(|r| !r.is_empty())
            {
                if self.reasoning.is_none() {
                    let o = self.alloc();
                    self.reasoning = Some((o, String::new()));
                    out.push(StreamEvent::OutputItemAdded {
                        output_index: o,
                        item: reasoning_item(o, ""),
                        extra: Map::new(),
                    });
                }
                let (o, buf) = self.reasoning.as_mut().expect("set above");
                buf.push_str(&r);
                out.push(StreamEvent::ReasoningSummaryTextDelta {
                    output_index: *o,
                    summary_index: Some(0),
                    delta: r,
                    extra: Map::new(),
                });
            }
            if let Some(t) = d.content.filter(|t| !t.is_empty()) {
                if self.text.is_none() {
                    let o = self.alloc();
                    self.text = Some((o, String::new()));
                    out.push(StreamEvent::OutputItemAdded {
                        output_index: o,
                        item: message_item(""),
                        extra: Map::new(),
                    });
                }
                let (o, buf) = self.text.as_mut().expect("set above");
                buf.push_str(&t);
                out.push(StreamEvent::OutputTextDelta {
                    output_index: *o,
                    content_index: Some(0),
                    delta: t,
                    extra: Map::new(),
                });
            }
            for tc in d.tool_calls.unwrap_or_default() {
                if !self.tools.contains_key(&tc.index) {
                    let output = self.alloc();
                    let slot = ToolSlot {
                        output,
                        call_id: tc
                            .id
                            .clone()
                            .unwrap_or_else(|| format!("call_{}", tc.index)),
                        name: tc
                            .function
                            .as_ref()
                            .and_then(|f| f.name.clone())
                            .unwrap_or_default(),
                        args: String::new(),
                    };
                    out.push(StreamEvent::OutputItemAdded {
                        output_index: output,
                        item: call_item(&slot),
                        extra: Map::new(),
                    });
                    self.tools.insert(tc.index, slot);
                }
                let slot = self.tools.get_mut(&tc.index).expect("inserted above");
                if let Some(name) = tc
                    .function
                    .as_ref()
                    .and_then(|f| f.name.as_deref())
                    .filter(|n| !n.is_empty() && slot.name.is_empty())
                {
                    slot.name = name.to_owned();
                }
                if let Some(args) = tc
                    .function
                    .and_then(|f| f.arguments)
                    .filter(|a| !a.is_empty())
                {
                    slot.args.push_str(&args);
                    out.push(StreamEvent::FunctionCallArgumentsDelta {
                        output_index: slot.output,
                        delta: args,
                        extra: Map::new(),
                    });
                }
            }
            if let Some(reason) = choice.finish_reason {
                self.finish = Some(reason);
                self.close_all(&mut out);
            }
        }
        Ok(out)
    }

    /// The upstream body ended without `[DONE]`: complete if a finish reason
    /// was seen, otherwise report the truncation.
    pub fn on_end(&mut self) -> Vec<StreamEvent> {
        if self.completed || !self.created {
            return vec![];
        }
        if self.finish.is_some() {
            return self.terminal();
        }
        self.completed = true;
        vec![StreamEvent::Error {
            message: Some("upstream stream ended unexpectedly".into()),
            extra: Map::new(),
        }]
    }
}

// ------------------------------------------------------- IR -> client

/// IR (Responses) events -> Chat Completions client SSE.
#[derive(Debug, Default)]
pub struct IrToClient {
    include_usage: bool,
    started: bool,
    finished: bool,
    id: String,
    model: String,
    created: i64,
    /// IR output_index -> Chat tool_calls index.
    tool_index: HashMap<u64, u64>,
    /// Output items that already produced a delta.
    seen: HashSet<u64>,
}

fn data(v: Value) -> Bytes {
    sse::encode(None, &v.to_string())
}

impl IrToClient {
    pub fn new() -> Self {
        Self::default()
    }

    /// `include_usage` mirrors the client's own `stream_options.include_usage`.
    pub fn with_usage(include_usage: bool) -> Self {
        Self {
            include_usage,
            ..Self::default()
        }
    }

    fn chunk(&self, delta: Value, finish_reason: Option<&str>) -> Bytes {
        data(json!({
            "id": self.id,
            "object": "chat.completion.chunk",
            "created": self.created,
            "model": self.model,
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason}],
        }))
    }

    fn ensure_started(&mut self, response: Option<&ir::Response>, out: &mut Vec<Bytes>) {
        if self.started {
            return;
        }
        self.started = true;
        self.id = response
            .and_then(|r| r.id.clone())
            .unwrap_or_else(|| "chatcmpl-yi".into());
        self.model = response.and_then(|r| r.model.clone()).unwrap_or_default();
        self.created = response.and_then(|r| r.created_at).unwrap_or_else(now_secs);
        out.push(self.chunk(json!({"role": "assistant", "content": ""}), None));
    }

    fn error(&mut self, message: &str, out: &mut Vec<Bytes>) {
        out.push(data(
            json!({"error": {"message": message, "type": "api_error"}}),
        ));
        out.push(sse::encode(None, "[DONE]"));
        self.finished = true;
    }

    fn tool_delta(&self, index: u64, function: Value, call_id: Option<&str>) -> Bytes {
        let mut call = json!({"index": index, "function": function});
        if let Some(id) = call_id {
            call["id"] = json!(id);
            call["type"] = json!("function");
        }
        self.chunk(json!({"tool_calls": [call]}), None)
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
            } => match item {
                InputItem::FunctionCall(c) => {
                    self.ensure_started(None, &mut out);
                    let idx = self.tool_index.len() as u64;
                    self.tool_index.insert(*output_index, idx);
                    let args = c.arguments.clone().unwrap_or_default();
                    if !args.is_empty() {
                        self.seen.insert(*output_index);
                    }
                    out.push(self.tool_delta(
                        idx,
                        json!({"name": c.name.clone().unwrap_or_default(), "arguments": args}),
                        Some(c.call_id.as_deref().unwrap_or_default()),
                    ));
                }
                InputItem::Message(_) | InputItem::Reasoning(_) => {
                    self.ensure_started(None, &mut out);
                }
                other => tracing::debug!(
                    ?other,
                    "dropping output item with no chat stream equivalent"
                ),
            },
            StreamEvent::OutputTextDelta {
                output_index,
                delta,
                ..
            } => {
                self.ensure_started(None, &mut out);
                self.seen.insert(*output_index);
                out.push(self.chunk(json!({"content": delta}), None));
            }
            StreamEvent::ReasoningSummaryTextDelta {
                output_index,
                delta,
                ..
            } => {
                self.ensure_started(None, &mut out);
                self.seen.insert(*output_index);
                out.push(self.chunk(json!({REASONING_CONTENT: delta}), None));
            }
            StreamEvent::FunctionCallArgumentsDelta {
                output_index,
                delta,
                ..
            } => {
                if let Some(idx) = self.tool_index.get(output_index) {
                    self.seen.insert(*output_index);
                    out.push(self.tool_delta(*idx, json!({"arguments": delta}), None));
                }
            }
            StreamEvent::OutputItemDone {
                output_index, item, ..
            } => {
                // Upstreams that only send `done` events still deliver their content.
                if self.seen.contains(output_index) {
                    return out;
                }
                match item {
                    InputItem::Message(m) => {
                        let text: String = match &m.content {
                            Content::Text(t) => t.clone(),
                            Content::Parts(ps) => ps
                                .iter()
                                .filter_map(|p| match p {
                                    ContentPart::OutputText(t) | ContentPart::InputText(t) => {
                                        Some(t.text.as_str())
                                    }
                                    _ => None,
                                })
                                .collect(),
                        };
                        if !text.is_empty() {
                            self.ensure_started(None, &mut out);
                            out.push(self.chunk(json!({"content": text}), None));
                        }
                    }
                    InputItem::Reasoning(r) => {
                        let text = r
                            .extra
                            .get(REASONING_CONTENT)
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            .unwrap_or_else(|| {
                                r.summary
                                    .iter()
                                    .flatten()
                                    .filter_map(|s| s.get("text").and_then(Value::as_str))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            });
                        if !text.is_empty() {
                            self.ensure_started(None, &mut out);
                            out.push(self.chunk(json!({REASONING_CONTENT: text}), None));
                        }
                    }
                    InputItem::FunctionCall(c) => {
                        if let (Some(idx), Some(a)) = (
                            self.tool_index.get(output_index),
                            c.arguments.as_deref().filter(|a| !a.is_empty()),
                        ) {
                            out.push(self.tool_delta(*idx, json!({"arguments": a}), None));
                        }
                    }
                    _ => {}
                }
            }
            StreamEvent::Completed { response, .. } | StreamEvent::Incomplete { response, .. } => {
                self.ensure_started(Some(response), &mut out);
                let has_call = !self.tool_index.is_empty()
                    || response
                        .output
                        .iter()
                        .any(|i| matches!(i, InputItem::FunctionCall(_)));
                let finish = if has_call {
                    "tool_calls"
                } else if matches!(ev, StreamEvent::Incomplete { .. }) {
                    incomplete_finish_reason(response)
                } else {
                    "stop"
                };
                out.push(self.chunk(json!({}), Some(finish)));
                if self.include_usage {
                    out.push(data(json!({
                        "id": self.id,
                        "object": "chat.completion.chunk",
                        "created": self.created,
                        "model": self.model,
                        "choices": [],
                        "usage": chat_usage(response.usage()),
                    })));
                }
                out.push(sse::encode(None, "[DONE]"));
                self.finished = true;
            }
            StreamEvent::Failed { response, .. } => {
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
                let msg = message.clone().unwrap_or_else(|| "upstream error".into());
                self.error(&msg, &mut out);
            }
            StreamEvent::Unknown { event, .. } => {
                // Foreign events (anthropic.* and anything else) have no Chat form.
                if event.starts_with(ANTHROPIC_EVENT_PREFIX) {
                    tracing::debug!(event = %event, "dropping foreign anthropic stream event");
                } else {
                    tracing::debug!(event = %event, "dropping stream event with no chat equivalent");
                }
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
    use serde_json::{Value, json};

    fn run(s: &str) -> (Vec<StreamEvent>, UpstreamToIr) {
        let mut up = UpstreamToIr::new();
        let mut out = Vec::new();
        for f in Decoder::default().push(s.as_bytes()) {
            out.extend(up.on_frame(f).unwrap());
        }
        (out, up)
    }

    fn chunk(v: Value) -> String {
        format!("data: {v}\n\n")
    }

    fn tool_stream() -> String {
        [
            chunk(json!({"id":"c1","model":"m","choices":[{"index":0,"delta":{"role":"assistant","content":""}}]})),
            chunk(json!({"id":"c1","choices":[{"index":0,"delta":{"reasoning_content":"hm"}}]})),
            chunk(json!({"id":"c1","choices":[{"index":0,"delta":{"content":"Hel"}}]})),
            chunk(json!({"id":"c1","choices":[{"index":0,"delta":{"content":"lo"}}]})),
            chunk(json!({"id":"c1","choices":[{"index":0,"delta":{"tool_calls":[
                {"index":0,"id":"call_a","type":"function","function":{"name":"f","arguments":""}}]}}]})),
            chunk(json!({"id":"c1","choices":[{"index":0,"delta":{"tool_calls":[
                {"index":0,"function":{"arguments":"{\"a\""}}]}}]})),
            chunk(json!({"id":"c1","choices":[{"index":0,"delta":{"tool_calls":[
                {"index":1,"id":"call_b","type":"function","function":{"name":"g","arguments":"{}"}},
                {"index":0,"function":{"arguments":":1}"}}]}}]})),
            chunk(json!({"id":"c1","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":null})),
            chunk(json!({"id":"c1","choices":[],"usage":{"prompt_tokens":11,"completion_tokens":6,
                "prompt_tokens_details":{"cached_tokens":4},"completion_tokens_details":{"reasoning_tokens":2}}})),
            "data: [DONE]\n\n".to_owned(),
        ]
        .concat()
    }

    #[test]
    fn stream_tool_call_deltas_assembled() {
        let (evs, _) = run(&tool_stream());
        assert!(matches!(&evs[0], StreamEvent::Created { response, .. }
            if response.id.as_deref() == Some("c1") && response.model.as_deref() == Some("m")));
        let text: String = evs
            .iter()
            .filter_map(|e| match e {
                StreamEvent::OutputTextDelta { delta, .. } => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "Hello");
        let Some(StreamEvent::Completed { response, .. }) = evs.last() else {
            panic!("last must be completed: {:?}", evs.last())
        };
        // reasoning, message, call_a, call_b
        assert_eq!(response.output.len(), 4);
        assert!(matches!(&response.output[0], InputItem::Reasoning(r)
            if r.extra["reasoning_content"] == "hm"));
        assert!(matches!(&response.output[2], InputItem::FunctionCall(c)
            if c.call_id.as_deref() == Some("call_a") && c.name.as_deref() == Some("f")
            && c.arguments.as_deref() == Some("{\"a\":1}")));
        assert!(matches!(&response.output[3], InputItem::FunctionCall(c)
            if c.call_id.as_deref() == Some("call_b") && c.arguments.as_deref() == Some("{}")));
        // every added output item is also done
        let added = evs
            .iter()
            .filter(|e| matches!(e, StreamEvent::OutputItemAdded { .. }))
            .count();
        let done = evs
            .iter()
            .filter(|e| matches!(e, StreamEvent::OutputItemDone { .. }))
            .count();
        assert_eq!((added, done), (4, 4));
        let args: String = evs
            .iter()
            .filter_map(|e| match e {
                StreamEvent::FunctionCallArgumentsDelta {
                    output_index: 2,
                    delta,
                    ..
                } => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(args, "{\"a\":1}");
    }

    #[test]
    fn stream_usage_from_final_chunk() {
        let (evs, up) = run(&tool_stream());
        let u = up.usage();
        assert_eq!(
            (
                u.input_tokens,
                u.output_tokens,
                u.cached_tokens,
                u.reasoning_tokens
            ),
            (11, 6, 4, 2)
        );
        // completion waits for the usage-only chunk after finish_reason
        let Some(StreamEvent::Completed { response, .. }) = evs.last() else {
            panic!("completed")
        };
        assert_eq!(response.usage().input_tokens, 11);
        assert_eq!(
            evs.iter()
                .filter(|e| matches!(e, StreamEvent::Completed { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn length_finish_is_incomplete_and_end_without_done_completes() {
        let s = [
            chunk(json!({"id":"c","choices":[{"index":0,"delta":{"content":"x"},"finish_reason":"length"}]})),
            chunk(json!({"id":"c","choices":[],"usage":{"prompt_tokens":1,"completion_tokens":2}})),
        ]
        .concat();
        let (mut evs, mut up) = run(&s);
        assert!(
            !evs.iter()
                .any(|e| matches!(e, StreamEvent::Incomplete { .. }))
        );
        evs.extend(up.on_end());
        assert!(
            matches!(evs.last(), Some(StreamEvent::Incomplete { response, .. })
            if response.usage().output_tokens == 2)
        );
        assert!(up.on_end().is_empty());
    }

    #[test]
    fn truncated_stream_without_finish_is_error() {
        let (mut evs, mut up) = run(&chunk(
            json!({"id":"c","choices":[{"index":0,"delta":{"content":"x"}}]}),
        ));
        evs.extend(up.on_end());
        assert!(matches!(evs.last(), Some(StreamEvent::Error { .. })));
    }

    #[test]
    fn upstream_error_frame_becomes_error_event() {
        let (evs, _) = run(&chunk(json!({"error":{"message":"boom","type":"x"}})));
        assert!(
            matches!(&evs[0], StreamEvent::Error { message, .. } if message.as_deref() == Some("boom"))
        );
    }

    fn to_client(evs: &[StreamEvent], include_usage: bool) -> Vec<String> {
        let mut down = IrToClient::with_usage(include_usage);
        evs.iter()
            .flat_map(|e| down.on_event(e))
            .map(|b| String::from_utf8(b.to_vec()).unwrap())
            .collect()
    }

    fn data_json(frame: &str) -> Value {
        serde_json::from_str(frame.strip_prefix("data: ").unwrap().trim()).unwrap()
    }

    #[test]
    fn ir_events_to_chat_sse_with_done() {
        let (evs, _) = run(&tool_stream());
        let out = to_client(&evs, false);
        assert_eq!(out.last().map(String::as_str), Some("data: [DONE]\n\n"));
        assert!(
            out.iter()
                .all(|f| f.starts_with("data: ") && !f.contains("event:"))
        );
        let chunks: Vec<Value> = out[..out.len() - 1].iter().map(|f| data_json(f)).collect();
        assert!(
            chunks
                .iter()
                .all(|c| c["object"] == "chat.completion.chunk" && c["id"] == "c1")
        );
        assert_eq!(chunks[0]["choices"][0]["delta"]["role"], "assistant");
        let text: String = chunks
            .iter()
            .filter_map(|c| c["choices"][0]["delta"]["content"].as_str())
            .collect();
        assert_eq!(text, "Hello");
        let reasoning: String = chunks
            .iter()
            .filter_map(|c| c["choices"][0]["delta"]["reasoning_content"].as_str())
            .collect();
        assert_eq!(reasoning, "hm");
        // tool call starts carry id/name with sequential tool indices
        let starts: Vec<&Value> = chunks
            .iter()
            .filter_map(|c| c["choices"][0]["delta"]["tool_calls"].get(0))
            .filter(|t| t.get("id").is_some())
            .collect();
        assert_eq!(starts.len(), 2);
        assert_eq!(
            (starts[0]["index"].as_u64(), starts[1]["index"].as_u64()),
            (Some(0), Some(1))
        );
        assert_eq!(starts[0]["function"]["name"], "f");
        let args: String = chunks
            .iter()
            .filter_map(|c| c["choices"][0]["delta"]["tool_calls"].get(0))
            .filter(|t| t["index"] == 0)
            .filter_map(|t| t["function"]["arguments"].as_str())
            .collect();
        assert_eq!(args, "{\"a\":1}");
        let last = chunks.last().unwrap();
        assert_eq!(last["choices"][0]["finish_reason"], "tool_calls");
        // no usage chunk unless the client asked for one
        assert!(chunks.iter().all(|c| c.get("usage").is_none()));
    }

    #[test]
    fn usage_chunk_only_when_requested() {
        let (evs, _) = run(&tool_stream());
        let out = to_client(&evs, true);
        assert_eq!(out.last().map(String::as_str), Some("data: [DONE]\n\n"));
        let usage = data_json(&out[out.len() - 2]);
        assert_eq!(usage["choices"], json!([]));
        assert_eq!(usage["usage"]["prompt_tokens"], 11);
        assert_eq!(usage["usage"]["completion_tokens"], 6);
        assert_eq!(usage["usage"]["total_tokens"], 17);
    }

    #[test]
    fn responses_sourced_events_produce_valid_chat_stream() {
        let ev = |v: Value| {
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
                json!({"type":"response.output_item.done","output_index":1,"item":{"type":"function_call","call_id":"c1","name":"f","arguments":"{}"}}),
            ),
            ev(
                json!({"type":"response.completed","response":{"id":"resp_1","usage":{"input_tokens":3,"output_tokens":4}}}),
            ),
        ];
        let out = to_client(&events, false);
        let joined = out.join("");
        assert!(joined.contains(r#""content":"yo""#));
        // arguments only present on the done event are still delivered
        assert!(joined.contains(r#""arguments":"{}""#));
        assert!(joined.contains(r#""finish_reason":"tool_calls""#));
        assert!(joined.ends_with("data: [DONE]\n\n"));
    }

    #[test]
    fn failed_response_emits_error_then_done() {
        let out = to_client(
            &[StreamEvent::Error {
                message: Some("bad".into()),
                extra: Default::default(),
            }],
            false,
        );
        assert!(out[0].contains(r#""message":"bad""#));
        assert_eq!(out.last().map(String::as_str), Some("data: [DONE]\n\n"));
    }

    #[test]
    fn foreign_anthropic_events_are_dropped() {
        let evs = [
            StreamEvent::Unknown {
                event: "anthropic.content_block_start".into(),
                data: json!({"type":"content_block_start","index":1,
                    "content_block":{"type":"server_tool_use","id":"srv"}}),
            },
            StreamEvent::Unknown {
                event: "anthropic.content_block_delta".into(),
                data: json!({"type":"content_block_delta","delta":{"type":"citations_delta"}}),
            },
            StreamEvent::Unknown {
                event: "response.future".into(),
                data: json!({"a":1}),
            },
            StreamEvent::OutputItemAdded {
                output_index: 3,
                item: serde_json::from_value(
                    json!({"type":"anthropic.unknown_block","block":{"type":"x"}}),
                )
                .unwrap(),
                extra: Default::default(),
            },
        ];
        assert!(to_client(&evs, false).is_empty());
    }
}
