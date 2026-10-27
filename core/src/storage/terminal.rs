//! Terminal client profiles (proxy and direct mode).

use super::{enum_str, map_sqlite, parse_enum, parse_json, query_as};
use crate::domain::{DirectProfile, ProfileModel, ProxyProfile};
use crate::domain::Client;
use crate::error::CoreResult;
use rusqlite::{Connection, params};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
struct ProxyRow {
    client: String,
    protocol: String,
    route_mode: String,
    models: String,
    default_model: Option<String>,
}

impl ProxyRow {
    fn into_profile(self) -> CoreResult<ProxyProfile> {
        Ok(ProxyProfile {
            client: parse_enum(&self.client)?,
            protocol: parse_enum(&self.protocol)?,
            route_mode: parse_enum(&self.route_mode)?,
            models: parse_json::<Vec<ProfileModel>>(&self.models, "终端模型配置")?,
            default_model: self.default_model,
        })
    }
}

#[derive(Deserialize)]
struct DirectRow {
    client: String,
    provider_id: String,
    model_source: String,
    model: Option<String>,
}

impl DirectRow {
    fn into_profile(self) -> CoreResult<DirectProfile> {
        Ok(DirectProfile {
            client: parse_enum(&self.client)?,
            model_source: parse_enum(&self.model_source)?,
            provider_id: self.provider_id,
            model: self.model,
        })
    }
}

const PROXY_SELECT: &str =
    "SELECT client, protocol, route_mode, models, default_model FROM terminal_proxy_profiles";
const DIRECT_SELECT: &str =
    "SELECT client, provider_id, model_source, model FROM terminal_direct_profiles";

pub fn list_proxy(conn: &Connection) -> CoreResult<Vec<ProxyProfile>> {
    query_as::<ProxyRow>(conn, &format!("{PROXY_SELECT} ORDER BY rowid"), [])?
        .into_iter()
        .map(ProxyRow::into_profile)
        .collect()
}

pub fn get_proxy(conn: &Connection, client: &str) -> CoreResult<Option<ProxyProfile>> {
    query_as::<ProxyRow>(conn, &format!("{PROXY_SELECT} WHERE client = ?1"), [client])?
        .into_iter()
        .next()
        .map(ProxyRow::into_profile)
        .transpose()
}

/// Validates and stores a proxy profile (one per client, replaced wholesale).
pub fn save_proxy(conn: &Connection, profile: &ProxyProfile) -> CoreResult<()> {
    profile.validate()?;
    conn.execute(
        "INSERT INTO terminal_proxy_profiles (client, protocol, route_mode, models, default_model) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(client) DO UPDATE SET protocol = excluded.protocol, \
           route_mode = excluded.route_mode, models = excluded.models, \
           default_model = excluded.default_model",
        params![
            enum_str(&profile.client)?,
            enum_str(&profile.protocol)?,
            enum_str(&profile.route_mode)?,
            serde_json::to_string(&profile.models)?,
            profile.default_model,
        ],
    )
    .map_err(map_sqlite)?;
    Ok(())
}

/// Records the engine id the proxy configuration was written with; `None` (no engine) is stored as "".
pub fn save_proxy_applied_engine(
    conn: &Connection,
    client: Client,
    engine_id: Option<&str>,
) -> CoreResult<()> {
    conn.execute(
        "UPDATE terminal_proxy_profiles SET applied_engine = ?2 WHERE client = ?1",
        params![enum_str(&client)?, engine_id.unwrap_or("")],
    )
    .map_err(map_sqlite)?;
    Ok(())
}

/// Engine ids recorded at apply time per client ("" = applied without an engine).
/// Clients absent from the map have no record (profiles written before the record existed).
pub fn list_proxy_applied_engines(conn: &Connection) -> CoreResult<HashMap<Client, String>> {
    let mut stmt = conn
        .prepare("SELECT client, applied_engine FROM terminal_proxy_profiles WHERE applied_engine IS NOT NULL")
        .map_err(map_sqlite)?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(map_sqlite)?;
    let mut engines = HashMap::new();
    for row in rows {
        let (client, engine_id) = row.map_err(map_sqlite)?;
        engines.insert(parse_enum(&client)?, engine_id);
    }
    Ok(engines)
}

pub fn list_direct(conn: &Connection) -> CoreResult<Vec<DirectProfile>> {
    query_as::<DirectRow>(conn, &format!("{DIRECT_SELECT} ORDER BY rowid"), [])?
        .into_iter()
        .map(DirectRow::into_profile)
        .collect()
}

pub fn get_direct(conn: &Connection, client: &str) -> CoreResult<Option<DirectProfile>> {
    query_as::<DirectRow>(
        conn,
        &format!("{DIRECT_SELECT} WHERE client = ?1"),
        [client],
    )?
    .into_iter()
    .next()
    .map(DirectRow::into_profile)
    .transpose()
}

/// Stores a direct profile; the provider must exist (foreign key).
pub fn save_direct(conn: &Connection, profile: &DirectProfile) -> CoreResult<()> {
    conn.execute(
        "INSERT INTO terminal_direct_profiles (client, provider_id, model_source, model) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(client) DO UPDATE SET provider_id = excluded.provider_id, \
           model_source = excluded.model_source, model = excluded.model",
        params![
            enum_str(&profile.client)?,
            profile.provider_id,
            enum_str(&profile.model_source)?,
            profile.model,
        ],
    )
    .map_err(map_sqlite)?;
    Ok(())
}
