use super::{
    direct_provider, direct_startup_model, provider_models, proxy_model_caps, resolved_models,
};
use crate::domain::{
    AUTO_MODEL, Capabilities, Client, DirectProfile, ModelSource, Protocol, ProxyProfile, RouteMode,
};
use crate::error::CoreResult;
use crate::terminal::doc::{Doc, DocFormat};
use crate::terminal::spec::{ClientCtx, ClientSpec};
use serde_json::{Value, json};
use std::path::PathBuf;

pub struct ClaudeCode;

const SETTINGS_FILE: &str = "settings.json";

fn set_model_limits(doc: &mut Doc, caps: &Capabilities) -> CoreResult<()> {
    match caps.context_window {
        Some(context) => doc.set(
            &["env", "CLAUDE_CODE_MAX_CONTEXT_TOKENS"],
            json!(context.to_string()),
        )?,
        None => doc.remove(&["env", "CLAUDE_CODE_MAX_CONTEXT_TOKENS"])?,
    }
    match caps.max_output_tokens {
        Some(output) => doc.set(
            &["env", "CLAUDE_CODE_MAX_OUTPUT_TOKENS"],
            json!(output.to_string()),
        )?,
        None => doc.remove(&["env", "CLAUDE_CODE_MAX_OUTPUT_TOKENS"])?,
    }
    Ok(())
}

fn clear_model_limits(doc: &mut Doc) -> CoreResult<()> {
    doc.remove(&["env", "CLAUDE_CODE_MAX_CONTEXT_TOKENS"])?;
    doc.remove(&["env", "CLAUDE_CODE_MAX_OUTPUT_TOKENS"])?;
    Ok(())
}

fn config_dir(ctx: &ClientCtx) -> PathBuf {
    crate::terminal::paths::env_path(ctx, "CLAUDE_CONFIG_DIR")
        .unwrap_or_else(|| ctx.home.join(".claude"))
}

impl ClientSpec for ClaudeCode {
    fn client(&self) -> Client {
        Client::ClaudeCode
    }

    fn files(&self, ctx: &ClientCtx) -> Vec<(PathBuf, DocFormat)> {
        vec![(config_dir(ctx).join(SETTINGS_FILE), DocFormat::Json)]
    }

    fn render_proxy(
        &self,
        docs: &mut [Doc],
        profile: &ProxyProfile,
        ctx: &ClientCtx,
    ) -> CoreResult<Vec<String>> {
        let auto = profile.route_mode == RouteMode::Auto;
        let resolved = if auto {
            Vec::new()
        } else {
            resolved_models(profile, ctx)?
        };
        let default = profile
            .default_model
            .clone()
            .filter(|model| !model.trim().is_empty())
            .unwrap_or_else(|| AUTO_MODEL.to_owned());
        let doc = &mut docs[0];
        let endpoint = super::super::spec::proxy_endpoint(
            Client::ClaudeCode,
            Protocol::Anthropic,
            &ctx.proxy_base,
        );
        doc.set(&["model"], json!(default))?;
        doc.set(&["env", "ANTHROPIC_BASE_URL"], json!(endpoint))?;
        doc.set(&["env", "ANTHROPIC_AUTH_TOKEN"], json!("yi-token"))?;
        let (caps, _) = proxy_model_caps(ctx, Protocol::Anthropic);
        set_model_limits(doc, &caps)?;
        doc.set(&["modelPicker", "replaceBuiltInOptions"], json!(true))?;
        let mut options: Vec<Value> = vec![json!({"model": AUTO_MODEL, "label": AUTO_MODEL})];
        if !auto {
            options.extend(resolved.iter().map(|resolved| {
                json!({
                    "model": resolved.selection.model,
                    "label": resolved.selection.model
                })
            }));
        }
        doc.set(&["modelPicker", "options"], Value::Array(options))?;
        Ok(Vec::new())
    }

    fn render_direct(
        &self,
        docs: &mut [Doc],
        profile: &DirectProfile,
        ctx: &ClientCtx,
    ) -> CoreResult<Vec<String>> {
        let provider = direct_provider(profile, ctx)?;
        let doc = &mut docs[0];
        clear_model_limits(doc)?;
        doc.set(
            &["env", "ANTHROPIC_BASE_URL"],
            json!(provider.base_url.trim_end_matches('/')),
        )?;
        if provider.api_key.trim().is_empty() {
            if doc
                .get(&["env", "ANTHROPIC_AUTH_TOKEN"])
                .and_then(|value| value.as_str().map(str::to_owned))
                .as_deref()
                == Some("yi-token")
            {
                doc.remove(&["env", "ANTHROPIC_AUTH_TOKEN"])?;
            }
        } else {
            doc.set(
                &["env", "ANTHROPIC_AUTH_TOKEN"],
                json!(provider.api_key.as_str()),
            )?;
        }

        if profile.model_source == ModelSource::Provider {
            let selected = direct_startup_model(profile, provider, ctx)?;
            doc.set(&["model"], json!(selected.as_str()))?;
            let mut options: Vec<Value> = provider_models(&provider.id, ctx)
                .into_iter()
                .map(|model| {
                    json!({
                        "model": model.upstream_model,
                        "label": model.upstream_model
                    })
                })
                .collect();
            if !options
                .iter()
                .any(|option| option["model"].as_str() == Some(selected.as_str()))
            {
                options.push(json!({"model": selected, "label": selected}));
            }
            doc.set(&["modelPicker", "replaceBuiltInOptions"], json!(true))?;
            doc.set(&["modelPicker", "options"], Value::Array(options))?;
        } else {
            doc.remove(&["model"])?;
            doc.remove(&["modelPicker"])?;
        }
        let secrets = if provider.api_key.trim().is_empty() {
            Vec::new()
        } else {
            vec![provider.api_key.clone()]
        };
        Ok(secrets)
    }

    fn render_clear(&self, docs: &mut [Doc], ctx: &ClientCtx) -> CoreResult<Vec<bool>> {
        let doc = &mut docs[0];
        let base = doc
            .get(&["env", "ANTHROPIC_BASE_URL"])
            .and_then(|value| value.as_str().map(str::to_owned));
        let token = doc
            .get(&["env", "ANTHROPIC_AUTH_TOKEN"])
            .and_then(|value| value.as_str().map(str::to_owned));
        let proxy = base
            .as_deref()
            .is_some_and(|url| url.contains("/clients/claude-code"));
        let proxy_token = token.as_deref() == Some("yi-token");
        let direct = ctx.providers.iter().any(|provider| {
            let normalized = provider.base_url.trim_end_matches('/');
            base.as_deref() == Some(normalized)
                && (token.as_deref() == Some(provider.api_key.as_str())
                    || (provider.api_key.trim().is_empty() && token.is_none()))
        });
        if proxy || proxy_token {
            if proxy {
                doc.remove(&["env", "ANTHROPIC_BASE_URL"])?;
            }
            if proxy_token {
                doc.remove(&["env", "ANTHROPIC_AUTH_TOKEN"])?;
            }
            clear_model_limits(doc)?;
            doc.remove(&["modelPicker"])?;
            if doc
                .get(&["model"])
                .and_then(|value| value.as_str().map(str::to_owned))
                .as_deref()
                == Some(AUTO_MODEL)
            {
                doc.remove(&["model"])?;
            }
            return Ok(vec![true]);
        }
        if direct {
            doc.remove(&["env", "ANTHROPIC_BASE_URL"])?;
            doc.remove(&["env", "ANTHROPIC_AUTH_TOKEN"])?;
            doc.remove(&["modelPicker"])?;
            doc.remove(&["model"])?;
            return Ok(vec![true]);
        }
        Ok(vec![false])
    }
}
