use super::{
    PROXY_PROVIDER_ID, direct_name, direct_provider, direct_provider_id, direct_startup_model,
    input_for, is_owned_direct_id, protocol_api, provider_models, proxy_model_caps,
    resolved_models,
};
use crate::domain::{
    AUTO_MODEL, Capabilities, Client, DirectProfile, Protocol, ProxyProfile, RouteMode,
};
use crate::error::{CoreError, CoreResult};
use crate::terminal::doc::{Doc, DocFormat};
use crate::terminal::spec::{ClientCtx, ClientSpec};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

pub struct DeepSeekHarness;

const PLUGIN_ID: &str = "llm-pi-ai";
const PLUGIN_NAME: &str = "@deepseek-ai/dsh-llm-pi-ai";
const AGENT_LOOP_ID: &str = "agent-loop";
const CREDENTIALS_FILE: &str = ".credentials.yaml";
const PROXY_KEY_REFERENCE: &str = "YI_TOKEN_PROXY";
const DIRECT_KEY_REFERENCE_PREFIX: &str = "YI_TOKEN_DIRECT_";

fn home(ctx: &ClientCtx) -> PathBuf {
    let Some(path) = crate::terminal::paths::env_path(ctx, "DSH_HOME") else {
        return ctx.home.join(".dsh");
    };
    crate::terminal::paths::expand_home(ctx, path)
}

fn profiles(home: &Path) -> Vec<&'static str> {
    let mut profiles = Vec::new();
    for profile in ["desktop", "web"] {
        if home.join("profiles").join(profile).is_dir() {
            profiles.push(profile);
        }
    }
    if profiles.is_empty() {
        profiles.push("desktop");
    }
    profiles
}

fn patch_paths(home: &Path) -> Vec<PathBuf> {
    profiles(home)
        .into_iter()
        .map(|profile| home.join("profiles").join(profile).join("cordis.patch.yml"))
        .collect()
}

fn plugin_index(doc: &mut Vec<Value>) -> usize {
    if let Some(index) = doc.iter().position(|entry| {
        entry.get("id").and_then(Value::as_str) == Some(PLUGIN_ID)
            || entry.get("name").and_then(Value::as_str) == Some(PLUGIN_NAME)
            || entry.get("name").and_then(Value::as_str) == Some(PLUGIN_ID)
    }) {
        return index;
    }
    doc.push(json!({"id": PLUGIN_ID, "config": {"providers": {}}}));
    doc.len() - 1
}

fn patch_array(doc: &mut Doc) -> CoreResult<&mut Vec<Value>> {
    let Doc::Yaml(value) = doc else {
        return Err(CoreError::Internal(
            "DeepSeek Harness 配置必须是 YAML".into(),
        ));
    };
    if value.is_null() {
        *value = json!([]);
    }
    value.as_array_mut().ok_or_else(|| {
        CoreError::Validation(
            "DeepSeek Harness cordis.patch.yml 必须是条目列表，未做任何修改".into(),
        )
    })
}

fn object_at<'a>(value: &'a mut Value, key: &str) -> CoreResult<&'a mut Map<String, Value>> {
    if !value.is_object() {
        return Err(CoreError::Validation(format!(
            "配置 {key} 不是对象，未做任何修改"
        )));
    }
    let map = value.as_object_mut().expect("checked object");
    let entry = map.entry(key.to_owned()).or_insert_with(|| json!({}));
    entry
        .as_object_mut()
        .ok_or_else(|| CoreError::Validation(format!("配置 {key} 不是对象，未做任何修改")))
}

fn providers(doc: &mut Vec<Value>) -> CoreResult<&mut Map<String, Value>> {
    let index = plugin_index(doc);
    let entry = &mut doc[index];
    if !entry.is_object() {
        return Err(CoreError::Validation(
            "DeepSeek Harness llm-pi-ai 条目不是对象，未做任何修改".into(),
        ));
    }
    let config = object_at(entry, "config")?;
    let value = config.entry("providers").or_insert_with(|| json!({}));
    value
        .as_object_mut()
        .ok_or_else(|| CoreError::Validation("配置 providers 不是对象，未做任何修改".into()))
}

fn update_default_agent(doc: &mut [Value], provider: &str, model: &str) {
    for entry in doc.iter_mut() {
        let agent_loop = entry.get("id").and_then(Value::as_str) == Some(AGENT_LOOP_ID)
            || entry
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| name.ends_with(AGENT_LOOP_ID));
        if !agent_loop {
            continue;
        }
        let Some(agents) = entry
            .get_mut("config")
            .and_then(|config| config.get_mut("agents"))
            .and_then(Value::as_array_mut)
        else {
            continue;
        };
        for agent in agents {
            if agent.get("id").and_then(Value::as_str) == Some("main") {
                agent["provider"] = json!(provider);
                agent["model"] = json!(model);
            }
        }
    }
}

fn model_entry(
    id: &str,
    caps: &Capabilities,
    client_protocol: Protocol,
    upstream: Protocol,
) -> Value {
    let input = input_for(caps, client_protocol, upstream);
    let mut entry = json!({"id": id, "name": id, "input": input});
    if let Some(context) = caps.context_window {
        entry["contextWindow"] = json!(context);
    }
    if let Some(max) = caps.max_output_tokens {
        entry["maxTokens"] = json!(max);
    }
    let supported = caps.efforts_for(client_protocol, upstream);
    if !supported.is_empty() {
        let mut efforts = Map::new();
        efforts.insert("off".into(), Value::Null);
        for level in super::THINKING_LEVELS {
            if supported.iter().any(|supported| supported == level) {
                efforts.insert(level.into(), json!(level));
            }
        }
        entry["reasoningEfforts"] = Value::Object(efforts);
    }
    entry
}

fn direct_key_reference(provider_id: &str) -> String {
    let safe: String = provider_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("{DIRECT_KEY_REFERENCE_PREFIX}{safe}")
}

fn prune_direct(providers: &mut Map<String, Value>, keep: Option<&str>) {
    providers.retain(|id, entry| {
        if !is_owned_direct_id(id) {
            return true;
        }
        if keep == Some(id.as_str()) {
            return true;
        }
        let name = entry
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        !name.starts_with(super::DIRECT_NAME_PREFIX)
    });
}

fn credential_doc(docs: &mut [Doc]) -> CoreResult<&mut Map<String, Value>> {
    let doc = docs
        .last_mut()
        .ok_or_else(|| CoreError::Internal("缺少 dsh 凭据文件".into()))?;
    let Doc::Yaml(value) = doc else {
        return Err(CoreError::Internal("dsh 凭据文件必须是 YAML".into()));
    };
    if value.is_null() {
        *value = json!({});
    }
    value.as_object_mut().ok_or_else(|| {
        CoreError::Validation(
            "dsh 凭据存储 .credentials.yaml 必须是映射，请先修复后重新应用".into(),
        )
    })
}

fn provision_credentials(
    docs: &mut [Doc],
    reference: &str,
    value: &str,
    current_direct: Option<&str>,
) -> CoreResult<()> {
    let document = credential_doc(docs)?;
    if let Some(version) = document.get("version")
        && version.as_i64() != Some(1)
    {
        return Err(CoreError::Validation(
            "dsh 凭据存储声明版本不是 1，请先修复后重新应用".into(),
        ));
    }
    document.insert("version".into(), json!(1));
    let refs_value = document.entry("refs").or_insert_with(|| json!({}));
    let refs = refs_value.as_object_mut().ok_or_else(|| {
        CoreError::Validation("dsh 凭据存储 refs 必须是映射，请先修复后重新应用".into())
    })?;
    refs.retain(|key, _| {
        if !key.starts_with(DIRECT_KEY_REFERENCE_PREFIX) {
            return true;
        }
        current_direct.is_some_and(|reference| reference == key)
    });
    if value.is_empty() {
        refs.remove(reference);
        return Ok(());
    }
    let existing = refs
        .get(reference)
        .and_then(Value::as_str)
        .is_some_and(|stored| !stored.is_empty());
    if !existing || current_direct.is_some() {
        refs.insert(reference.to_owned(), json!(value));
    }
    Ok(())
}

fn owned_secrets(provider: &crate::domain::Provider) -> Vec<String> {
    if provider.api_key.trim().is_empty() {
        Vec::new()
    } else {
        vec![provider.api_key.clone()]
    }
}

fn proxy_models(resolved: &[super::ResolvedModel<'_>], protocol: Protocol) -> Vec<Value> {
    resolved
        .iter()
        .map(|resolved| {
            model_entry(
                &resolved.selection.model,
                resolved.capabilities,
                protocol,
                resolved.upstream(),
            )
        })
        .collect()
}

impl ClientSpec for DeepSeekHarness {
    fn client(&self) -> Client {
        Client::DeepseekHarness
    }

    fn files(&self, ctx: &ClientCtx) -> Vec<(PathBuf, DocFormat)> {
        let home = home(ctx);
        let mut files: Vec<(PathBuf, DocFormat)> = patch_paths(&home)
            .into_iter()
            .map(|path| (path, DocFormat::Yaml))
            .collect();
        files.push((home.join(CREDENTIALS_FILE), DocFormat::Yaml));
        files
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
        let endpoint = super::super::spec::proxy_endpoint(
            Client::DeepseekHarness,
            profile.protocol,
            &ctx.proxy_base,
        );
        let api = protocol_api(profile.protocol);
        let (caps, upstream) = proxy_model_caps(ctx, profile.protocol);
        let mut models = vec![model_entry(AUTO_MODEL, &caps, profile.protocol, upstream)];
        if !auto {
            models.extend(proxy_models(&resolved, profile.protocol));
        }
        let patch_count = docs.len().saturating_sub(1);
        for doc in &mut docs[..patch_count] {
            let doc = patch_array(doc)?;
            {
                let providers = providers(doc)?;
                if let Some(existing) = providers.get(PROXY_PROVIDER_ID)
                    && existing.get("name").and_then(Value::as_str).is_some()
                    && existing.get("name").and_then(Value::as_str) != Some("yi-token")
                {
                    return Err(CoreError::Conflict(
                        "DeepSeek Harness Provider 标识 yitoken 已被其他配置占用".into(),
                    ));
                }
                prune_direct(providers, None);
                providers.insert(
                    PROXY_PROVIDER_ID.into(),
                    json!({
                        "name": "yi-token",
                        "apiKeyEnv": PROXY_KEY_REFERENCE,
                        "api": api,
                        "baseURL": endpoint,
                        "models": models
                    }),
                );
            }
            update_default_agent(doc, PROXY_PROVIDER_ID, &default);
        }
        provision_credentials(
            &mut docs[patch_count..],
            PROXY_KEY_REFERENCE,
            "yi-token",
            None,
        )?;
        Ok(Vec::new())
    }

    fn render_direct(
        &self,
        docs: &mut [Doc],
        profile: &DirectProfile,
        ctx: &ClientCtx,
    ) -> CoreResult<Vec<String>> {
        let provider = direct_provider(profile, ctx)?;
        let id = direct_provider_id(&provider.id).to_lowercase();
        let name = direct_name(&provider.id);
        let key_reference = direct_key_reference(&provider.id);
        let selected = direct_startup_model(profile, provider, ctx)?;
        let api = protocol_api(provider.provider_type);
        let models: Vec<Value> = provider_models(&provider.id, ctx)
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
        let mut models = models;
        if !models
            .iter()
            .any(|entry| entry["id"].as_str() == Some(selected.as_str()))
        {
            models.push(json!({"id": selected, "name": selected}));
        }
        let patch_count = docs.len().saturating_sub(1);
        for doc in &mut docs[..patch_count] {
            let doc = patch_array(doc)?;
            {
                let providers = providers(doc)?;
                if let Some(existing) = providers.get(&id)
                    && existing.get("name").and_then(Value::as_str) != Some(name.as_str())
                {
                    return Err(CoreError::Conflict(format!(
                        "DeepSeek Harness Provider 标识 {id} 已被其他配置占用"
                    )));
                }
                prune_direct(providers, Some(&id));
                let mut entry = Map::new();
                entry.insert("api".into(), json!(api));
                entry.insert("name".into(), json!(name));
                entry.insert(
                    "baseURL".into(),
                    json!(provider.base_url.trim_end_matches('/')),
                );
                if !provider.api_key.trim().is_empty() {
                    entry.insert("apiKeyEnv".into(), json!(key_reference));
                }
                entry.insert("models".into(), Value::Array(models.clone()));
                providers.insert(id.clone(), Value::Object(entry));
            }
            update_default_agent(doc, &id, &selected);
        }
        provision_credentials(
            &mut docs[patch_count..],
            &key_reference,
            provider.api_key.trim(),
            Some(&key_reference),
        )?;
        Ok(owned_secrets(provider))
    }

    fn render_clear(&self, docs: &mut [Doc], _ctx: &ClientCtx) -> CoreResult<Vec<bool>> {
        let last = docs.len().saturating_sub(1);
        let multiple = docs.len() > 1;
        docs.iter_mut()
            .enumerate()
            .map(|(index, doc)| {
                if multiple && index == last {
                    clear_credentials(doc)
                } else {
                    clear_patch(doc)
                }
            })
            .collect()
    }
}

fn plugin_entry(entry: &Value) -> bool {
    entry.get("id").and_then(Value::as_str) == Some(PLUGIN_ID)
        || entry.get("name").and_then(Value::as_str) == Some(PLUGIN_NAME)
        || entry.get("name").and_then(Value::as_str) == Some(PLUGIN_ID)
}

fn clear_patch(doc: &mut Doc) -> CoreResult<bool> {
    let Doc::Yaml(value) = doc else {
        return Ok(false);
    };
    let Some(entries) = value.as_array_mut() else {
        return Ok(false);
    };
    let mut changed = false;
    if let Some(entry) = entries.iter_mut().find(|entry| plugin_entry(entry))
        && let Some(providers) = entry
            .pointer_mut("/config/providers")
            .and_then(Value::as_object_mut)
    {
        if providers.remove(PROXY_PROVIDER_ID).is_some() {
            changed = true;
        }
        let before = providers.len();
        prune_direct(providers, None);
        if providers.len() != before {
            changed = true;
        }
    }
    for entry in entries.iter_mut() {
        let agent_loop = entry.get("id").and_then(Value::as_str) == Some(AGENT_LOOP_ID)
            || entry
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| name.ends_with(AGENT_LOOP_ID));
        if !agent_loop {
            continue;
        }
        let Some(agents) = entry
            .pointer_mut("/config/agents")
            .and_then(Value::as_array_mut)
        else {
            continue;
        };
        for agent in agents {
            let provider = agent.get("provider").and_then(Value::as_str).unwrap_or("");
            if (provider == PROXY_PROVIDER_ID || super::is_owned_direct_id(provider))
                && let Some(map) = agent.as_object_mut()
            {
                map.remove("provider");
                map.remove("model");
                changed = true;
            }
        }
    }
    Ok(changed)
}

fn clear_credentials(doc: &mut Doc) -> CoreResult<bool> {
    let Doc::Yaml(value) = doc else {
        return Ok(false);
    };
    if value.is_null() {
        return Ok(false);
    }
    let Some(refs) = value.get_mut("refs").and_then(Value::as_object_mut) else {
        return Ok(false);
    };
    let before = refs.len();
    refs.retain(|key, _| {
        key != PROXY_KEY_REFERENCE && !key.starts_with(DIRECT_KEY_REFERENCE_PREFIX)
    });
    Ok(refs.len() != before)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

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
    fn missing_profiles_default_to_desktop() {
        let dir = tempfile::tempdir().unwrap();
        let files = DeepSeekHarness.files(&ctx(dir.path()));
        assert_eq!(
            files[0].0,
            dir.path().join(".dsh/profiles/desktop/cordis.patch.yml")
        );
    }

    #[test]
    fn existing_profiles_are_both_managed() {
        let dir = tempfile::tempdir().unwrap();
        let dsh = dir.path().join(".dsh");
        std::fs::create_dir_all(dsh.join("profiles/desktop")).unwrap();
        std::fs::create_dir_all(dsh.join("profiles/web")).unwrap();
        let files = DeepSeekHarness.files(&ctx(dir.path()));
        assert_eq!(files.len(), 3);
        assert!(files[0].0.ends_with("desktop/cordis.patch.yml"));
        assert!(files[1].0.ends_with("web/cordis.patch.yml"));
    }

    #[test]
    fn direct_credentials_update_existing_key() {
        let mut docs = vec![
            Doc::parse(
                DocFormat::Yaml,
                "version: 1\nrefs:\n  YI_TOKEN_DIRECT_P1: old-key\n",
            )
            .unwrap(),
        ];
        provision_credentials(
            &mut docs,
            "YI_TOKEN_DIRECT_P1",
            "new-key",
            Some("YI_TOKEN_DIRECT_P1"),
        )
        .unwrap();
        assert_eq!(
            docs[0].get(&["refs", "YI_TOKEN_DIRECT_P1"]),
            Some(json!("new-key"))
        );
    }
}
