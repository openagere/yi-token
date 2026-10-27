//! Request preparation: raw same-protocol passthrough and IR translation guards.

use crate::domain::{Capabilities, Protocol, Thinking};
use crate::error::{CoreError, CoreResult};
use crate::protocol::{compatibility, ir, thinking};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct PrepareCtx {
    pub inbound: Protocol,
    pub upstream: Protocol,
    pub upstream_model: String,
    pub capabilities: Capabilities,
    pub thinking: Option<Thinking>,
    pub non_standard: bool,
}

/// Same-protocol path: replace `model`, apply compatibility rules for
/// `non_standard` mappings, then fill the thinking fallback. Every other field,
/// known or not, is left untouched.
pub fn passthrough(body: &mut Value, ctx: &PrepareCtx) -> CoreResult<()> {
    let obj = body
        .as_object_mut()
        .ok_or_else(|| CoreError::Validation("请求体必须是 JSON 对象".into()))?;
    obj.insert("model".into(), Value::String(ctx.upstream_model.clone()));
    if ctx.upstream == Protocol::OpenaiChat {
        developer_to_system(obj);
    }
    if ctx.non_standard {
        let report = compatibility::adapt(body, ctx.upstream);
        if let Some(reason) = report.unsatisfiable {
            return Err(CoreError::Validation(reason));
        }
        if report.changed() {
            tracing::debug!(?report, "applied non-standard compatibility rules");
        }
    }
    thinking::apply_raw(ctx.upstream, body, ctx.thinking, &ctx.capabilities);
    Ok(())
}

/// Codex sends its instructions as a `developer` message, which most
/// Chat-compatible upstreams reject. Only the role name changes.
fn developer_to_system(obj: &mut serde_json::Map<String, Value>) {
    let Some(messages) = obj.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for message in messages {
        if message.get("role").and_then(Value::as_str) == Some("developer") {
            message["role"] = Value::String("system".into());
        }
    }
}

/// Reject server-side conversation state when translating to an upstream that
/// cannot honour it.
pub fn validate_for_translation(req: &ir::Request, upstream: Protocol) -> CoreResult<()> {
    if upstream != Protocol::Responses
        && (req.store == Some(true) || req.previous_response_id.is_some())
    {
        return Err(CoreError::Validation(
            "该上游不支持服务端会话状态（store / previous_response_id）".into(),
        ));
    }
    if structured_output_requested(req) {
        // Translation of structured output is a tracked follow-up; never drop it silently.
        return Err(CoreError::Validation(
            "当前上游不支持结构化输出（json_schema）".into(),
        ));
    }
    Ok(())
}

/// Whether the client asked for `json_schema` structured output through
/// `response_format` (Chat), `text.format` (Responses) or `output_config.format`
/// (Anthropic).
fn structured_output_requested(req: &ir::Request) -> bool {
    let is_schema = |v: Option<&Value>| {
        v.and_then(|v| v.get("type")).and_then(Value::as_str) == Some("json_schema")
    };
    is_schema(req.extra.get("response_format"))
        || is_schema(req.extra.get("text").and_then(|t| t.get("format")))
        || is_schema(req.extra.get("output_config").and_then(|t| t.get("format")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::capabilities::{Effort, Support};
    use serde_json::json;

    fn ctx(upstream: Protocol, non_standard: bool) -> PrepareCtx {
        PrepareCtx {
            inbound: upstream,
            upstream,
            upstream_model: "up-model".into(),
            capabilities: Capabilities {
                effort: Effort {
                    support: Support::Supported,
                    levels: vec!["low".into()],
                    default: None,
                },
                ..Capabilities::default()
            },
            thinking: Some(Thinking::High),
            non_standard,
        }
    }

    fn ir_from(v: Value) -> ir::Request {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn store_rejected_for_chat_upstream() {
        let req = ir_from(json!({"model": "m", "input": "hi", "store": true}));
        let err = validate_for_translation(&req, Protocol::OpenaiChat).unwrap_err();
        assert!(matches!(&err, CoreError::Validation(m) if m.contains("store")));
        let req = ir_from(json!({"model": "m", "input": "hi", "previous_response_id": "r1"}));
        assert!(validate_for_translation(&req, Protocol::Anthropic).is_err());
        let req = ir_from(json!({"model": "m", "input": "hi", "store": false}));
        assert!(validate_for_translation(&req, Protocol::Anthropic).is_ok());
    }

    #[test]
    fn json_schema_structured_output_is_rejected_on_translation() {
        for (inbound, body) in [
            (
                Protocol::OpenaiChat,
                json!({"model": "m", "messages": [{"role": "user", "content": "hi"}],
                    "response_format": {"type": "json_schema", "json_schema": {"name": "x"}}}),
            ),
            (
                Protocol::Responses,
                json!({"model": "m", "input": "hi",
                    "text": {"format": {"type": "json_schema", "name": "x", "schema": {}}}}),
            ),
            (
                Protocol::Anthropic,
                json!({"model": "m", "max_tokens": 5,
                    "messages": [{"role": "user", "content": "hi"}],
                    "output_config": {"format": {"type": "json_schema", "schema": {}}}}),
            ),
        ] {
            let req = crate::protocol::to_ir(inbound, body).unwrap();
            let err = validate_for_translation(&req, Protocol::Anthropic).unwrap_err();
            assert!(matches!(&err, CoreError::Validation(m) if m.contains("结构化输出")));
        }
        let req =
            ir_from(json!({"model": "m", "input": "hi", "text": {"format": {"type": "text"}}}));
        assert!(validate_for_translation(&req, Protocol::OpenaiChat).is_ok());
    }

    #[test]
    fn previous_response_id_allowed_for_responses_upstream() {
        let req = ir_from(
            json!({"model": "m", "input": "hi", "store": true, "previous_response_id": "r1"}),
        );
        assert!(validate_for_translation(&req, Protocol::Responses).is_ok());
    }

    #[test]
    fn passthrough_replaces_model_and_keeps_unknown_fields() {
        let mut body = json!({
            "model": "alias", "input": "hi", "x_vendor": {"a": 1},
            "tools": [{"type": "namespace", "name": "n"}]
        });
        passthrough(&mut body, &ctx(Protocol::Responses, false)).unwrap();
        assert_eq!(body["model"], "up-model");
        assert_eq!(body["x_vendor"], json!({"a": 1}));
        // standard mapping: no compatibility rules
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
        // thinking fallback applied
        assert_eq!(body["reasoning"]["effort"], "high");

        let mut body = json!({"model": "alias", "tools": [{"type": "namespace"}], "x": 1});
        passthrough(&mut body, &ctx(Protocol::Responses, true)).unwrap();
        assert!(body.get("tools").is_none());
        assert_eq!(body["x"], 1);
    }

    #[test]
    fn chat_passthrough_expresses_developer_as_system() {
        let mut body = json!({"model": "a", "messages": [
            {"role": "developer", "content": "rules"}, {"role": "user", "content": "hi"}]});
        passthrough(&mut body, &ctx(Protocol::OpenaiChat, false)).unwrap();
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "rules");
        assert_eq!(body["messages"][1]["role"], "user");
        // other protocols keep the role
        let mut body = json!({"model": "a", "input": [{"role": "developer", "content": "r"}]});
        passthrough(&mut body, &ctx(Protocol::Responses, false)).unwrap();
        assert_eq!(body["input"][0]["role"], "developer");
    }

    #[test]
    fn passthrough_non_standard_rejects_stateful_request() {
        let mut body = json!({"model": "a", "input": "hi", "store": true});
        assert!(passthrough(&mut body, &ctx(Protocol::Responses, true)).is_err());
        let mut body = json!({"model": "a", "input": "hi", "store": true});
        assert!(passthrough(&mut body, &ctx(Protocol::Responses, false)).is_ok());
    }
}
