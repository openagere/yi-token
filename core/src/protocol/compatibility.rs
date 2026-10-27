//! Protocol-scoped compatibility rules for upstream mappings explicitly marked
//! `non_standard`. The normal path remains an exact native pass-through.
//!
//! Keep each rule small and evidence-based. Never silently discard
//! conversation history, a forced tool choice, or a request to persist a
//! response: those are reported in [`CompatibilityReport::unsatisfiable`].

use crate::domain::Protocol;
use serde_json::Value;

/// Responses tools rejected by the observed partial Responses gateway.
/// Ordinary function tools are retained.
const UNSUPPORTED_RESPONSES_TOOLS: &[&str] = &["namespace", "web_search", "web_search_preview"];
const UNSUPPORTED_RESPONSES_INCLUDE: &[&str] = &["message.annotations"];

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CompatibilityReport {
    pub omitted_tool_types: Vec<String>,
    pub omitted_include: Vec<String>,
    pub adjusted_fields: Vec<&'static str>,
    /// The request asks for something the upstream cannot do and that must not
    /// be silently dropped; the caller should reject it.
    pub unsatisfiable: Option<String>,
}

impl CompatibilityReport {
    pub fn changed(&self) -> bool {
        !self.omitted_tool_types.is_empty()
            || !self.omitted_include.is_empty()
            || !self.adjusted_fields.is_empty()
    }
}

/// Applied once to a request about to be forwarded to a `non_standard`
/// upstream mapping. Only the Responses rules are currently known.
pub fn adapt(body: &mut Value, protocol: Protocol) -> CompatibilityReport {
    let mut report = CompatibilityReport::default();
    if protocol != Protocol::Responses {
        return report;
    }
    let Some(obj) = body.as_object_mut() else {
        return report;
    };

    if let Some(tools) = obj.get_mut("tools").and_then(Value::as_array_mut) {
        tools.retain(|tool| {
            let kind = tool.get("type").and_then(Value::as_str).unwrap_or_default();
            if UNSUPPORTED_RESPONSES_TOOLS.contains(&kind) {
                report.omitted_tool_types.push(kind.to_owned());
                false
            } else {
                true
            }
        });
        if tools.is_empty() {
            obj.remove("tools");
        }
    }

    let has_tools = obj
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|t| !t.is_empty());
    if let Some(choice) = obj.get("tool_choice").cloned() {
        let kind = choice
            .as_str()
            .or_else(|| choice.get("type").and_then(Value::as_str));
        match kind {
            Some("required") if !has_tools => {
                report.unsatisfiable =
                    Some("非标准模型不支持请求中的工具，tool_choice=required 无法满足".into());
            }
            Some(k) if UNSUPPORTED_RESPONSES_TOOLS.contains(&k) => {
                report.unsatisfiable = Some(format!("非标准模型不支持强制调用 {k} 工具"));
            }
            Some("function") => {
                let name = choice.get("name").and_then(Value::as_str).unwrap_or("");
                let found = obj
                    .get("tools")
                    .and_then(Value::as_array)
                    .is_some_and(|tools| {
                        tools.iter().any(|t| {
                            t.get("type").and_then(Value::as_str) == Some("function")
                                && t.get("name").and_then(Value::as_str) == Some(name)
                        })
                    });
                if !found {
                    report.unsatisfiable = Some("非标准模型强制指定的 function 工具不可用".into());
                }
            }
            _ => {}
        }
        if report.unsatisfiable.is_none() {
            if !has_tools && kind == Some("auto") {
                obj.remove("tool_choice");
                report.adjusted_fields.push("/tool_choice");
            } else if choice.is_object()
                && let Some(k @ ("auto" | "none" | "required")) = kind
            {
                obj.insert("tool_choice".into(), Value::String(k.to_owned()));
                report.adjusted_fields.push("/tool_choice");
            }
        }
    }

    if let Some(include) = obj.get_mut("include").and_then(Value::as_array_mut) {
        include.retain(|item| match item.as_str() {
            Some(k) if UNSUPPORTED_RESPONSES_INCLUDE.contains(&k) => {
                report.omitted_include.push(k.to_owned());
                false
            }
            _ => true,
        });
        if include.is_empty() {
            obj.remove("include");
        }
    }

    if let Some(text) = obj.get_mut("text").and_then(Value::as_object_mut) {
        if text.remove("verbosity").is_some() {
            report.adjusted_fields.push("/text/verbosity");
        }
        if text.is_empty() {
            obj.remove("text");
        }
    }
    if obj.get("truncation").and_then(Value::as_str) == Some("auto") {
        obj.remove("truncation");
        report.adjusted_fields.push("/truncation");
    }
    if obj.remove("stream_options").is_some() {
        report.adjusted_fields.push("/stream_options");
    }
    // Stateful semantics must not be silently dropped.
    if report.unsatisfiable.is_none() {
        if obj.get("store") == Some(&Value::Bool(true)) {
            report.unsatisfiable = Some("非标准模型不支持 store=true；请使用 store=false".into());
        } else if obj
            .get("previous_response_id")
            .is_some_and(|v| !v.is_null())
        {
            report.unsatisfiable =
                Some("非标准模型不支持 previous_response_id；请发送完整 input 历史".into());
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compatibility_drops_namespace_tools_only_when_non_standard() {
        let mut body = json!({
            "model": "test", "input": "hello", "store": false,
            "tools": [
                {"type": "function", "name": "shell", "parameters": {"type": "object"}},
                {"type": "namespace", "name": "mcp", "tools": []},
                {"type": "web_search"}, {"type": "web_search_preview"}
            ],
            "tool_choice": {"type": "auto"},
            "include": ["reasoning.encrypted_content", "message.annotations"],
            "text": {"verbosity": "medium", "format": {"type": "text"}},
            "truncation": "auto", "stream_options": {"include_usage": true}
        });
        let report = adapt(&mut body, Protocol::Responses);
        assert_eq!(
            report.omitted_tool_types,
            ["namespace", "web_search", "web_search_preview"]
        );
        assert_eq!(report.omitted_include, ["message.annotations"]);
        assert!(report.unsatisfiable.is_none());
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
        assert_eq!(body["text"], json!({"format": {"type": "text"}}));
        assert!(body.get("truncation").is_none() && body.get("stream_options").is_none());
        assert_eq!(body["store"], false);
    }

    #[test]
    fn unsatisfiable_requests_are_reported_not_dropped() {
        for body in [
            json!({"store": true}),
            json!({"previous_response_id": "resp_1"}),
            json!({"tools": [{"type": "namespace", "name": "mcp"}], "tool_choice": "required"}),
            json!({"tools": [{"type": "web_search"}], "tool_choice": {"type": "web_search"}}),
        ] {
            let mut body = body;
            assert!(
                adapt(&mut body, Protocol::Responses)
                    .unsatisfiable
                    .is_some()
            );
        }
    }

    #[test]
    fn drops_auto_choice_when_every_tool_was_omitted() {
        let mut body = json!({"tools": [{"type": "namespace", "name": "mcp"}], "tool_choice": {"type": "auto"}});
        adapt(&mut body, Protocol::Responses);
        assert!(body.get("tools").is_none() && body.get("tool_choice").is_none());
    }

    #[test]
    fn other_protocols_untouched() {
        let original = json!({"tools": [{"type": "namespace"}], "custom": {"kept": true}});
        for p in [Protocol::Anthropic, Protocol::OpenaiChat] {
            let mut body = original.clone();
            assert!(!adapt(&mut body, p).changed());
            assert_eq!(body, original);
        }
    }
}
