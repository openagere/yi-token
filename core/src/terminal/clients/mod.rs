//! Client-specific renderers.

mod claude_code;
mod codex;
mod deepseek_harness;
mod opencode;
mod pi;

use super::spec::{ClientCtx, ClientSpec};
use crate::domain::{
    Capabilities, Client, DirectProfile, Effort, Modality, Model, ProfileModel, Protocol, Provider,
    Support,
};
use crate::error::{CoreError, CoreResult};

pub use self::claude_code::ClaudeCode;
pub use self::codex::Codex;
pub use self::deepseek_harness::DeepSeekHarness;
pub use self::opencode::OpenCode;
pub use self::pi::Pi;

pub const PROXY_PROVIDER_ID: &str = "yitoken";
pub const DIRECT_PROVIDER_PREFIX: &str = "yitoken_direct_";
pub const DIRECT_NAME_PREFIX: &str = "yi-token direct:";

pub fn spec_for(client: Client) -> &'static dyn ClientSpec {
    match client {
        Client::Codex => &Codex,
        Client::ClaudeCode => &ClaudeCode,
        Client::Opencode => &OpenCode,
        Client::Pi => &Pi,
        Client::DeepseekHarness => &DeepSeekHarness,
    }
}

#[derive(Clone, Copy)]
pub struct ResolvedModel<'a> {
    pub selection: &'a ProfileModel,
    pub provider: &'a Provider,
    pub model: &'a Model,
    pub capabilities: &'a Capabilities,
}

impl ResolvedModel<'_> {
    /// The wire protocol the selected model is called with upstream.
    pub fn upstream(&self) -> Protocol {
        self.model.upstream_protocol(self.provider)
    }
}

/// Resolves every selected model to its provider and model record, in profile order.
pub fn resolved_models<'a>(
    profile: &'a crate::domain::ProxyProfile,
    ctx: &'a ClientCtx,
) -> CoreResult<Vec<ResolvedModel<'a>>> {
    profile
        .models
        .iter()
        .map(|selection| {
            let provider = ctx
                .providers
                .iter()
                .find(|provider| provider.id == selection.provider_id)
                .ok_or_else(|| {
                    CoreError::Validation("终端配置中的服务商不存在或已停用，请重新选择".into())
                })?;
            let model = ctx
                .models
                .iter()
                .find(|model| {
                    model.provider_id == selection.provider_id
                        && (model.exposed_name == selection.model || model.id == selection.model)
                })
                .ok_or_else(|| {
                    CoreError::Validation("终端配置中的模型不存在或已停用，请重新选择".into())
                })?;
            Ok(ResolvedModel {
                selection,
                provider,
                model,
                capabilities: &model.capabilities,
            })
        })
        .collect()
}

/// Fallback capabilities when a proxy connection has no selected engine.
///
/// Text and image input, with no invented context window. Connect prefers
/// [`proxy_model_caps`], which copies the selected engine instead.
pub fn auto_caps(client_protocol: Protocol) -> (Capabilities, Protocol) {
    let caps = Capabilities {
        input_modalities: Some(vec![Modality::Text, Modality::Image]),
        output_modalities: Some(vec![Modality::Text]),
        context_window: None,
        max_output_tokens: None,
        effort: Effort {
            support: Support::Supported,
            levels: ["low", "medium", "high"].map(str::to_owned).to_vec(),
            default: Some("medium".to_owned()),
        },
    };
    (caps, client_protocol)
}

/// Capabilities of the virtual `yi-token` model written when connecting.
///
/// The selected engine's context window, input and output modalities, output
/// limit and effort are copied at connect time. Later engine switches do not
/// render the terminal file again, so that snapshot stays until the next connect.
pub fn proxy_model_caps(ctx: &ClientCtx, client_protocol: Protocol) -> (Capabilities, Protocol) {
    let Some(model) = ctx.active_engine.as_ref() else {
        return auto_caps(client_protocol);
    };
    let upstream = ctx
        .providers
        .iter()
        .find(|provider| provider.id == model.provider_id)
        .map(|provider| model.upstream_protocol(provider))
        .unwrap_or(client_protocol);
    (model.capabilities.clone(), upstream)
}

/// Models a direct connection can reach: direct access talks to the provider's
/// native protocol, so models overriding it are left out.
pub fn provider_models<'a>(provider_id: &str, ctx: &'a ClientCtx) -> Vec<&'a Model> {
    let native = ctx
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .map(|provider| provider.provider_type);
    ctx.models
        .iter()
        .filter(|model| model.provider_id == provider_id)
        .filter(|model| model.protocol.is_none() || model.protocol == native)
        .collect()
}

pub fn direct_provider<'a>(
    profile: &DirectProfile,
    ctx: &'a ClientCtx,
) -> CoreResult<&'a Provider> {
    ctx.providers
        .iter()
        .find(|provider| provider.id == profile.provider_id)
        .ok_or_else(|| CoreError::NotFound("直连 Provider 不存在，请重新选择".into()))
}

pub fn direct_startup_model(
    profile: &DirectProfile,
    provider: &Provider,
    ctx: &ClientCtx,
) -> CoreResult<String> {
    if let Some(model) = profile
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        return Ok(model.to_owned());
    }
    provider_models(&provider.id, ctx)
        .first()
        .map(|model| model.upstream_model.clone())
        .filter(|model| !model.is_empty())
        .ok_or_else(|| {
            CoreError::Validation("该 Provider 尚未维护上游模型，无法使用 Provider 模型".into())
        })
}

/// Provider id used for a direct terminal route.
pub fn direct_provider_id(provider_id: &str) -> String {
    let safe: String = provider_id
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect();
    format!("{DIRECT_PROVIDER_PREFIX}{safe}")
}

pub fn direct_name(provider_id: &str) -> String {
    format!("{DIRECT_NAME_PREFIX}{provider_id}")
}

pub fn is_owned_direct_id(id: &str) -> bool {
    id.starts_with(DIRECT_PROVIDER_PREFIX)
}

pub fn is_owned_direct(id: &str, name: &str) -> bool {
    is_owned_direct_id(id) && name.starts_with(DIRECT_NAME_PREFIX)
}

pub fn protocol_api(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Anthropic => "anthropic-messages",
        Protocol::Responses => "openai-responses",
        Protocol::OpenaiChat => "openai-completions",
    }
}

pub fn npm_package(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::OpenaiChat => "@ai-sdk/openai-compatible",
        Protocol::Responses => "@ai-sdk/openai",
        Protocol::Anthropic => "@ai-sdk/anthropic",
    }
}

fn declared_input(capabilities: &Capabilities) -> Vec<crate::domain::Modality> {
    capabilities
        .input_modalities
        .clone()
        .unwrap_or_else(|| vec![crate::domain::Modality::Text])
}

fn declared_output(capabilities: &Capabilities) -> Vec<crate::domain::Modality> {
    capabilities
        .output_modalities
        .clone()
        .unwrap_or_else(|| vec![crate::domain::Modality::Text])
}

/// Input modalities limited to what every managed client can express.
pub fn input_modalities(capabilities: &Capabilities) -> Vec<crate::domain::Modality> {
    declared_input(capabilities)
        .into_iter()
        .filter(|modality| {
            matches!(
                modality,
                crate::domain::Modality::Text | crate::domain::Modality::Image
            )
        })
        .collect()
}

/// Input modalities after translation; same-protocol routes keep the declared set.
pub fn input_for(
    capabilities: &Capabilities,
    client: Protocol,
    upstream: Protocol,
) -> Vec<crate::domain::Modality> {
    if client == upstream {
        declared_input(capabilities)
    } else {
        input_modalities(capabilities)
    }
}

/// Output modalities after translation; cross-protocol routes only carry text.
pub fn output_for(
    capabilities: &Capabilities,
    client: Protocol,
    upstream: Protocol,
) -> Vec<crate::domain::Modality> {
    if client == upstream {
        declared_output(capabilities)
    } else {
        declared_output(capabilities)
            .into_iter()
            .filter(|modality| *modality == crate::domain::Modality::Text)
            .collect()
    }
}
