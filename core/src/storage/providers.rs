//! Providers, their supported protocols and their model sets.

use super::{enum_str, map_sqlite, models, parse_enum, parse_json, query_as, standard_models};
use crate::domain::{Company, Model, Protocol, Provider, exposed_name, provider_slug};
use crate::error::{CoreError, CoreResult};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Deserialize;
use std::collections::HashMap;

const SELECT: &str = "SELECT id, type, name, base_url, api_key, enabled, thinking, \
     is_default, sort_order, extra FROM providers";

#[derive(Deserialize)]
struct ProviderRow {
    id: String,
    r#type: String,
    name: String,
    base_url: String,
    api_key: String,
    enabled: bool,
    thinking: Option<String>,
    is_default: bool,
    sort_order: i64,
    extra: String,
}

#[derive(Deserialize)]
struct ProtocolRow {
    provider_id: String,
    protocol: String,
}

impl ProviderRow {
    fn into_provider(self, protocols: Vec<Protocol>) -> CoreResult<Provider> {
        Ok(Provider {
            provider_type: parse_enum(&self.r#type)?,
            thinking: self.thinking.as_deref().map(parse_enum).transpose()?,
            extra: parse_json(&self.extra, "服务商扩展配置")?,
            id: self.id,
            name: self.name,
            base_url: self.base_url,
            api_key: self.api_key,
            enabled: self.enabled,
            is_default: self.is_default,
            sort_order: self.sort_order,
            protocols,
        })
    }
}

fn fetch(
    conn: &Connection,
    tail: &str,
    params: impl rusqlite::Params,
) -> CoreResult<Vec<Provider>> {
    let rows = query_as::<ProviderRow>(conn, &format!("{SELECT} {tail}"), params)?;
    let mut protocols: HashMap<String, Vec<Protocol>> = HashMap::new();
    for row in query_as::<ProtocolRow>(
        conn,
        "SELECT provider_id, protocol FROM provider_protocols ORDER BY rowid",
        [],
    )? {
        protocols
            .entry(row.provider_id)
            .or_default()
            .push(parse_enum(&row.protocol)?);
    }
    rows.into_iter()
        .map(|row| {
            let own = protocols.remove(&row.id).unwrap_or_default();
            row.into_provider(own)
        })
        .collect()
}

pub fn list(conn: &Connection) -> CoreResult<Vec<Provider>> {
    fetch(conn, "ORDER BY sort_order, rowid", [])
}

pub fn get(conn: &Connection, id: &str) -> CoreResult<Option<Provider>> {
    Ok(fetch(conn, "WHERE id = ?1", [id])?.pop())
}

/// Inserts or updates a provider and rewrites its protocols and model set.
///
/// Models are matched to existing ones by `upstream_model`, so their ids (and
/// everything referencing them, such as engine selections) survive; models
/// that are no longer listed are deleted. `exposed_name` is always recomputed
/// as `{provider}/{upstream_model}`.
pub fn upsert(tx: &Transaction, provider: &Provider, models_in: &[Model]) -> CoreResult<()> {
    provider.validate()?;
    check_name(tx, provider)?;
    let planned = plan_models(tx, provider, models_in)?;

    let protocol = enum_str(&provider.provider_type)?;
    let thinking = provider.thinking.as_ref().map(enum_str).transpose()?;
    tx.execute(
        "INSERT INTO providers (id, type, name, short_code, base_url, api_key, enabled, \
           thinking, is_default, sort_order, extra) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
         ON CONFLICT(id) DO UPDATE SET type = excluded.type, name = excluded.name, \
           base_url = excluded.base_url, \
           api_key = excluded.api_key, enabled = excluded.enabled, thinking = excluded.thinking, \
           is_default = excluded.is_default, sort_order = excluded.sort_order, \
           extra = excluded.extra",
        params![
            provider.id,
            protocol,
            provider.name,
            // Legacy NOT NULL UNIQUE column; the id keeps it unique and nothing reads it.
            provider.id,
            provider.base_url,
            provider.api_key,
            provider.enabled,
            thinking,
            provider.is_default,
            provider.sort_order,
            provider.extra.to_string(),
        ],
    )
    .map_err(map_sqlite)?;
    if provider.is_default {
        tx.execute(
            "UPDATE providers SET is_default = 0 WHERE id != ?1",
            [&provider.id],
        )?;
    }

    tx.execute(
        "DELETE FROM provider_protocols WHERE provider_id = ?1",
        [&provider.id],
    )?;
    for p in &provider.protocols {
        tx.execute(
            "INSERT OR IGNORE INTO provider_protocols (provider_id, protocol) VALUES (?1, ?2)",
            params![provider.id, enum_str(p)?],
        )?;
    }

    write_models(tx, provider, planned)
}

/// Provider names prefix every exposed model name, so two providers may not fold to the same slug.
fn check_name(tx: &Transaction, provider: &Provider) -> CoreResult<()> {
    let slug = provider_slug(&provider.name).to_lowercase();
    let mut stmt = tx.prepare("SELECT name FROM providers WHERE id != ?1")?;
    let names = stmt.query_map([&provider.id], |r| r.get::<_, String>(0))?;
    for name in names {
        let name = name?;
        if provider_slug(&name).to_lowercase() == slug {
            return Err(CoreError::Conflict(format!("Provider 名称 {name} 已存在")));
        }
    }
    Ok(())
}

/// A model prepared for writing; `existing` is true when its id is kept.
struct Planned {
    model: Model,
    exposed: String,
    existing: bool,
}

fn plan_models(tx: &Transaction, provider: &Provider, input: &[Model]) -> CoreResult<Vec<Planned>> {
    let kept: HashMap<String, String> = models::list_by_provider(tx, &provider.id)?
        .into_iter()
        .map(|m| (m.upstream_model, m.id))
        .collect();
    let mut seen: Vec<String> = Vec::new();
    let mut planned = Vec::with_capacity(input.len());
    for m in input {
        let upstream = m.upstream_model.trim().to_owned();
        if upstream.is_empty() {
            return Err(CoreError::Validation("模型名称不能为空".into()));
        }
        let exposed = exposed_name(&provider.name, &upstream);
        if seen.contains(&upstream) {
            return Err(CoreError::Conflict(format!("模型名称 {exposed} 已存在")));
        }
        seen.push(upstream.clone());
        let other: Option<String> = tx
            .query_row(
                "SELECT p.name FROM models m JOIN providers p ON p.id = m.provider_id \
                 WHERE m.exposed_name = ?1 AND m.provider_id != ?2",
                params![exposed, provider.id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(name) = other {
            return Err(CoreError::Conflict(format!(
                "模型名称 {exposed} 已被服务商 {name} 使用"
            )));
        }
        if let Some(protocol) = m.protocol
            && !provider.protocols.contains(&protocol)
        {
            return Err(CoreError::Validation(format!(
                "模型 {exposed} 的协议不是服务商支持的协议"
            )));
        }
        let upstream_protocol = m.protocol.unwrap_or(provider.provider_type);
        match &m.standard_model_id {
            Some(id) => {
                let standard = standard_models::get(tx, id)?
                    .ok_or_else(|| CoreError::Validation("关联的标准模型不存在".into()))?;
                if standard.company != Company::of(upstream_protocol) {
                    return Err(CoreError::Validation(
                        "标准模型与模型的上游协议不匹配".into(),
                    ));
                }
            }
            None => m
                .capabilities
                .validate(upstream_protocol)
                .map_err(|e| CoreError::Validation(format!("模型 {exposed}：{e}")))?,
        }
        let (id, existing) = match kept.get(&upstream) {
            Some(id) => (id.clone(), true),
            None if m.id.is_empty() => (uuid::Uuid::new_v4().to_string(), false),
            None => (m.id.clone(), false),
        };
        planned.push(Planned {
            model: Model {
                id,
                provider_id: provider.id.clone(),
                upstream_model: upstream,
                ..m.clone()
            },
            exposed,
            existing,
        });
    }
    Ok(planned)
}

fn write_models(tx: &Transaction, provider: &Provider, planned: Vec<Planned>) -> CoreResult<()> {
    // Drop models that disappeared first, so their names and ids can be reused.
    let keep: Vec<&str> = planned
        .iter()
        .filter(|p| p.existing)
        .map(|p| p.model.id.as_str())
        .collect();
    for old in models::list_by_provider(tx, &provider.id)? {
        if !keep.contains(&old.id.as_str()) {
            tx.execute("DELETE FROM models WHERE id = ?1", [&old.id])?;
        }
    }
    for p in &planned {
        let m = &p.model;
        let caps = serde_json::to_string(&m.capabilities)?;
        if p.existing {
            tx.execute(
                "UPDATE models SET standard_model_id = ?2, exposed_name = ?3, capabilities = ?4, \
                   non_standard = ?5, protocol = ?6 WHERE id = ?1",
                params![
                    m.id,
                    m.standard_model_id,
                    p.exposed,
                    caps,
                    m.non_standard,
                    protocol_str(m.protocol)?
                ],
            )
            .map_err(map_sqlite)?;
        } else {
            tx.execute(
                "INSERT INTO models (id, provider_id, standard_model_id, upstream_model, \
                   exposed_name, capabilities, non_standard, protocol) \
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    m.id,
                    m.provider_id,
                    m.standard_model_id,
                    m.upstream_model,
                    p.exposed,
                    caps,
                    m.non_standard,
                    protocol_str(m.protocol)?
                ],
            )
            .map_err(map_sqlite)?;
        }
    }
    Ok(())
}

fn protocol_str(protocol: Option<Protocol>) -> CoreResult<Option<String>> {
    protocol.as_ref().map(enum_str).transpose()
}

/// Deletes a provider; models, protocols and its direct profile cascade.
pub fn delete(conn: &Connection, id: &str) -> CoreResult<()> {
    conn.execute("DELETE FROM providers WHERE id = ?1", [id])?;
    Ok(())
}

/// Makes `id` the only default provider.
pub fn set_default(conn: &Connection, id: &str) -> CoreResult<()> {
    let found = conn.execute("UPDATE providers SET is_default = 1 WHERE id = ?1", [id])?;
    if found == 0 {
        return Err(CoreError::NotFound("服务商不存在".into()));
    }
    conn.execute("UPDATE providers SET is_default = 0 WHERE id != ?1", [id])?;
    Ok(())
}
