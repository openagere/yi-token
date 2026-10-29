use super::payload::{Payload, ProviderBackup};
use crate::domain::{Company, Model, Provider, exposed_name, provider_slug};
use crate::error::{CoreError, CoreResult};
use crate::storage::{engines, enum_str, map_sqlite, models, providers, standard_models, terminal};
use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportReport {
    pub providers: usize,
    pub standard_models: usize,
    pub models: usize,
    pub terminal_profiles: usize,
    pub skipped: Vec<String>,
}

pub fn merge(tx: &Transaction, mut payload: Payload) -> CoreResult<ImportReport> {
    payload.validate()?;
    let standard_ids = remap_standard_models(tx, &payload)?;
    for model in &mut payload.providers {
        for model in &mut model.models {
            if let Some(id) = model
                .standard_model_id
                .as_ref()
                .and_then(|id| standard_ids.get(id))
            {
                model.standard_model_id = Some(id.clone());
            }
        }
    }
    let providers = upsert_providers(tx, &payload.providers)?;
    let models = upsert_models(tx, &payload.providers)?;
    let mut report = ImportReport {
        providers,
        standard_models: payload.standard_models.len(),
        models,
        terminal_profiles: 0,
        skipped: Vec::new(),
    };
    import_proxy_profiles(tx, &payload, &mut report)?;
    import_direct_profiles(tx, &payload, &mut report)?;
    import_engines(tx, &payload, &mut report)?;
    tx.execute(
        "UPDATE settings SET log_level = ?1 WHERE id = 1",
        [enum_str(&payload.settings.log_level)?],
    )?;
    Ok(report)
}

fn remap_standard_models(
    tx: &Transaction,
    payload: &Payload,
) -> CoreResult<HashMap<String, String>> {
    let mut remapped = HashMap::new();
    for standard in &payload.standard_models {
        let existing: Option<String> = tx
            .query_row(
                "SELECT id FROM standard_models WHERE name = ?1 AND company = ?2",
                params![standard.name.trim(), enum_str(&standard.company)?],
                |row| row.get(0),
            )
            .optional()?;
        let mut standard = standard.clone();
        if let Some(id) = existing {
            remapped.insert(standard.id.clone(), id.clone());
            standard.id = id;
        }
        standard_models::upsert(tx, &standard)?;
    }
    Ok(remapped)
}

fn upsert_providers(tx: &Transaction, entries: &[ProviderBackup]) -> CoreResult<usize> {
    let mut next_sort = providers::list(tx)?
        .into_iter()
        .map(|provider| provider.sort_order)
        .max()
        .map_or(0, |order| order + 1);
    let mut default_id = None;
    for entry in entries {
        let provider = &entry.provider;
        let sort_order = provider_sort_order(tx, provider)?;
        let sort_order = match sort_order {
            Some(order) => order,
            None => {
                let order = next_sort;
                next_sort += 1;
                order
            }
        };
        check_name(tx, provider)?;
        let thinking = provider.thinking.as_ref().map(enum_str).transpose()?;
        tx.execute(
            "INSERT INTO providers (id, type, name, short_code, base_url, api_key, enabled, \
               thinking, is_default, sort_order, extra) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
             ON CONFLICT(id) DO UPDATE SET type = excluded.type, name = excluded.name, \
               base_url = excluded.base_url, \
               api_key = excluded.api_key, enabled = excluded.enabled, \
               thinking = excluded.thinking, is_default = excluded.is_default, \
               sort_order = excluded.sort_order, extra = excluded.extra",
            params![
                provider.id,
                enum_str(&provider.provider_type)?,
                provider.name.trim(),
                // Legacy unique column, see storage::providers::upsert.
                provider.id,
                provider.base_url.trim(),
                provider.api_key,
                provider.enabled,
                thinking,
                provider.is_default,
                sort_order,
                provider.extra.to_string(),
            ],
        )
        .map_err(map_sqlite)?;
        tx.execute(
            "DELETE FROM provider_protocols WHERE provider_id = ?1",
            [&provider.id],
        )?;
        for protocol in &provider.protocols {
            tx.execute(
                "INSERT OR IGNORE INTO provider_protocols (provider_id, protocol) \
                 VALUES (?1, ?2)",
                params![provider.id, enum_str(protocol)?],
            )?;
        }
        if provider.is_default {
            default_id = Some(provider.id.as_str());
        }
    }
    if let Some(id) = default_id {
        tx.execute("UPDATE providers SET is_default = 0 WHERE id != ?1", [id])?;
    }
    Ok(entries.len())
}

fn provider_sort_order(tx: &Transaction, provider: &Provider) -> CoreResult<Option<i64>> {
    Ok(tx
        .query_row(
            "SELECT sort_order FROM providers WHERE id = ?1",
            [&provider.id],
            |row| row.get(0),
        )
        .optional()?)
}

fn check_name(tx: &Transaction, provider: &Provider) -> CoreResult<()> {
    let slug = provider_slug(&provider.name).to_lowercase();
    let mut stmt = tx.prepare("SELECT name FROM providers WHERE id != ?1")?;
    let names = stmt.query_map([&provider.id], |row| row.get::<_, String>(0))?;
    for name in names {
        let name = name?;
        if provider_slug(&name).to_lowercase() == slug {
            return Err(CoreError::Conflict(format!("Provider 名称 {name} 已被其他服务商使用")));
        }
    }
    Ok(())
}

fn upsert_models(tx: &Transaction, entries: &[ProviderBackup]) -> CoreResult<usize> {
    let mut count = 0;
    for entry in entries {
        let provider = &entry.provider;
        for model in &entry.models {
            let model = prepare_model(tx, provider, model)?;
            check_exposed_name(tx, &model)?;
            tx.execute(
                "INSERT INTO models (id, provider_id, standard_model_id, upstream_model, \
                   exposed_name, capabilities, non_standard, protocol) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
                 ON CONFLICT(id) DO UPDATE SET provider_id = excluded.provider_id, \
                   standard_model_id = excluded.standard_model_id, \
                   upstream_model = excluded.upstream_model, \
                   exposed_name = excluded.exposed_name, \
                   capabilities = excluded.capabilities, \
                   non_standard = excluded.non_standard, \
                   protocol = excluded.protocol",
                params![
                    model.id,
                    model.provider_id,
                    model.standard_model_id,
                    model.upstream_model,
                    model.exposed_name,
                    serde_json::to_string(&model.capabilities)?,
                    model.non_standard,
                    model.protocol.as_ref().map(enum_str).transpose()?,
                ],
            )
            .map_err(map_sqlite)?;
            count += 1;
        }
    }
    Ok(count)
}

fn prepare_model(tx: &Transaction, provider: &Provider, model: &Model) -> CoreResult<Model> {
    let mut model = model.clone();
    model.provider_id = provider.id.clone();
    model.upstream_model = model.upstream_model.trim().to_owned();
    if model.upstream_model.is_empty() {
        return Err(CoreError::Validation("模型名称不能为空".into()));
    }
    model.exposed_name = exposed_name(&provider.name, &model.upstream_model);
    match &model.standard_model_id {
        Some(id) => {
            let standard = standard_models::get(tx, id)?.ok_or_else(|| {
                CoreError::Validation(format!(
                    "服务商「{}」的模型「{}」引用的标准模型不存在：{id}",
                    provider.name.trim(),
                    model.upstream_model
                ))
            })?;
            if standard.company != Company::of(model.upstream_protocol(provider)) {
                return Err(CoreError::Validation(format!(
                    "服务商「{}」的模型「{}」与标准模型「{}」的协议不一致",
                    provider.name.trim(),
                    model.upstream_model,
                    standard.name.trim()
                )));
            }
        }
        None => model
            .capabilities
            .validate(model.upstream_protocol(provider))
            .map_err(|error| {
                CoreError::Validation(format!(
                    "服务商「{}」的模型「{}」：{error}",
                    provider.name.trim(),
                    model.upstream_model
                ))
            })?,
    }
    Ok(model)
}

fn check_exposed_name(tx: &Transaction, model: &Model) -> CoreResult<()> {
    let holder: Option<String> = tx
        .query_row(
            "SELECT p.name FROM models m JOIN providers p ON p.id = m.provider_id \
             WHERE m.exposed_name = ?1 AND m.id != ?2",
            params![model.exposed_name, model.id],
            |row| row.get(0),
        )
        .optional()?;
    match holder {
        Some(name) => Err(CoreError::Conflict(format!(
            "模型名称 {} 已被服务商 {} 使用",
            model.exposed_name, name
        ))),
        None => Ok(()),
    }
}

fn import_proxy_profiles(
    tx: &Transaction,
    payload: &Payload,
    report: &mut ImportReport,
) -> CoreResult<()> {
    let mut refs: HashMap<String, HashSet<String>> = HashMap::new();
    for model in models::list(tx)? {
        let entry = refs.entry(model.provider_id).or_default();
        entry.insert(model.id);
        entry.insert(model.exposed_name);
    }
    for profile in &payload.terminal_proxy_profiles {
        let valid = profile.models.iter().all(|selection| {
            refs.get(&selection.provider_id)
                .is_some_and(|models| models.contains(&selection.model))
        });
        if !valid {
            report.skipped.push(format!(
                "{} 的代理配置引用了不存在的服务商或模型，已跳过",
                profile.client.as_str()
            ));
            continue;
        }
        terminal::save_proxy(tx, profile)?;
        report.terminal_profiles += 1;
    }
    Ok(())
}

fn import_direct_profiles(
    tx: &Transaction,
    payload: &Payload,
    report: &mut ImportReport,
) -> CoreResult<()> {
    let providers: HashSet<String> = providers::list(tx)?
        .into_iter()
        .map(|provider| provider.id)
        .collect();
    for profile in &payload.terminal_direct_profiles {
        if !providers.contains(&profile.provider_id) {
            report.skipped.push(format!(
                "{} 的直连配置引用了不存在的服务商，已跳过",
                profile.client.as_str()
            ));
            continue;
        }
        terminal::save_direct(tx, profile)?;
        report.terminal_profiles += 1;
    }
    Ok(())
}

fn import_engines(
    tx: &Transaction,
    payload: &Payload,
    report: &mut ImportReport,
) -> CoreResult<()> {
    let model_ids: HashSet<String> = models::list(tx)?
        .into_iter()
        .map(|model| model.id)
        .collect();
    for engine in &payload.terminal_engines {
        if let Some(id) = &engine.model_id
            && !model_ids.contains(id)
        {
            report.skipped.push(format!(
                "{} 的当前引擎引用了不存在的模型，已跳过",
                engine.client.as_str()
            ));
            continue;
        }
        engines::set(tx, engine.client, engine.model_id.as_deref())?;
    }
    Ok(())
}
