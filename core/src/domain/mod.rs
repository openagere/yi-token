//! Domain types shared by storage, services and the frontend bindings.

pub mod capabilities;
pub mod client;
pub mod company;
pub mod discovered;
pub mod model;
pub mod names;
pub mod protocol;
pub mod provider;
pub mod session;
pub mod settings;
pub mod standard_model;
pub mod terminal;
pub mod thinking;
pub mod usage;

pub use capabilities::{Capabilities, Effort, Modality, Support};
pub use client::Client;
pub use company::Company;
pub use discovered::DiscoveredModel;
pub use model::Model;
pub use names::{exposed_name, provider_slug};
pub use protocol::Protocol;
pub use provider::Provider;
pub use session::{
    BlobView, ContextItem, KeySource, LivePhase, LiveRequest, LiveSnapshot, RecordKind,
    SessionChanges, SessionDetail, SessionListInput, SessionPage, SessionSummary, StepContext,
    ThreadKindView, ThreadView, TrajectoryInput, TrajectoryPage, TrajectoryRecord,
    TrajectoryStatus, TrajectoryStep, TrajectoryTurn,
};
pub use settings::{LogLevel, Settings, TrajectorySettings};
pub use standard_model::StandardModel;
pub use terminal::{AUTO_MODEL, DirectProfile, ModelSource, ProfileModel, ProxyProfile, RouteMode};
pub use thinking::Thinking;
pub use usage::{
    UsageClient, UsageDailyModel, UsageDay, UsageEvent, UsageGroupBy, UsageGroupRow, UsageModel,
    UsageRecord, UsageReport, UsageTotals,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{CoreError, ErrorCode};
    use std::str::FromStr;

    fn provider() -> Provider {
        Provider {
            id: "p1".into(),
            provider_type: Protocol::OpenaiChat,
            name: "OpenAI".into(),
            base_url: "https://api.openai.com/v1".into(),
            api_key: "sk".into(),
            enabled: true,
            thinking: None,
            is_default: false,
            sort_order: 0,
            extra: serde_json::json!({}),
        }
    }

    #[test]
    fn protocol_serde_names() {
        for (p, s) in [
            (Protocol::Anthropic, "anthropic"),
            (Protocol::OpenaiChat, "openai_chat"),
            (Protocol::Responses, "responses"),
        ] {
            assert_eq!(serde_json::to_string(&p).unwrap(), format!("\"{s}\""));
            assert_eq!(p.as_str(), s);
            assert_eq!(Protocol::from_str(s).unwrap(), p);
        }
        assert!(Protocol::from_str("x").is_err());
        assert_eq!(Protocol::all().len(), 3);
    }

    #[test]
    fn client_serde_names_and_protocols() {
        for (c, s) in [
            (Client::Codex, "codex"),
            (Client::ClaudeCode, "claude-code"),
            (Client::Opencode, "opencode"),
            (Client::Pi, "pi"),
            (Client::DeepseekHarness, "deepseek-harness"),
        ] {
            assert_eq!(serde_json::to_string(&c).unwrap(), format!("\"{s}\""));
            assert_eq!(Client::from_str(s).unwrap(), c);
        }
        assert_eq!(Client::Codex.protocols(), &[Protocol::Responses]);
        assert_eq!(Client::ClaudeCode.protocols(), &[Protocol::Anthropic]);
        assert_eq!(Client::Pi.protocols().len(), 3);
        assert_eq!(Client::Codex.default_protocol(), Protocol::Responses);
    }

    #[test]
    fn provider_validate_rejects_blank_name_and_bad_url() {
        assert!(provider().validate().is_ok());
        for name in ["", "   ", "//"] {
            let mut p = provider();
            p.name = name.into();
            assert_eq!(
                p.validate().unwrap_err().code(),
                ErrorCode::Validation,
                "{name}"
            );
        }
        let mut p = provider();
        p.base_url = "ftp://x".into();
        assert!(p.validate().is_err());
    }

    #[test]
    fn exposed_name_format() {
        assert_eq!(exposed_name("OpenAI", "gpt-5"), "OpenAI/gpt-5");
        assert_eq!(exposed_name(" Deep  Seek ", " v4 "), "Deep-Seek/v4");
    }

    fn fixed(client: Client, protocol: Protocol) -> ProxyProfile {
        ProxyProfile {
            client,
            protocol,
            route_mode: RouteMode::Fixed,
            models: vec![ProfileModel {
                provider_id: "p1".into(),
                model: "m".into(),
            }],
            default_model: Some("m".into()),
        }
    }

    #[test]
    fn proxy_profile_rejects_protocol_not_supported_by_client() {
        assert!(fixed(Client::Codex, Protocol::Responses).validate().is_ok());
        let err = fixed(Client::Codex, Protocol::Anthropic)
            .validate()
            .unwrap_err();
        assert!(matches!(err, CoreError::Validation(_)));
        let mut p = fixed(Client::Pi, Protocol::Anthropic);
        p.default_model = Some("other".into());
        assert!(p.validate().is_err());
        p.models.clear();
        assert!(p.validate().is_err());
        p.default_model = Some(AUTO_MODEL.into());
        assert!(p.validate().is_ok());
        p.default_model = None;
        assert!(p.validate().is_ok());
    }

    #[test]
    fn proxy_profile_auto_allows_empty_models() {
        let p = ProxyProfile {
            client: Client::Pi,
            protocol: Protocol::OpenaiChat,
            route_mode: RouteMode::Auto,
            models: vec![],
            default_model: None,
        };
        assert!(p.validate().is_ok());
        assert_eq!(AUTO_MODEL, "yi-token");
        assert_eq!(serde_json::to_string(&RouteMode::Auto).unwrap(), "\"auto\"");
    }

    #[test]
    fn settings_default() {
        let s = Settings::default();
        assert_eq!(
            (s.host.as_str(), s.port, s.log_level),
            ("127.0.0.1", 11435, LogLevel::Info)
        );
    }

    fn caps() -> Capabilities {
        Capabilities {
            input_modalities: Some(vec![Modality::Text, Modality::Image]),
            output_modalities: Some(vec![Modality::Text]),
            max_output_tokens: Some(4096),
            context_window: Some(65536),
            effort: Effort {
                support: Support::Supported,
                levels: vec!["low".into(), "high".into()],
                default: Some("low".into()),
            },
        }
    }

    #[test]
    fn capabilities_validate_matches_yi_llm() {
        let r = Protocol::Responses;
        let mut v = caps();
        assert!(v.validate(r).is_ok());
        v.input_modalities = Some(vec![Modality::Text, Modality::Text]);
        assert!(v.validate(r).is_err());
        v = caps();
        v.context_window = Some(0);
        assert!(v.validate(r).is_err());
        v = caps();
        v.max_output_tokens = Some(65537);
        assert!(v.validate(r).is_err());
        v = caps();
        v.effort.levels.push("xhigh".into());
        assert!(v.validate(r).is_ok());
        assert!(v.validate(Protocol::Anthropic).is_ok());
        v.effort.levels.push("ultra".into());
        assert!(v.validate(Protocol::Anthropic).is_err());
        v = caps();
        v.effort.support = Support::Unsupported;
        assert!(v.validate(r).is_err());
    }

    #[test]
    fn capabilities_efforts_for_intersects_protocol_levels() {
        let mut v = caps();
        v.effort
            .levels
            .extend(["xhigh".into(), "max".into(), "ultra".into()]);
        let e = v.efforts_for(Protocol::Anthropic, Protocol::Responses);
        assert!(e.contains(&"max".to_string()) && !e.contains(&"ultra".to_string()));
        assert!(v.supports_reasoning());
        assert!(!Capabilities::default().supports_reasoning());
    }
}
