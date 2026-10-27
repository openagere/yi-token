//! Aggregates Responses stream events into a non-streaming [`Response`].

use crate::protocol::ir::{Content, ContentPart, InputItem, Response, StreamEvent, TextPart};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub struct Collector {
    response: Option<Response>,
    items: BTreeMap<u64, InputItem>,
}

impl Collector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, event: &StreamEvent) {
        match event {
            StreamEvent::Created { response, .. } => self.response = Some(response.clone()),
            StreamEvent::Completed { response, .. }
            | StreamEvent::Incomplete { response, .. }
            | StreamEvent::Failed { response, .. } => self.response = Some(response.clone()),
            StreamEvent::OutputItemAdded {
                output_index, item, ..
            }
            | StreamEvent::OutputItemDone {
                output_index, item, ..
            } => {
                self.items.insert(*output_index, item.clone());
            }
            StreamEvent::OutputTextDelta {
                output_index,
                content_index,
                delta,
                ..
            } => {
                if let Some(InputItem::Message(m)) = self.items.get_mut(output_index) {
                    append_text(&mut m.content, content_index.unwrap_or(0) as usize, delta);
                }
            }
            StreamEvent::FunctionCallArgumentsDelta {
                output_index,
                delta,
                ..
            } => {
                if let Some(InputItem::FunctionCall(c)) = self.items.get_mut(output_index) {
                    c.arguments.get_or_insert_with(String::new).push_str(delta);
                }
            }
            StreamEvent::ReasoningSummaryTextDelta {
                output_index,
                summary_index,
                delta,
                ..
            } => {
                if let Some(InputItem::Reasoning(r)) = self.items.get_mut(output_index) {
                    let summary = r.summary.get_or_insert_with(Vec::new);
                    let idx = summary_index.unwrap_or(0) as usize;
                    while summary.len() <= idx {
                        summary.push(serde_json::json!({"type": "summary_text", "text": ""}));
                    }
                    if let Value::Object(o) = &mut summary[idx] {
                        let text = o.get("text").and_then(Value::as_str).unwrap_or("");
                        o.insert("text".into(), Value::String(format!("{text}{delta}")));
                    }
                }
            }
            StreamEvent::Error { .. } | StreamEvent::Unknown { .. } => {}
        }
    }

    /// The aggregated response, or `None` if the stream carried no response envelope.
    pub fn finish(self) -> Option<Response> {
        let mut response = self.response?;
        if response.output.is_empty() {
            response.output = self.items.into_values().collect();
        }
        Some(response)
    }
}

fn append_text(content: &mut Content, index: usize, delta: &str) {
    match content {
        Content::Text(s) => s.push_str(delta),
        Content::Parts(parts) => {
            while parts.len() <= index {
                parts.push(ContentPart::OutputText(TextPart {
                    kind: "output_text".into(),
                    text: String::new(),
                    extra: Map::new(),
                }));
            }
            if let ContentPart::InputText(p) | ContentPart::OutputText(p) = &mut parts[index] {
                p.text.push_str(delta);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::sse::Decoder;
    use serde_json::json;

    fn events(sse: &str) -> Vec<StreamEvent> {
        Decoder::default()
            .push(sse.as_bytes())
            .iter()
            .filter_map(StreamEvent::from_frame)
            .collect()
    }

    #[test]
    fn collector_builds_response_from_events() {
        let sse = concat!(
            "event: response.created\n",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\",\"model\":\"m\",\"status\":\"in_progress\"}}\n\n",
            "event: response.output_item.added\n",
            "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"content\":[]}}\n\n",
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"content_index\":0,\"delta\":\"Hel\"}\n\n",
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"content_index\":0,\"delta\":\"lo\"}\n\n",
            "event: response.output_item.added\n",
            "data: {\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"type\":\"function_call\",\"call_id\":\"c1\",\"name\":\"f\",\"arguments\":\"\"}}\n\n",
            "event: response.function_call_arguments.delta\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":1,\"delta\":\"{\\\"a\\\"\"}\n\n",
            "event: response.function_call_arguments.delta\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":1,\"delta\":\":1}\"}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"model\":\"m\",\"status\":\"completed\",\"usage\":{\"input_tokens\":3,\"output_tokens\":4}}}\n\n",
            "data: [DONE]\n\n",
        );
        let mut c = Collector::new();
        for e in events(sse) {
            c.push(&e);
        }
        let r = c.finish().unwrap();
        assert_eq!(r.status.as_deref(), Some("completed"));
        assert_eq!(r.usage().output_tokens, 4);
        assert_eq!(
            serde_json::to_value(&r.output).unwrap(),
            json!([
                {"type":"message","id":"msg_1","role":"assistant",
                 "content":[{"type":"output_text","text":"Hello"}]},
                {"type":"function_call","call_id":"c1","name":"f","arguments":"{\"a\":1}"}
            ])
        );
    }

    fn sse_of(events: &[Value]) -> String {
        events
            .iter()
            .map(|e| format!("event: {}\ndata: {}\n\n", e["type"].as_str().unwrap(), e))
            .collect()
    }

    #[test]
    fn collector_handles_full_realistic_sequence() {
        let msg = json!({"type":"message","id":"msg_1","role":"assistant",
            "content":[{"type":"output_text","text":"Hi there","annotations":[]}]});
        let call =
            json!({"type":"function_call","call_id":"c1","name":"f","arguments":"{\"a\":1}"});
        let started = json!({"type":"message","id":"msg_1","role":"assistant","content":[]});
        let seq = [
            json!({"type":"response.created","response":{"id":"r1","status":"in_progress","output":[]}}),
            json!({"type":"response.output_item.added","output_index":0,"item":started}),
            json!({"type":"response.content_part.added","output_index":0,"content_index":0,
                "part":{"type":"output_text","text":""}}),
            json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"Hi "}),
            json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"there"}),
            json!({"type":"response.output_text.done","output_index":0,"content_index":0,"text":"Hi there"}),
            json!({"type":"response.content_part.done","output_index":0,"content_index":0,
                "part":{"type":"output_text","text":"Hi there"}}),
            json!({"type":"response.output_item.done","output_index":0,"item":msg}),
            json!({"type":"response.output_item.added","output_index":1,"item":call}),
            json!({"type":"response.output_item.done","output_index":1,"item":call}),
            json!({"type":"response.completed","response":{"id":"r1","status":"completed",
                "output":[msg, call]}}),
        ];
        let sse = sse_of(&seq);

        // Completed envelope carries output: it is used as-is.
        let mut c = Collector::new();
        for e in events(&sse) {
            c.push(&e);
        }
        let r = c.finish().unwrap();
        assert_eq!(serde_json::to_value(&r.output).unwrap(), json!([msg, call]));

        // Envelope without output: items path assembles the same result.
        let mut seq = seq.to_vec();
        seq[10] = json!({"type":"response.completed","response":{"id":"r1","status":"completed"}});
        let mut c = Collector::new();
        for e in events(&sse_of(&seq)) {
            c.push(&e);
        }
        let r = c.finish().unwrap();
        assert_eq!(serde_json::to_value(&r.output).unwrap(), json!([msg, call]));
    }

    #[test]
    fn collector_without_envelope_yields_none() {
        assert!(Collector::new().finish().is_none());
    }
}
