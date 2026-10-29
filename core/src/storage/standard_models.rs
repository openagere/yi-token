//! Standard (catalog) models shared by provider models.

use super::{enum_str, map_sqlite, parse_enum, parse_json, query_as};
use crate::domain::{Company, Protocol, StandardModel};
use crate::error::{CoreError, CoreResult};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;

const SELECT: &str = "SELECT id, name, company, brand, capabilities FROM standard_models";

#[derive(Deserialize)]
struct StandardRow {
    id: String,
    name: String,
    company: String,
    brand: String,
    capabilities: String,
}

impl StandardRow {
    fn into_model(self) -> CoreResult<StandardModel> {
        Ok(StandardModel {
            company: parse_enum(&self.company)?,
            capabilities: parse_json(&self.capabilities, "标准模型能力")?,
            id: self.id,
            name: self.name,
            brand: self.brand,
        })
    }
}

fn fetch(
    conn: &Connection,
    tail: &str,
    params: impl rusqlite::Params,
) -> CoreResult<Vec<StandardModel>> {
    query_as::<StandardRow>(conn, &format!("{SELECT} {tail}"), params)?
        .into_iter()
        .map(StandardRow::into_model)
        .collect()
}

pub fn list(conn: &Connection) -> CoreResult<Vec<StandardModel>> {
    fetch(conn, "ORDER BY name COLLATE NOCASE, company, id", [])
}

pub fn get(conn: &Connection, id: &str) -> CoreResult<Option<StandardModel>> {
    Ok(fetch(conn, "WHERE id = ?1", [id])?.pop())
}

/// Effective upstream protocols of the provider models bound to `id`, with the provider name.
fn bound_protocols(conn: &Connection, id: &str) -> CoreResult<Vec<(Protocol, String)>> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(m.protocol, p.type), p.name FROM models m \
         JOIN providers p ON p.id = m.provider_id WHERE m.standard_model_id = ?1",
    )?;
    let rows = stmt
        .query_map([id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(protocol, provider)| Ok((parse_enum(&protocol)?, provider)))
        .collect()
}

fn validate(model: &StandardModel) -> CoreResult<()> {
    if model.name.trim().is_empty() {
        return Err(CoreError::Validation("标准模型名称不能为空".into()));
    }
    model
        .capabilities
        .validate(model.company.protocol())
        .map_err(|e| CoreError::Validation(format!("标准模型 {}：{e}", model.name)))
}

/// Inserts or updates a standard model.
pub fn upsert(conn: &Connection, model: &StandardModel) -> CoreResult<()> {
    validate(model)?;
    let name = model.name.trim();
    let company = enum_str(&model.company)?;
    let mismatched = bound_protocols(conn, &model.id)?
        .into_iter()
        .any(|(protocol, _)| Company::of(protocol) != model.company);
    if mismatched {
        return Err(CoreError::Conflict(
            "该标准模型已被其他公司协议的模型引用，不能修改公司".into(),
        ));
    }
    let duplicate: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM standard_models WHERE name = ?1 AND company = ?2 AND id != ?3)",
        params![name, company, model.id],
        |r| r.get(0),
    )?;
    if duplicate {
        return Err(CoreError::Conflict(
            "同一公司下的标准模型名称不能重复".into(),
        ));
    }
    write(conn, model, name, &company)
}

fn write(conn: &Connection, model: &StandardModel, name: &str, company: &str) -> CoreResult<()> {
    conn.execute(
        "INSERT INTO standard_models (id, name, company, brand, capabilities) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, company = excluded.company, \
           brand = excluded.brand, capabilities = excluded.capabilities",
        params![
            model.id,
            name,
            company,
            model.brand,
            serde_json::to_string(&model.capabilities)?
        ],
    )
    .map_err(map_sqlite)?;
    Ok(())
}

/// Deletes a standard model; refuses while any provider model references it.
pub fn delete(conn: &Connection, id: &str) -> CoreResult<()> {
    let in_use: i64 = conn.query_row(
        "SELECT COUNT(*) FROM models WHERE standard_model_id = ?1",
        [id],
        |r| r.get(0),
    )?;
    if in_use > 0 {
        let name: Option<String> = conn
            .query_row(
                "SELECT name FROM standard_models WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        return Err(CoreError::Conflict(format!(
            "标准模型 {} 仍被 {in_use} 个服务商模型引用，请先解除关联",
            name.as_deref().unwrap_or(id)
        )));
    }
    conn.execute("DELETE FROM standard_models WHERE id = ?1", [id])
        .map_err(map_sqlite)?;
    Ok(())
}

/// Makes the table equal to `entries`: upserts every entry and removes the rest.
///
/// Fails with `Conflict` (nothing is changed: the transaction rolls back) if an
/// entry that is still referenced would disappear or change company.
pub fn replace_catalog_entries(
    conn: &rusqlite::Transaction,
    entries: &[StandardModel],
) -> CoreResult<()> {
    for entry in entries {
        validate(entry)?;
    }
    let mut refs = conn.prepare(
        "SELECT m.standard_model_id, COALESCE(m.protocol, p.type), p.name FROM models m \
         JOIN providers p ON p.id = m.provider_id WHERE m.standard_model_id IS NOT NULL",
    )?;
    let refs = refs
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, protocol, provider) in refs {
        let entry = entries.iter().find(|e| e.id == id).ok_or_else(|| {
            CoreError::Conflict(format!(
                "标准模型 {id} 仍被服务商「{provider}」引用，不能从目录删除"
            ))
        })?;
        let protocol: Protocol = parse_enum(&protocol)?;
        if Company::of(protocol) != entry.company {
            return Err(CoreError::Conflict(format!(
                "标准模型「{}」与服务商「{provider}」的 {} 协议不属于该公司",
                entry.name,
                protocol.as_str()
            )));
        }
    }
    // Park every name on a unique placeholder so a revision may swap names.
    conn.execute("UPDATE standard_models SET name = CHAR(0) || id", [])?;
    for entry in entries {
        write(conn, entry, entry.name.trim(), &enum_str(&entry.company)?)?;
    }
    let existing: Vec<String> = conn
        .prepare("SELECT id FROM standard_models")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in existing {
        if !entries.iter().any(|e| e.id == id) {
            conn.execute("DELETE FROM standard_models WHERE id = ?1", [id])
                .map_err(map_sqlite)?;
        }
    }
    Ok(())
}
