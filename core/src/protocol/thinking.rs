//! Thinking (reasoning effort) mapping. The single source of truth for how a
//! provider-level [`Thinking`] level becomes a protocol-specific parameter,
//! shared by the raw passthrough path and the IR outbound conversions.

use crate::domain::{Capabilities, Protocol, Thinking};
use serde_json::{Value, json};

/// Smallest `budget_tokens` Anthropic accepts.
pub const MIN_ANTHROPIC_BUDGET: u64 = 1024;

/// Whether the request already carries its own reasoning parameter.
pub fn has_reasoning(protocol: Protocol, body: &Value) -> bool {
    let present = |v: Option<&Value>| v.is_some_and(|v| !v.is_null());
    match protocol {
        Protocol::Anthropic => {
            present(body.get("thinking")) || present(body.pointer("/output_config/effort"))
        }
        Protocol::OpenaiChat => present(body.get("reasoning_effort")),
        Protocol::Responses => present(body.pointer("/reasoning/effort")),
    }
}

/// Effort name for Chat / Responses (`reasoning_effort` / `reasoning.effort`).
pub fn effort_name(t: Thinking) -> &'static str {
    match t {
        Thinking::Minimal => "minimal",
        Thinking::Low => "low",
        Thinking::Medium => "medium",
        Thinking::High => "high",
        Thinking::Xhigh => "xhigh",
    }
}

/// Anthropic `budget_tokens` for a level, before any `max_tokens` handling.
/// `Minimal` is not expressible on Anthropic and yields `None`.
pub fn anthropic_budget(t: Thinking) -> Option<u64> {
    match t {
        Thinking::Minimal => None,
        Thinking::Low => Some(4096),
        Thinking::Medium => Some(10000),
        Thinking::High => Some(32000),
        Thinking::Xhigh => Some(64000),
    }
}

/// Budget capped below `max_tokens`; `None` when the result would fall under
/// Anthropic's 1024 minimum (thinking is then not enabled).
pub fn clamp_budget(budget: u64, max_tokens: u64) -> Option<u64> {
    let clamped = budget.min(max_tokens.saturating_sub(1));
    (clamped >= MIN_ANTHROPIC_BUDGET).then_some(clamped)
}

/// Fill the reasoning parameter of a raw request body, only when the client
/// did not set one and the model is known to support reasoning.
pub fn apply_raw(
    protocol: Protocol,
    body: &mut Value,
    thinking: Option<Thinking>,
    caps: &Capabilities,
) {
    let Some(t) = thinking else { return };
    if !caps.supports_reasoning() || has_reasoning(protocol, body) {
        return;
    }
    let Some(obj) = body.as_object_mut() else {
        return;
    };
    match protocol {
        Protocol::Anthropic => {
            let max_tokens = obj
                .get("max_tokens")
                .and_then(Value::as_u64)
                .or(caps.max_output_tokens.map(u64::from));
            let budget = anthropic_budget(t)
                .zip(max_tokens)
                .and_then(|(b, m)| clamp_budget(b, m));
            if let Some(b) = budget {
                obj.insert(
                    "thinking".into(),
                    json!({"type": "enabled", "budget_tokens": b}),
                );
                // Rejected by Anthropic while thinking is enabled.
                obj.remove("temperature");
                obj.remove("top_p");
            }
        }
        Protocol::OpenaiChat => {
            obj.insert("reasoning_effort".into(), json!(effort_name(t)));
        }
        Protocol::Responses => {
            let reasoning = obj.entry("reasoning").or_insert_with(|| json!({}));
            if reasoning.is_null() {
                *reasoning = json!({});
            }
            if let Some(r) = reasoning.as_object_mut() {
                r.insert("effort".into(), json!(effort_name(t)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::capabilities::{Effort, Support};

    fn caps(supported: bool) -> Capabilities {
        Capabilities {
            effort: Effort {
                support: if supported {
                    Support::Supported
                } else {
                    Support::Unknown
                },
                levels: if supported {
                    vec!["low".into()]
                } else {
                    vec![]
                },
                default: None,
            },
            ..Capabilities::default()
        }
    }

    #[test]
    fn thinking_fills_anthropic_budget() {
        let mut body = json!({"model": "m", "max_tokens": 20000, "temperature": 0.3});
        apply_raw(
            Protocol::Anthropic,
            &mut body,
            Some(Thinking::Medium),
            &caps(true),
        );
        assert_eq!(
            body["thinking"],
            json!({"type": "enabled", "budget_tokens": 10000})
        );
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn thinking_exact_low_and_high_budgets() {
        for (t, want) in [(Thinking::Low, 4096), (Thinking::High, 32000)] {
            let mut body = json!({"max_tokens": 100000});
            apply_raw(Protocol::Anthropic, &mut body, Some(t), &caps(true));
            assert_eq!(body["thinking"]["budget_tokens"], want);
        }
    }

    #[test]
    fn thinking_budget_clamped_below_max_tokens() {
        let mut body = json!({"max_tokens": 5000});
        apply_raw(
            Protocol::Anthropic,
            &mut body,
            Some(Thinking::Xhigh),
            &caps(true),
        );
        assert_eq!(body["thinking"]["budget_tokens"], 4999);
        let mut body = json!({"max_tokens": 1000});
        apply_raw(
            Protocol::Anthropic,
            &mut body,
            Some(Thinking::Low),
            &caps(true),
        );
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn anthropic_output_config_effort_counts_as_client_reasoning() {
        let mut body = json!({"max_tokens": 9000, "output_config": {"effort": "high"}});
        apply_raw(
            Protocol::Anthropic,
            &mut body,
            Some(Thinking::High),
            &caps(true),
        );
        assert!(body.get("thinking").is_none());
        assert!(has_reasoning(Protocol::Anthropic, &body));
        assert!(!has_reasoning(
            Protocol::Anthropic,
            &json!({"output_config": {"effort": null}})
        ));
    }

    #[test]
    fn thinking_respects_client_value() {
        let mut body = json!({"reasoning_effort": "low"});
        apply_raw(
            Protocol::OpenaiChat,
            &mut body,
            Some(Thinking::High),
            &caps(true),
        );
        assert_eq!(body["reasoning_effort"], "low");
        let mut body = json!({"reasoning": {"effort": "low"}});
        apply_raw(
            Protocol::Responses,
            &mut body,
            Some(Thinking::High),
            &caps(true),
        );
        assert_eq!(body["reasoning"]["effort"], "low");
        let mut body = json!({"max_tokens": 9000, "thinking": {"type": "disabled"}});
        apply_raw(
            Protocol::Anthropic,
            &mut body,
            Some(Thinking::High),
            &caps(true),
        );
        assert_eq!(body["thinking"], json!({"type": "disabled"}));
    }

    #[test]
    fn thinking_skipped_when_model_has_no_reasoning() {
        let mut body = json!({"max_tokens": 9000});
        let before = body.clone();
        apply_raw(
            Protocol::Anthropic,
            &mut body,
            Some(Thinking::High),
            &caps(false),
        );
        assert_eq!(body, before);
    }

    #[test]
    fn thinking_minimal_anthropic_disabled() {
        let mut body = json!({"max_tokens": 9000});
        apply_raw(
            Protocol::Anthropic,
            &mut body,
            Some(Thinking::Minimal),
            &caps(true),
        );
        assert!(body.get("thinking").is_none());
        let mut body = json!({});
        apply_raw(
            Protocol::OpenaiChat,
            &mut body,
            Some(Thinking::Minimal),
            &caps(true),
        );
        assert_eq!(body["reasoning_effort"], "minimal");
        let mut body = json!({});
        apply_raw(
            Protocol::Responses,
            &mut body,
            Some(Thinking::Minimal),
            &caps(true),
        );
        assert_eq!(body["reasoning"]["effort"], "minimal");
    }

    #[test]
    fn has_reasoning_per_protocol() {
        assert!(has_reasoning(
            Protocol::Anthropic,
            &json!({"thinking": {"type": "enabled"}})
        ));
        assert!(!has_reasoning(
            Protocol::Anthropic,
            &json!({"reasoning_effort": "low"})
        ));
        assert!(has_reasoning(
            Protocol::Responses,
            &json!({"reasoning": {"effort": "low"}})
        ));
        assert!(!has_reasoning(
            Protocol::Responses,
            &json!({"reasoning": {"summary": "auto"}})
        ));
    }
}
