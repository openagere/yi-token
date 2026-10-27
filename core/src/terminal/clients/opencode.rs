use super::{
    PROXY_PROVIDER_ID, direct_name, direct_provider, direct_provider_id, direct_startup_model,
    input_for, is_owned_direct, is_owned_direct_id, npm_package, output_for, provider_models,
    proxy_model_caps, resolved_models,
};
use crate::domain::{
    AUTO_MODEL, Capabilities, Client, DirectProfile, Protocol, ProxyProfile, RouteMode,
};
use crate::error::{CoreError, CoreResult};
use crate::terminal::doc::{Doc, DocFormat};
use crate::terminal::paths;
use crate::terminal::spec::{ClientCtx, ClientSpec};
use serde_json::{Value, json};
use std::path::PathBuf;

pub struct OpenCode;

fn variant(protocol: Protocol, level: &str) -> Value {
    match protocol {
        Protocol::Responses => json!({
            "reasoningEffort": level,
            "reasoningSummary": "auto",
            "include": ["reasoning.encrypted_content"],
            "forceReasoning": true
        }),
        Protocol::Anthropic => json!({"effort": level}),
        Protocol::OpenaiChat => json!({"reasoningEffort": level}),
    }
}

fn default_option(protocol: Protocol, level: &str) -> Value {
    match protocol {
        Protocol::Responses => json!({"reasoningEffort": level, "forceReasoning": true}),
        Protocol::Anthropic => json!({"effort": level}),
        Protocol::OpenaiChat => json!({"reasoningEffort": level}),
    }
}

fn model_entry(
    name: &str,
    caps: &Capabilities,
    client_protocol: Protocol,
    upstream: Protocol,
) -> Value {
    let mut entry = json!({"name": name});
    entry["modalities"] = json!({
        "input": input_for(caps, client_protocol, upstream),
        "output": output_for(caps, client_protocol, upstream)
    });
    if let (Some(context), Some(output)) = (caps.context_window, caps.max_output_tokens) {
        entry["limit"] = json!({"context": context, "output": output});
    }
    let levels = caps.efforts_for(client_protocol, upstream);
    if !levels.is_empty() {
        entry["variants"] = Value::Object(
            levels
                .iter()
                .map(|level| (level.clone(), variant(client_protocol, level)))
                .collect(),
        );
        if let Some(default) = caps
            .effort
            .default
            .as_ref()
            .filter(|level| levels.contains(level))
        {
            entry["options"] = default_option(client_protocol, default);
        }
    }
    entry
}

fn config_path(ctx: &ClientCtx) -> (PathBuf, DocFormat) {
    if let Some(path) = paths::env_path(ctx, "OPENCODE_CONFIG") {
        let format = if path
            .extension()
            .is_some_and(|extension| extension == "jsonc")
        {
            DocFormat::Jsonc
        } else {
            DocFormat::Json
        };
        return (path, format);
    }
    let dir = paths::config_home(ctx).join("opencode");
    let jsonc = dir.join("opencode.jsonc");
    if jsonc.exists() {
        (jsonc, DocFormat::Jsonc)
    } else {
        (dir.join("opencode.json"), DocFormat::Json)
    }
}

impl ClientSpec for OpenCode {
    fn client(&self) -> Client {
        Client::Opencode
    }

    fn files(&self, ctx: &ClientCtx) -> Vec<(PathBuf, DocFormat)> {
        let (path, format) = config_path(ctx);
        vec![(path, format)]
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
        doc.retain_in(&["provider"], |id, provider| {
            let name = provider
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            !is_owned_direct(id, name)
        })?;
        if let Some(existing) = doc.get(&["provider", PROXY_PROVIDER_ID])
            && existing.get("name").and_then(Value::as_str).is_some()
            && existing.get("name").and_then(Value::as_str) != Some("yi-token")
        {
            return Err(CoreError::Conflict(
                "OpenCode Provider 标识 yitoken 已被其他配置占用".into(),
            ));
        }
        doc.set(&["$schema"], json!("https://opencode.ai/config.json"))?;
        doc.set(&["model"], json!(format!("{PROXY_PROVIDER_ID}/{default}")))?;
        doc.set(
            &["provider", PROXY_PROVIDER_ID, "npm"],
            json!(npm_package(profile.protocol)),
        )?;
        doc.set(&["provider", PROXY_PROVIDER_ID, "name"], json!("yi-token"))?;
        let endpoint =
            super::super::spec::proxy_endpoint(Client::Opencode, profile.protocol, &ctx.proxy_base);
        doc.set(
            &["provider", PROXY_PROVIDER_ID, "options", "baseURL"],
            json!(endpoint),
        )?;
        doc.set(
            &["provider", PROXY_PROVIDER_ID, "options", "apiKey"],
            json!("yi-token"),
        )?;
        let (caps, upstream) = proxy_model_caps(ctx, profile.protocol);
        let mut map = serde_json::Map::new();
        map.insert(
            AUTO_MODEL.to_owned(),
            model_entry(AUTO_MODEL, &caps, profile.protocol, upstream),
        );
        if !auto {
            for resolved in &resolved {
                map.insert(
                    resolved.selection.model.clone(),
                    model_entry(
                        &resolved.selection.model,
                        resolved.capabilities,
                        profile.protocol,
                        resolved.provider.provider_type,
                    ),
                );
            }
        }
        let models = Value::Object(map);
        doc.set(&["provider", PROXY_PROVIDER_ID, "models"], models)?;
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
        let name = direct_name(&provider.id);
        let doc = &mut docs[0];
        if let Some(existing) = doc.get(&["provider", id.as_str()])
            && existing.get("name").and_then(Value::as_str) != Some(name.as_str())
        {
            return Err(CoreError::Conflict(format!(
                "OpenCode Provider 标识 {id} 已被其他配置占用"
            )));
        }
        doc.retain_in(&["provider"], |key, value| {
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
        doc.set(&["$schema"], json!("https://opencode.ai/config.json"))?;
        doc.set(
            &["provider", id.as_str(), "npm"],
            json!(npm_package(provider.provider_type)),
        )?;
        doc.set(&["provider", id.as_str(), "name"], json!(name.as_str()))?;
        let base = if provider.provider_type == Protocol::Anthropic {
            format!("{}/v1", provider.base_url.trim_end_matches('/'))
        } else {
            provider.base_url.trim_end_matches('/').to_owned()
        };
        doc.set(
            &["provider", id.as_str(), "options", "baseURL"],
            json!(base),
        )?;
        if provider.api_key.trim().is_empty() {
            doc.remove(&["provider", id.as_str(), "options", "apiKey"])?;
        } else {
            doc.set(
                &["provider", id.as_str(), "options", "apiKey"],
                json!(provider.api_key.as_str()),
            )?;
        }
        let selected = direct_startup_model(profile, provider, ctx)?;
        let mut models = serde_json::Map::new();
        for model in provider_models(&provider.id, ctx) {
            models.insert(
                model.upstream_model.clone(),
                model_entry(
                    &model.upstream_model,
                    &model.capabilities,
                    provider.provider_type,
                    provider.provider_type,
                ),
            );
        }
        models
            .entry(selected.clone())
            .or_insert_with(|| json!({"name": selected}));
        doc.set(&["provider", id.as_str(), "models"], Value::Object(models))?;
        doc.set(&["model"], json!(format!("{id}/{selected}")))?;
        let secrets = if provider.api_key.trim().is_empty() {
            Vec::new()
        } else {
            vec![provider.api_key.clone()]
        };
        Ok(secrets)
    }

    fn render_clear(&self, docs: &mut [Doc], _ctx: &ClientCtx) -> CoreResult<Vec<bool>> {
        let doc = &mut docs[0];
        let mut changed = false;
        if doc
            .get(&["provider", PROXY_PROVIDER_ID, "name"])
            .and_then(|value| value.as_str().map(str::to_owned))
            .as_deref()
            == Some("yi-token")
        {
            doc.remove(&["provider", PROXY_PROVIDER_ID])?;
            changed = true;
        }
        let before = doc.get(&["provider"]);
        doc.retain_in(&["provider"], |id, value| {
            let name = value
                .get("name")
                .and_then(|item| item.as_str())
                .unwrap_or_default();
            !is_owned_direct(id, name)
        })?;
        if doc.get(&["provider"]) != before {
            changed = true;
        }
        if doc
            .get(&["model"])
            .and_then(|value| value.as_str().map(str::to_owned))
            .is_some_and(|model| {
                model.starts_with("yitoken/") || model.starts_with("yitoken_direct_")
            })
        {
            doc.remove(&["model"])?;
            changed = true;
        }
        Ok(vec![changed])
    }
}
