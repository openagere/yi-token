use super::{
    PROXY_PROVIDER_ID, direct_name, direct_provider, direct_provider_id, direct_startup_model,
    input_for, is_owned_direct, is_owned_direct_id, protocol_api, provider_models,
    proxy_model_caps, resolved_models,
};
use crate::domain::{
    AUTO_MODEL, Capabilities, Client, DirectProfile, Modality, Protocol, ProxyProfile, RouteMode,
};
use crate::error::{CoreError, CoreResult};
use crate::terminal::doc::{Doc, DocFormat};
use crate::terminal::spec::{ClientCtx, ClientSpec};
use serde_json::{Map, Value, json};
use std::path::PathBuf;

pub struct Pi;

const MODELS_FILE: &str = "models.json";
const SETTINGS_FILE: &str = "settings.json";
const PI_THINKING_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

fn config_dir(ctx: &ClientCtx) -> PathBuf {
    let Some(path) = crate::terminal::paths::env_path(ctx, "PI_CODING_AGENT_DIR") else {
        return ctx.home.join(".pi").join("agent");
    };
    let text = path.to_string_lossy();
    let Some(rest) = text.strip_prefix('~') else {
        return path;
    };
    let rest = rest.trim_start_matches(['/', '\\']);
    if rest.is_empty() {
        ctx.home.clone()
    } else {
        ctx.home.join(rest)
    }
}

fn models_path(ctx: &ClientCtx) -> PathBuf {
    config_dir(ctx).join(MODELS_FILE)
}

fn settings_path(ctx: &ClientCtx) -> PathBuf {
    config_dir(ctx).join(SETTINGS_FILE)
}

fn model_entry(
    id: &str,
    caps: &Capabilities,
    client_protocol: Protocol,
    upstream: Protocol,
) -> Value {
    let input: Vec<Modality> = input_for(caps, client_protocol, upstream);
    let mut entry = json!({"id": id, "name": id, "input": input});
    if let Some(context) = caps.context_window {
        entry["contextWindow"] = json!(context);
    }
    if let Some(max) = caps.max_output_tokens {
        entry["maxTokens"] = json!(max);
    }
    let supported = caps.efforts_for(client_protocol, upstream);
    let levels: Vec<&str> = PI_THINKING_LEVELS
        .iter()
        .copied()
        .filter(|level| supported.iter().any(|supported| supported == level))
        .collect();
    if !levels.is_empty() {
        entry["reasoning"] = json!(true);
        let mut map = Map::new();
        map.insert("minimal".into(), Value::Null);
        if client_protocol == Protocol::Responses {
            map.insert("off".into(), Value::Null);
        }
        for level in PI_THINKING_LEVELS {
            map.insert(
                level.into(),
                if levels.contains(&level) {
                    json!(level)
                } else {
                    Value::Null
                },
            );
        }
        entry["thinkingLevelMap"] = Value::Object(map);
    }
    entry
}

fn remove_owned_direct(doc: &mut Doc) -> CoreResult<()> {
    doc.retain_in(&["providers"], |id, provider| {
        let name = provider
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        !is_owned_direct(id, name)
    })
}

fn set_settings(doc: &mut Doc, provider: &str, model: &str) -> CoreResult<()> {
    doc.set(&["defaultProvider"], json!(provider))?;
    doc.set(&["defaultModel"], json!(model))
}

impl ClientSpec for Pi {
    fn client(&self) -> Client {
        Client::Pi
    }

    fn files(&self, ctx: &ClientCtx) -> Vec<(PathBuf, DocFormat)> {
        vec![
            (models_path(ctx), DocFormat::Jsonc),
            (settings_path(ctx), DocFormat::Json),
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
        let models = &mut docs[0];
        remove_owned_direct(models)?;
        if let Some(existing) = models.get(&["providers", PROXY_PROVIDER_ID])
            && existing.get("name").and_then(Value::as_str).is_some()
            && existing.get("name").and_then(Value::as_str) != Some("yi-token")
        {
            return Err(CoreError::Conflict(
                "Pi Provider 标识 yitoken 已被其他配置占用".into(),
            ));
        }
        let endpoint =
            super::super::spec::proxy_endpoint(Client::Pi, profile.protocol, &ctx.proxy_base);
        models.set(&["providers", PROXY_PROVIDER_ID, "name"], json!("yi-token"))?;
        models.set(
            &["providers", PROXY_PROVIDER_ID, "baseUrl"],
            json!(endpoint),
        )?;
        models.set(
            &["providers", PROXY_PROVIDER_ID, "api"],
            json!(protocol_api(profile.protocol)),
        )?;
        models.set(
            &["providers", PROXY_PROVIDER_ID, "apiKey"],
            json!("yi-token"),
        )?;
        let (caps, upstream) = proxy_model_caps(ctx, profile.protocol);
        let mut entries = vec![model_entry(AUTO_MODEL, &caps, profile.protocol, upstream)];
        if !auto {
            entries.extend(resolved.iter().map(|resolved| {
                model_entry(
                    &resolved.selection.model,
                    resolved.capabilities,
                    profile.protocol,
                    resolved.upstream(),
                )
            }));
        }
        models.set(
            &["providers", PROXY_PROVIDER_ID, "models"],
            Value::Array(entries),
        )?;
        set_settings(&mut docs[1], PROXY_PROVIDER_ID, &default)?;
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
        let models = &mut docs[0];
        if let Some(existing) = models.get(&["providers", id.as_str()])
            && existing.get("name").and_then(Value::as_str) != Some(name.as_str())
        {
            return Err(CoreError::Conflict(format!(
                "Pi Provider 标识 {id} 已被其他配置占用"
            )));
        }
        models.retain_in(&["providers"], |key, value| {
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
        models.set(
            &["providers", id.as_str(), "api"],
            json!(protocol_api(provider.provider_type)),
        )?;
        models.set(&["providers", id.as_str(), "name"], json!(name.as_str()))?;
        models.set(
            &["providers", id.as_str(), "baseUrl"],
            json!(provider.base_url.trim_end_matches('/')),
        )?;
        if provider.api_key.trim().is_empty() {
            models.remove(&["providers", id.as_str(), "apiKey"])?;
        } else {
            models.set(
                &["providers", id.as_str(), "apiKey"],
                json!(provider.api_key.as_str()),
            )?;
        }
        let selected = direct_startup_model(profile, provider, ctx)?;
        let mut entries: Vec<Value> = provider_models(&provider.id, ctx)
            .into_iter()
            .map(|model| {
                model_entry(
                    &model.upstream_model,
                    &model.capabilities,
                    provider.provider_type,
                    provider.provider_type,
                )
            })
            .collect();
        if !entries
            .iter()
            .any(|entry| entry["id"].as_str() == Some(selected.as_str()))
        {
            entries.push(json!({"id": selected, "name": selected}));
        }
        models.set(&["providers", id.as_str(), "models"], Value::Array(entries))?;
        set_settings(&mut docs[1], &id, &selected)?;
        let secrets = if provider.api_key.trim().is_empty() {
            Vec::new()
        } else {
            vec![provider.api_key.clone()]
        };
        Ok(secrets)
    }

    fn render_clear(&self, docs: &mut [Doc], _ctx: &ClientCtx) -> CoreResult<Vec<bool>> {
        let models = &mut docs[0];
        let mut models_changed = false;
        if models
            .get(&["providers", PROXY_PROVIDER_ID, "name"])
            .and_then(|value| value.as_str().map(str::to_owned))
            .as_deref()
            == Some("yi-token")
        {
            models.remove(&["providers", PROXY_PROVIDER_ID])?;
            models_changed = true;
        }
        let before = models.get(&["providers"]);
        remove_owned_direct(models)?;
        if models.get(&["providers"]) != before {
            models_changed = true;
        }
        let mut settings_changed = false;
        if docs.len() > 1 {
            let settings = &mut docs[1];
            if settings
                .get(&["defaultProvider"])
                .and_then(|value| value.as_str().map(str::to_owned))
                .is_some_and(|provider| {
                    provider == PROXY_PROVIDER_ID || is_owned_direct_id(&provider)
                })
            {
                settings.remove(&["defaultProvider"])?;
                settings.remove(&["defaultModel"])?;
                settings_changed = true;
            }
        }
        let mut flags = vec![models_changed];
        if docs.len() > 1 {
            flags.push(settings_changed);
        }
        Ok(flags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::Path;

    fn ctx(home: &Path) -> ClientCtx {
        ClientCtx {
            proxy_base: "http://127.0.0.1:11435".into(),
            providers: Vec::new(),
            models: Vec::new(),
            standard_models: Vec::new(),
            active_engine: None,
            home: home.to_path_buf(),
            env: HashMap::new(),
        }
    }

    #[test]
    fn tilde_env_paths_expand_under_home() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(dir.path());
        ctx.env
            .insert("PI_CODING_AGENT_DIR".into(), "~/custom".into());
        assert_eq!(config_dir(&ctx), dir.path().join("custom"));
    }
}
