use super::{
    PROXY_PROVIDER_ID, direct_provider, direct_provider_id, direct_startup_model, input_modalities,
    is_owned_direct, is_owned_direct_id, provider_models, proxy_model_caps, resolved_models,
};
use crate::domain::{
    AUTO_MODEL, Capabilities, Client, DirectProfile, Protocol, ProxyProfile, RouteMode, Support,
};
use crate::error::{CoreError, CoreResult};
use crate::terminal::doc::{Doc, DocFormat};
use crate::terminal::spec::{ClientCtx, ClientSpec};
use serde_json::{Value, json};
use std::path::PathBuf;

pub struct Codex;

const BASE_INSTRUCTIONS: &str = "You are a coding assistant. Work with the user in the shared workspace, use the available tools to inspect and edit files, and verify your changes.";
const CONFIG_FILE: &str = "config.toml";
const CATALOG_FILE: &str = "yi-token-models.json";

fn config_dir(ctx: &ClientCtx) -> PathBuf {
    crate::terminal::paths::env_path(ctx, "CODEX_HOME").unwrap_or_else(|| ctx.home.join(".codex"))
}

fn catalog_entry(
    slug: &str,
    upstream: Protocol,
    caps: &Capabilities,
    priority: usize,
    description: String,
) -> Value {
    let levels = caps.efforts_for(Protocol::Responses, upstream);
    let mut entry = json!({
        "slug": slug,
        "display_name": slug,
        "description": description,
        "supported_reasoning_levels": levels.iter().map(|level| json!({"effort": level, "description": format!("{level} effort")})).collect::<Vec<_>>(),
        "default_reasoning_level": caps.effort.default.as_ref().filter(|level| levels.contains(level)),
        "shell_type": "default",
        "visibility": "list",
        "supported_in_api": true,
        "priority": priority,
        "availability_nux": null,
        "upgrade": null,
        "base_instructions": BASE_INSTRUCTIONS,
        "supports_reasoning_summaries": false,
        "support_verbosity": false,
        "default_verbosity": null,
        "apply_patch_tool_type": null,
        "truncation_policy": {"mode": "bytes", "limit": 10000},
        "supports_parallel_tool_calls": false,
        "experimental_supported_tools": [],
        "input_modalities": input_modalities(caps),
    });
    if let Some(context) = caps.context_window {
        entry["context_window"] = json!(context);
        entry["max_context_window"] = json!(context);
    }
    entry
}

fn default_selection<'a>(
    profile: &'a ProxyProfile,
    resolved: &'a [super::ResolvedModel<'a>],
) -> Option<&'a super::ResolvedModel<'a>> {
    let name = profile.default_model.as_deref()?;
    resolved
        .iter()
        .find(|resolved| resolved.selection.model == name)
}

fn set_reasoning(
    config: &mut Doc,
    caps: Option<&Capabilities>,
    upstream: Protocol,
) -> CoreResult<()> {
    let level = caps
        .filter(|caps| caps.effort.support == Support::Supported)
        .and_then(|caps| {
            let levels = caps.efforts_for(Protocol::Responses, upstream);
            caps.effort
                .default
                .as_ref()
                .filter(|level| levels.contains(level))
        });
    match level {
        Some(level) => config.set(&["model_reasoning_effort"], json!(level)),
        None => config.remove(&["model_reasoning_effort"]),
    }
}

fn remove_owned_direct(doc: &mut Doc) -> CoreResult<()> {
    doc.retain_in(&["model_providers"], |id, value| {
        let name = value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        !is_owned_direct(id, name)
    })
}

impl ClientSpec for Codex {
    fn client(&self) -> Client {
        Client::Codex
    }

    fn files(&self, ctx: &ClientCtx) -> Vec<(PathBuf, DocFormat)> {
        let dir = config_dir(ctx);
        vec![
            (dir.join(CONFIG_FILE), DocFormat::Toml),
            (dir.join(CATALOG_FILE), DocFormat::Json),
        ]
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
        let config = &mut docs[0];
        remove_owned_direct(config)?;
        if let Some(existing) = config.get(&["model_providers", PROXY_PROVIDER_ID])
            && existing.get("name").and_then(Value::as_str).is_some()
            && existing.get("name").and_then(Value::as_str) != Some("yi-token")
        {
            return Err(CoreError::Conflict(
                "Codex Provider 标识 yitoken 已被其他配置占用".into(),
            ));
        }
        config.set(&["model"], json!(default))?;
        config.set(&["model_provider"], json!(PROXY_PROVIDER_ID))?;
        config.set(
            &["model_catalog_json"],
            json!(config_dir(ctx).join(CATALOG_FILE).display().to_string()),
        )?;

        let base =
            super::super::spec::proxy_endpoint(Client::Codex, Protocol::Responses, &ctx.proxy_base);
        for key in [
            "env_key",
            "env_key_instructions",
            "experimental_bearer_token",
            "http_headers",
            "env_http_headers",
        ] {
            config.remove(&["model_providers", PROXY_PROVIDER_ID, key])?;
        }
        config.set(
            &["model_providers", PROXY_PROVIDER_ID, "name"],
            json!("yi-token"),
        )?;
        config.set(
            &["model_providers", PROXY_PROVIDER_ID, "base_url"],
            json!(base),
        )?;
        config.set(
            &["model_providers", PROXY_PROVIDER_ID, "wire_api"],
            json!("responses"),
        )?;
        config.set(
            &["model_providers", PROXY_PROVIDER_ID, "requires_openai_auth"],
            json!(false),
        )?;

        let (auto_capabilities, auto_upstream) = proxy_model_caps(ctx, Protocol::Responses);
        let auto_entry = catalog_entry(
            AUTO_MODEL,
            auto_upstream,
            &auto_capabilities,
            0,
            "yi-token".to_owned(),
        );
        let mut models: Vec<Value> = vec![auto_entry];
        if auto || default == AUTO_MODEL {
            set_reasoning(config, Some(&auto_capabilities), auto_upstream)?;
        } else {
            let selection = default_selection(profile, &resolved);
            set_reasoning(
                config,
                selection.map(|selection| selection.capabilities),
                selection.map_or(Protocol::Responses, |selection| selection.upstream()),
            )?;
        }
        if !auto {
            models.extend(resolved.iter().enumerate().map(|(index, resolved)| {
                catalog_entry(
                    &resolved.selection.model,
                    resolved.upstream(),
                    resolved.capabilities,
                    index + 1,
                    "yi-token".to_owned(),
                )
            }));
        }
        docs[1].set(&["models"], Value::Array(models))?;
        Ok(Vec::new())
    }

    fn render_direct(
        &self,
        docs: &mut [Doc],
        profile: &DirectProfile,
        ctx: &ClientCtx,
    ) -> CoreResult<Vec<String>> {
        let provider = direct_provider(profile, ctx)?;
        let id = direct_provider_id(&provider.id);
        let name = format!("yi-token direct:{}:{}", provider.id, provider.name);
        let config = &mut docs[0];

        if let Some(existing) = config.get(&["model_providers", id.as_str()])
            && existing.get("name").and_then(Value::as_str) != Some(name.as_str())
        {
            return Err(CoreError::Conflict(format!(
                "Codex Provider 标识 {id} 已被其他配置占用"
            )));
        }
        config.retain_in(&["model_providers"], |key, value| {
            if !is_owned_direct_id(key) {
                return true;
            }
            if key == id {
                return true;
            }
            let owned_name = value
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            !owned_name.starts_with(super::DIRECT_NAME_PREFIX)
        })?;
        if config
            .get(&["model_providers", PROXY_PROVIDER_ID, "name"])
            .and_then(|value| value.as_str().map(str::to_owned))
            .as_deref()
            == Some("yi-token")
        {
            config.remove(&["model_providers", PROXY_PROVIDER_ID])?;
        }
        if config
            .get(&["model_catalog_json"])
            .and_then(|value| value.as_str().map(str::to_owned))
            .is_some_and(|catalog| {
                catalog == config_dir(ctx).join(CATALOG_FILE).display().to_string()
            })
        {
            config.remove(&["model_catalog_json"])?;
        }

        config.set(&["model_provider"], json!(id.as_str()))?;
        config.set(
            &["model_providers", id.as_str(), "name"],
            json!(name.as_str()),
        )?;
        config.set(
            &["model_providers", id.as_str(), "base_url"],
            json!(provider.base_url.trim_end_matches('/')),
        )?;
        config.set(
            &["model_providers", id.as_str(), "wire_api"],
            json!("responses"),
        )?;
        config.set(
            &["model_providers", id.as_str(), "requires_openai_auth"],
            json!(false),
        )?;
        for key in [
            "env_key",
            "env_key_instructions",
            "http_headers",
            "env_http_headers",
        ] {
            config.remove(&["model_providers", id.as_str(), key])?;
        }
        if provider.api_key.trim().is_empty() {
            config.remove(&["model_providers", id.as_str(), "experimental_bearer_token"])?;
        } else {
            config.set(
                &["model_providers", id.as_str(), "experimental_bearer_token"],
                json!(provider.api_key.as_str()),
            )?;
        }

        let models = provider_models(&provider.id, ctx);
        if profile.model_source == crate::domain::ModelSource::Provider {
            let selected = direct_startup_model(profile, provider, ctx)?;
            let mut entries = Vec::new();
            for (priority, model) in models.iter().enumerate() {
                entries.push(catalog_entry(
                    &model.upstream_model,
                    provider.provider_type,
                    &model.capabilities,
                    priority,
                    format!("{} · {}", provider.name, model.upstream_model),
                ));
            }
            if !entries
                .iter()
                .any(|entry| entry["slug"].as_str() == Some(selected.as_str()))
            {
                entries.push(json!({
                    "slug": selected,
                    "display_name": selected,
                    "description": format!("{} · {}", provider.name, selected),
                    "supported_reasoning_levels": [],
                    "default_reasoning_level": null,
                    "shell_type": "default",
                    "visibility": "list",
                    "supported_in_api": true,
                    "priority": entries.len(),
                    "availability_nux": null,
                    "upgrade": null,
                    "base_instructions": BASE_INSTRUCTIONS,
                    "supports_reasoning_summaries": false,
                    "support_verbosity": false,
                    "default_verbosity": null,
                    "apply_patch_tool_type": null,
                    "truncation_policy": {"mode": "bytes", "limit": 10000},
                    "supports_parallel_tool_calls": false,
                    "experimental_supported_tools": [],
                    "input_modalities": ["text"],
                }));
            }
            config.set(&["model"], json!(selected))?;
            config.set(
                &["model_catalog_json"],
                json!(config_dir(ctx).join(CATALOG_FILE).display().to_string()),
            )?;
            docs[1].set(&["models"], Value::Array(entries))?;
        } else {
            config.remove(&["model"])?;
            config.remove(&["model_reasoning_effort"])?;
            if config
                .get(&["model_catalog_json"])
                .and_then(|value| value.as_str().map(str::to_owned))
                .is_some_and(|catalog| {
                    catalog == config_dir(ctx).join(CATALOG_FILE).display().to_string()
                })
            {
                config.remove(&["model_catalog_json"])?;
            }
        }

        let secrets = if provider.api_key.trim().is_empty() {
            Vec::new()
        } else {
            vec![provider.api_key.clone()]
        };
        Ok(secrets)
    }

    fn render_clear(&self, docs: &mut [Doc], ctx: &ClientCtx) -> CoreResult<Vec<bool>> {
        let config = &mut docs[0];
        let mut config_changed = false;
        let provider_id = config
            .get(&["model_provider"])
            .and_then(|value| value.as_str().map(str::to_owned));
        let owned_provider = provider_id
            .as_deref()
            .is_some_and(|id| id == PROXY_PROVIDER_ID || super::is_owned_direct_id(id));
        let catalog_path = config_dir(ctx).join(CATALOG_FILE).display().to_string();
        let catalog_ours = config
            .get(&["model_catalog_json"])
            .and_then(|value| value.as_str().map(str::to_owned))
            .as_deref()
            == Some(catalog_path.as_str());
        if owned_provider {
            config.remove(&["model"])?;
            config.remove(&["model_provider"])?;
            config.remove(&["model_reasoning_effort"])?;
            config.remove(&["model_catalog_json"])?;
            config_changed = true;
        } else if catalog_ours {
            config.remove(&["model_catalog_json"])?;
            config_changed = true;
        }
        if config
            .get(&["model_providers", PROXY_PROVIDER_ID, "name"])
            .and_then(|value| value.as_str().map(str::to_owned))
            .as_deref()
            == Some("yi-token")
        {
            config.remove(&["model_providers", PROXY_PROVIDER_ID])?;
            config_changed = true;
        }
        let before = config.get(&["model_providers"]);
        remove_owned_direct(config)?;
        if config.get(&["model_providers"]) != before {
            config_changed = true;
        }
        let mut catalog_changed = false;
        if docs.len() > 1 && (owned_provider || catalog_ours) && docs[1].get(&["models"]).is_some()
        {
            docs[1].remove(&["models"])?;
            catalog_changed = true;
        }
        let mut flags = vec![config_changed];
        if docs.len() > 1 {
            flags.push(catalog_changed);
        }
        Ok(flags)
    }
}
