//! Wire protocol representations and conversions.

use crate::domain::{Capabilities, Protocol, Thinking};
use crate::error::{CoreError, CoreResult};
use bytes::Bytes;
use serde_json::Value;
use sse::SseFrame;

pub mod anthropic;
pub mod chat;
pub mod compatibility;
pub mod ir;
pub mod prepare;
pub mod responses;
pub mod sse;
pub mod thinking;
pub mod usage;

pub use usage::Usage;

/// Per-request context handed to outbound (client -> upstream) conversions.
#[derive(Debug, Clone)]
pub struct OutboundCtx {
    pub upstream_model: String,
    pub capabilities: Capabilities,
    pub thinking: Option<Thinking>,
}

/// Parse an inbound request body into the Responses-shaped IR.
pub fn to_ir(inbound: Protocol, body: Value) -> CoreResult<ir::Request> {
    match inbound {
        Protocol::Responses => serde_json::from_value(body)
            .map_err(|e| CoreError::Validation(format!("无效的 Responses 请求: {e}"))),
        Protocol::OpenaiChat => chat::inbound::to_ir(body),
        Protocol::Anthropic => anthropic::inbound::to_ir(body),
    }
}

/// Top-level keys defined by the Chat / Anthropic inbound requests that the
/// Responses API does not accept.
const FOREIGN_TO_RESPONSES: &[&str] = &[
    "response_format",
    "n",
    "seed",
    "stop",
    "stop_sequences",
    "top_k",
    "presence_penalty",
    "frequency_penalty",
    "logprobs",
    "top_logprobs",
    "logit_bias",
    "user",
    "output_config",
    "stream_options",
    "max_tokens",
    "max_completion_tokens",
    "messages",
    "system",
    "thinking",
    "reasoning_effort",
    "modalities",
    "audio",
    "prediction",
    "functions",
    "function_call",
];

/// Render the IR as an upstream request body.
pub fn build_upstream(
    req: &ir::Request,
    upstream: Protocol,
    ctx: &OutboundCtx,
) -> CoreResult<Value> {
    match upstream {
        Protocol::OpenaiChat => chat::outbound::from_ir(req, ctx),
        Protocol::Anthropic => anthropic::outbound::from_ir(req, ctx),
        Protocol::Responses => {
            let mut body = serde_json::to_value(req)
                .map_err(|e| CoreError::Validation(format!("无法序列化请求: {e}")))?;
            if let Some(obj) = body.as_object_mut() {
                obj.insert("model".into(), Value::String(ctx.upstream_model.clone()));
                // Parameters of the Chat / Anthropic wire that Responses rejects.
                // Vendor extensions (reasoning_content, namespace tools, ...)
                // live inside items and tools, so they are unaffected.
                for key in FOREIGN_TO_RESPONSES {
                    obj.remove(*key);
                }
            }
            thinking::apply_raw(
                Protocol::Responses,
                &mut body,
                ctx.thinking,
                &ctx.capabilities,
            );
            Ok(body)
        }
    }
}

/// Pick the wire protocol sent to the provider.
///
/// `provider.protocols` lists every client protocol yi-token accepts. It is not
/// the set of wire protocols the upstream speaks: a Responses provider is stored
/// with Anthropic marked as accepted because the proxy translates. Passthrough
/// is only safe when the client already speaks the provider's native protocol.
/// Otherwise Claude would forward an Anthropic body and `x-api-key` to a
/// Responses endpoint, and the upstream answers 401.
pub fn choose_upstream(
    inbound: Protocol,
    _accepted: &[Protocol],
    native: Protocol,
) -> (Protocol, bool) {
    if inbound == native {
        (native, true)
    } else {
        (native, false)
    }
}

/// Observer of every IR event the upstream parser produces, before translation.
pub type EventTap = Box<dyn FnMut(&ir::StreamEvent) + Send>;

/// Converts upstream SSE frames into client bytes while tracking usage.
pub trait StreamTranslator {
    fn on_frame(&mut self, f: SseFrame) -> CoreResult<Vec<Bytes>>;
    fn finish(&mut self) -> Vec<Bytes>;
    fn usage(&self) -> Usage;
    fn set_tap(&mut self, tap: EventTap);
}

/// Upstream stream parser producing IR events.
enum Source {
    Responses(Usage),
    Chat(chat::stream::UpstreamToIr),
    Anthropic(anthropic::stream::UpstreamToIr),
}

impl Source {
    fn new(p: Protocol) -> Self {
        match p {
            Protocol::Responses => Source::Responses(Usage::default()),
            Protocol::OpenaiChat => Source::Chat(chat::stream::UpstreamToIr::new()),
            Protocol::Anthropic => Source::Anthropic(anthropic::stream::UpstreamToIr::new()),
        }
    }

    fn on_frame(&mut self, frame: &SseFrame) -> CoreResult<Vec<ir::StreamEvent>> {
        match self {
            Source::Responses(usage) => {
                let Some(ev) = ir::StreamEvent::from_frame(frame) else {
                    return Ok(vec![]);
                };
                if let ir::StreamEvent::Completed { response, .. }
                | ir::StreamEvent::Incomplete { response, .. }
                | ir::StreamEvent::Failed { response, .. } = &ev
                    && response.usage.is_some()
                {
                    *usage = response.usage();
                }
                Ok(vec![ev])
            }
            Source::Chat(s) => s.on_frame(frame.clone()),
            Source::Anthropic(s) => s.on_frame(frame.clone()),
        }
    }

    fn on_end(&mut self) -> Vec<ir::StreamEvent> {
        match self {
            Source::Chat(s) => s.on_end(),
            _ => vec![],
        }
    }

    fn usage(&self) -> Usage {
        match self {
            Source::Responses(u) => *u,
            Source::Chat(s) => s.usage(),
            Source::Anthropic(s) => s.usage(),
        }
    }
}

/// IR events -> client bytes.
enum Sink {
    Responses,
    Chat(chat::stream::IrToClient),
    Anthropic(anthropic::stream::IrToClient),
}

impl Sink {
    fn new(p: Protocol, include_usage: bool) -> Self {
        match p {
            Protocol::Responses => Sink::Responses,
            Protocol::OpenaiChat => Sink::Chat(chat::stream::IrToClient::with_usage(include_usage)),
            Protocol::Anthropic => Sink::Anthropic(anthropic::stream::IrToClient::new()),
        }
    }

    fn on_event(&mut self, ev: &ir::StreamEvent) -> Vec<Bytes> {
        match self {
            Sink::Responses => vec![sse::encode(Some(&ev.name()), &ev.data().to_string())],
            Sink::Chat(s) => s.on_event(ev),
            Sink::Anthropic(s) => s.on_event(ev),
        }
    }
}

/// Same protocol: forward every frame unchanged, read usage on the side.
struct PassthroughStream {
    source: Source,
    tap: Option<EventTap>,
}

/// Hands the events to the tap, if one is attached. Nothing is returned.
fn emit(tap: &mut Option<EventTap>, events: &[ir::StreamEvent]) {
    if let Some(tap) = tap.as_mut() {
        for event in events {
            tap(event);
        }
    }
}

impl StreamTranslator for PassthroughStream {
    fn on_frame(&mut self, f: SseFrame) -> CoreResult<Vec<Bytes>> {
        // Usage extraction must never break forwarding, and a failed parse
        // gives the tap nothing.
        if let Ok(events) = self.source.on_frame(&f) {
            emit(&mut self.tap, &events);
        }
        Ok(vec![sse::encode(f.event.as_deref(), &f.data)])
    }

    fn finish(&mut self) -> Vec<Bytes> {
        // Chat completes a stream on end without `[DONE]`. Those events reach
        // the tap only; the client already has every byte.
        let events = self.source.on_end();
        emit(&mut self.tap, &events);
        Vec::new()
    }

    fn usage(&self) -> Usage {
        self.source.usage()
    }

    fn set_tap(&mut self, tap: EventTap) {
        self.tap = Some(tap);
    }
}

struct TranslatingStream {
    source: Source,
    sink: Sink,
    tap: Option<EventTap>,
}

impl StreamTranslator for TranslatingStream {
    fn on_frame(&mut self, f: SseFrame) -> CoreResult<Vec<Bytes>> {
        let events = self.source.on_frame(&f)?;
        emit(&mut self.tap, &events);
        Ok(events.iter().flat_map(|e| self.sink.on_event(e)).collect())
    }

    fn finish(&mut self) -> Vec<Bytes> {
        let events = self.source.on_end();
        emit(&mut self.tap, &events);
        events.iter().flat_map(|e| self.sink.on_event(e)).collect()
    }

    fn usage(&self) -> Usage {
        self.source.usage()
    }

    fn set_tap(&mut self, tap: EventTap) {
        self.tap = Some(tap);
    }
}

pub fn stream_translator(
    inbound: Protocol,
    upstream: Protocol,
) -> Box<dyn StreamTranslator + Send> {
    stream_translator_with_usage(inbound, upstream, false)
}

/// `include_usage` is the client's own `stream_options.include_usage` (read
/// from the raw request, which `to_ir` strips); it only matters when the
/// client speaks Chat and the stream is translated.
pub fn stream_translator_with_usage(
    inbound: Protocol,
    upstream: Protocol,
    include_usage: bool,
) -> Box<dyn StreamTranslator + Send> {
    if inbound == upstream {
        Box::new(PassthroughStream {
            source: Source::new(upstream),
            tap: None,
        })
    } else {
        Box::new(TranslatingStream {
            source: Source::new(upstream),
            sink: Sink::new(inbound, include_usage),
            tap: None,
        })
    }
}

/// Non-streaming response: upstream body -> client body, plus usage.
pub fn translate_response(
    inbound: Protocol,
    upstream: Protocol,
    body: Value,
) -> CoreResult<(Value, Usage)> {
    translate_response_parsed(inbound, upstream, body).map(|(out, usage, _)| (out, usage))
}

/// As [`translate_response`], and also returns the upstream response parsed
/// into IR when the body parsed, so a caller can reuse it.
pub fn translate_response_parsed(
    inbound: Protocol,
    upstream: Protocol,
    body: Value,
) -> CoreResult<(Value, Usage, Option<ir::Response>)> {
    let parsed: CoreResult<ir::Response> = match upstream {
        Protocol::Responses => serde_json::from_value(body.clone())
            .map_err(|e| CoreError::Validation(format!("无效的 Responses 响应: {e}"))),
        Protocol::OpenaiChat => chat::inbound::response_to_ir(body.clone()),
        Protocol::Anthropic => anthropic::inbound::response_to_ir(body.clone()),
    };
    if inbound == upstream {
        // Passthrough: usage extraction is best effort and never fails the call.
        let resp = parsed.ok();
        let usage = resp.as_ref().map(ir::Response::usage).unwrap_or_default();
        return Ok((body, usage, resp));
    }
    let resp = parsed?;
    let usage = resp.usage();
    let out = match inbound {
        Protocol::Responses => serde_json::to_value(&resp)
            .map_err(|e| CoreError::Validation(format!("无法序列化响应: {e}")))?,
        Protocol::OpenaiChat => chat::outbound::response_to_client(&resp),
        Protocol::Anthropic => anthropic::outbound::response_to_client(&resp),
    };
    Ok((out, usage, Some(resp)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn responses_ctx() -> OutboundCtx {
        OutboundCtx {
            upstream_model: "up".into(),
            capabilities: Capabilities::default(),
            thinking: None,
        }
    }

    #[test]
    fn chat_foreign_params_are_not_sent_to_responses() {
        let body = json!({"model": "m", "messages": [{"role": "user", "content": "hi"}],
            "response_format": {"type": "json_object"}, "n": 2, "stop": ["x"], "seed": 1,
            "user": "u"});
        let ir = to_ir(Protocol::OpenaiChat, body).unwrap();
        let out = build_upstream(&ir, Protocol::Responses, &responses_ctx()).unwrap();
        for k in ["response_format", "n", "stop", "seed", "user"] {
            assert!(out.get(k).is_none(), "{k} leaked");
        }
        assert_eq!(out["model"], "up");
    }

    #[test]
    fn anthropic_foreign_params_are_not_sent_to_responses() {
        let body = json!({"model": "m", "max_tokens": 5, "top_k": 3, "stop_sequences": ["x"],
            "output_config": {"a": 1}, "messages": [{"role": "user", "content": "hi"}]});
        let ir = to_ir(Protocol::Anthropic, body).unwrap();
        let out = build_upstream(&ir, Protocol::Responses, &responses_ctx()).unwrap();
        for k in ["top_k", "stop_sequences", "output_config", "max_tokens"] {
            assert!(out.get(k).is_none(), "{k} leaked");
        }
        assert_eq!(out["max_output_tokens"], 5);
    }

    #[test]
    fn same_protocol_response_usage_is_best_effort() {
        let body = json!({"output": "not-a-list", "usage": 3});
        let (out, usage) =
            translate_response(Protocol::Responses, Protocol::Responses, body.clone()).unwrap();
        assert_eq!(out, body);
        assert_eq!(usage, Usage::default());
        let body = json!({"choices": "bad"});
        let (out, usage) =
            translate_response(Protocol::OpenaiChat, Protocol::OpenaiChat, body.clone()).unwrap();
        assert_eq!((out, usage), (body, Usage::default()));
    }

    #[test]
    fn choose_upstream_passthrough_only_matches_the_native_protocol() {
        let got = choose_upstream(
            Protocol::Anthropic,
            &[Protocol::OpenaiChat, Protocol::Anthropic, Protocol::Responses],
            Protocol::Anthropic,
        );
        assert_eq!(got, (Protocol::Anthropic, true));
    }

    #[test]
    fn choose_upstream_translates_when_the_client_protocol_is_not_native() {
        // Saved providers list every client protocol, including ones they do not speak.
        let got = choose_upstream(
            Protocol::Anthropic,
            &[Protocol::Anthropic, Protocol::OpenaiChat, Protocol::Responses],
            Protocol::Responses,
        );
        assert_eq!(got, (Protocol::Responses, false));
    }

    #[test]
    fn passthrough_stream_forwards_and_reads_usage() {
        let mut t = stream_translator(Protocol::Responses, Protocol::Responses);
        let frame = SseFrame {
            event: Some("response.completed".into()),
            data: json!({"type": "response.completed", "response": {
                "output": [], "usage": {"input_tokens": 3, "output_tokens": 4}}})
            .to_string(),
        };
        let out = t.on_frame(frame).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out[0].starts_with(b"event: response.completed\n"));
        assert_eq!(t.usage().input_tokens, 3);
        assert_eq!(t.usage().output_tokens, 4);
    }

    #[test]
    fn translate_response_chat_to_anthropic_reports_usage() {
        let body = json!({"id": "c1", "object": "chat.completion", "model": "m",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"},
                "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 5, "completion_tokens": 2}});
        let (out, usage) =
            translate_response(Protocol::Anthropic, Protocol::OpenaiChat, body).unwrap();
        assert_eq!(out["type"], "message");
        assert_eq!(usage.input_tokens, 5);
        assert_eq!(usage.output_tokens, 2);
    }
}
