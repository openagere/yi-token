//! Read access to provider models.
//!
//! Writes go through [`super::providers::upsert`], which owns the model set of
//! a provider.

use super::{parse_json, query_as};
use crate::domain::Model;
use crate::error::CoreResult;
use rusqlite::Connection;
use serde::Deserialize;

/// A model bound to a standard model reads that standard model's capabilities.
const SELECT: &str = "SELECT m.id, m.provider_id, m.standard_model_id, m.upstream_model, \
     m.exposed_name, COALESCE(s.capabilities, m.capabilities) AS capabilities, m.non_standard \
     FROM models m LEFT JOIN standard_models s ON s.id = m.standard_model_id";

#[derive(Deserialize)]
struct ModelRow {
    id: String,
    provider_id: String,
    standard_model_id: Option<String>,
    upstream_model: String,
    exposed_name: String,
    capabilities: String,
    non_standard: bool,
}

impl ModelRow {
    fn into_model(self) -> CoreResult<Model> {
        Ok(Model {
            capabilities: parse_json(&self.capabilities, "模型能力")?,
            id: self.id,
            provider_id: self.provider_id,
            standard_model_id: self.standard_model_id,
            upstream_model: self.upstream_model,
            exposed_name: self.exposed_name,
            non_standard: self.non_standard,
        })
    }
}

fn fetch(conn: &Connection, tail: &str, params: impl rusqlite::Params) -> CoreResult<Vec<Model>> {
    query_as::<ModelRow>(conn, &format!("{SELECT} {tail}"), params)?
        .into_iter()
        .map(ModelRow::into_model)
        .collect()
}

pub fn list(conn: &Connection) -> CoreResult<Vec<Model>> {
    fetch(conn, "ORDER BY m.rowid", [])
}

pub fn list_by_provider(conn: &Connection, provider_id: &str) -> CoreResult<Vec<Model>> {
    fetch(
        conn,
        "WHERE m.provider_id = ?1 ORDER BY m.rowid",
        [provider_id],
    )
}

pub fn get(conn: &Connection, id: &str) -> CoreResult<Option<Model>> {
    Ok(fetch(conn, "WHERE m.id = ?1", [id])?.pop())
}

/// Inserts `upstream` on `provider` when that provider does not already have it.
pub fn ensure(
    conn: &Connection,
    provider: &crate::domain::Provider,
    upstream: &str,
) -> CoreResult<Model> {
    let upstream = upstream.trim();
    if upstream.is_empty() {
        return Err(crate::error::CoreError::Validation(
            "模型名称不能为空".into(),
        ));
    }
    if let Some(existing) = list_by_provider(conn, &provider.id)?
        .into_iter()
        .find(|model| model.upstream_model == upstream)
    {
        return Ok(existing);
    }
    let exposed = crate::domain::exposed_name(&provider.name, upstream);
    let model = Model {
        id: uuid::Uuid::new_v4().to_string(),
        provider_id: provider.id.clone(),
        standard_model_id: None,
        upstream_model: upstream.to_owned(),
        exposed_name: exposed,
        capabilities: crate::domain::Capabilities::default(),
        non_standard: false,
    };
    conn.execute(
        "INSERT INTO models (id, provider_id, standard_model_id, upstream_model, exposed_name, \
           capabilities, non_standard) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            model.id,
            model.provider_id,
            model.standard_model_id,
            model.upstream_model,
            model.exposed_name,
            serde_json::to_string(&model.capabilities)?,
            model.non_standard
        ],
    )
    .map_err(super::map_sqlite)?;
    Ok(model)
}
