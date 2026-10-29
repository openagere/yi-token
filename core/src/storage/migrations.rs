//! Schema migrations (`PRAGMA user_version`, starting at 1).
//!
//! `rusqlite_migration` applies every pending migration together with its
//! version stamp inside one transaction, so a failing migration leaves the
//! database untouched (regression B2).

use crate::error::{CoreError, CoreResult};
use rusqlite::{Connection, Transaction};
use rusqlite_migration::{HookResult, M, Migrations};
use std::collections::{HashMap, HashSet};

const V1_SQL: &str = "
CREATE TABLE providers (
    id           TEXT PRIMARY KEY,
    type         TEXT NOT NULL CHECK (type IN ('anthropic', 'openai_chat', 'responses')),
    name         TEXT NOT NULL,
    short_code   TEXT NOT NULL COLLATE NOCASE UNIQUE,
    base_url     TEXT NOT NULL,
    api_key      TEXT NOT NULL DEFAULT '',
    enabled      INTEGER NOT NULL DEFAULT 1,
    thinking     TEXT CHECK (thinking IS NULL OR thinking IN ('minimal', 'low', 'medium', 'high', 'xhigh')),
    is_default   INTEGER NOT NULL DEFAULT 0,
    sort_order   INTEGER NOT NULL DEFAULT 0,
    extra        TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE provider_protocols (
    provider_id  TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
    protocol     TEXT NOT NULL CHECK (protocol IN ('anthropic', 'openai_chat', 'responses')),
    PRIMARY KEY (provider_id, protocol)
);

CREATE TABLE standard_models (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    protocol     TEXT NOT NULL CHECK (protocol IN ('anthropic', 'openai_chat', 'responses')),
    brand        TEXT NOT NULL DEFAULT '',
    capabilities TEXT NOT NULL DEFAULT '{}',
    UNIQUE (name, protocol)
);

CREATE TABLE models (
    id                TEXT PRIMARY KEY,
    provider_id       TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
    standard_model_id TEXT REFERENCES standard_models(id) ON DELETE RESTRICT,
    upstream_model    TEXT NOT NULL,
    exposed_name      TEXT NOT NULL UNIQUE,
    capabilities      TEXT NOT NULL DEFAULT '{}',
    non_standard      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_models_provider ON models(provider_id);
CREATE INDEX idx_models_standard ON models(standard_model_id);

CREATE TABLE terminal_proxy_profiles (
    client        TEXT PRIMARY KEY,
    protocol      TEXT NOT NULL CHECK (protocol IN ('anthropic', 'openai_chat', 'responses')),
    route_mode    TEXT NOT NULL CHECK (route_mode IN ('fixed', 'auto')),
    models        TEXT NOT NULL DEFAULT '[]',
    default_model TEXT
);

CREATE TABLE terminal_direct_profiles (
    client       TEXT PRIMARY KEY,
    provider_id  TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
    model_source TEXT NOT NULL CHECK (model_source IN ('native', 'provider')),
    model        TEXT
);

CREATE TABLE terminal_engines (
    client   TEXT PRIMARY KEY,
    model_id TEXT REFERENCES models(id) ON DELETE SET NULL
);

CREATE TABLE settings (
    id        INTEGER PRIMARY KEY CHECK (id = 1),
    host      TEXT NOT NULL DEFAULT '127.0.0.1',
    port      INTEGER NOT NULL DEFAULT 11435,
    log_level TEXT NOT NULL DEFAULT 'info' CHECK (log_level IN ('error', 'warn', 'info', 'debug', 'trace'))
);
INSERT INTO settings (id) VALUES (1);

CREATE TABLE usage (
    id               TEXT PRIMARY KEY,
    requested_at     INTEGER NOT NULL,
    client           TEXT,
    route_mode       TEXT CHECK (route_mode IS NULL OR route_mode IN ('fixed', 'auto')),
    provider_name    TEXT NOT NULL,
    protocol         TEXT NOT NULL,
    model            TEXT NOT NULL,
    input_tokens     INTEGER NOT NULL DEFAULT 0,
    output_tokens    INTEGER NOT NULL DEFAULT 0,
    cached_tokens    INTEGER NOT NULL DEFAULT 0,
    reasoning_tokens INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_usage_requested_at ON usage(requested_at);
CREATE INDEX idx_usage_client ON usage(client);
";

/// The initial schema migration.
pub fn v1() -> M<'static> {
    M::up(V1_SQL)
}

/// Records the engine the written terminal configuration was built with, so that
/// switching the proxy engine later does not read as a configuration change.
pub fn v2() -> M<'static> {
    M::up("ALTER TABLE terminal_proxy_profiles ADD COLUMN applied_engine TEXT;")
}

/// Drops provider short codes (exposed model names become `{provider}/{model}`) and
/// opens every provider to all client protocols, which yi-token translates.
/// The `short_code` column stays (SQLite cannot drop a UNIQUE column in place) and
/// only mirrors the id from now on.
pub fn v3() -> M<'static> {
    M::up_with_hook(
        "UPDATE providers SET short_code = id;
         INSERT OR IGNORE INTO provider_protocols (provider_id, protocol)
           SELECT p.id, v.protocol FROM providers p
           CROSS JOIN (SELECT 'anthropic' AS protocol UNION ALL SELECT 'openai_chat' UNION ALL SELECT 'responses') v;",
        rename_exposed_models,
    )
}

fn rename_exposed_models(tx: &Transaction) -> HookResult {
    use crate::domain::{exposed_name, provider_slug};
    // Provider names now prefix every route, so names that fold together get a suffix.
    let providers: Vec<(String, String)> = {
        let mut stmt = tx.prepare("SELECT id, name FROM providers ORDER BY sort_order, rowid")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut taken = HashSet::new();
    let mut names = HashMap::new();
    for (id, name) in providers {
        let base = if provider_slug(&name).is_empty() { "Provider".to_owned() } else { name.trim().to_owned() };
        let mut candidate = base.clone();
        let mut n = 2;
        while !taken.insert(provider_slug(&candidate).to_lowercase()) {
            candidate = format!("{base} {n}");
            n += 1;
        }
        if candidate != name {
            tx.execute("UPDATE providers SET name = ?1 WHERE id = ?2", [&candidate, &id])?;
        }
        names.insert(id, candidate);
    }
    let models: Vec<(String, String, String, String)> = {
        let mut stmt = tx.prepare("SELECT id, provider_id, upstream_model, exposed_name FROM models")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    // Park every name first so the new names never collide with old ones mid-update.
    tx.execute("UPDATE models SET exposed_name = 'migrating:' || id", [])?;
    let mut renamed = HashMap::new();
    for (id, provider_id, upstream, old) in models {
        let provider = names.get(&provider_id).map(String::as_str).unwrap_or("Provider");
        let new = exposed_name(provider, &upstream);
        tx.execute("UPDATE models SET exposed_name = ?1 WHERE id = ?2", [&new, &id])?;
        renamed.insert(old, new);
    }
    let profiles: Vec<(String, String, Option<String>)> = {
        let mut stmt = tx.prepare("SELECT client, models, default_model FROM terminal_proxy_profiles")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    for (client, raw, default_model) in profiles {
        let mut list: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Array(Vec::new()));
        if let Some(items) = list.as_array_mut() {
            for item in items {
                if let Some(new) = item.get("model").and_then(|m| m.as_str()).and_then(|m| renamed.get(m)) {
                    item["model"] = serde_json::Value::String(new.clone());
                }
            }
        }
        let default_model = default_model.map(|m| renamed.get(&m).cloned().unwrap_or(m));
        tx.execute(
            "UPDATE terminal_proxy_profiles SET models = ?1, default_model = ?2 WHERE client = ?3",
            rusqlite::params![list.to_string(), default_model, client],
        )?;
    }
    Ok(())
}

/// Session trajectory columns: request metrics on `usage` and the recording settings on
/// `settings`. Every new column is nullable or has a default, so existing rows stay valid.
pub fn v4() -> M<'static> {
    M::up(
        "ALTER TABLE usage ADD COLUMN step_id TEXT;
         ALTER TABLE usage ADD COLUMN session_id TEXT;
         ALTER TABLE usage ADD COLUMN ttft_ms INTEGER;
         ALTER TABLE usage ADD COLUMN duration_ms INTEGER;
         ALTER TABLE usage ADD COLUMN status INTEGER;
         CREATE INDEX idx_usage_session ON usage(session_id);
         ALTER TABLE settings ADD COLUMN trajectory_enabled INTEGER NOT NULL DEFAULT 1;
         ALTER TABLE settings ADD COLUMN trajectory_disabled_clients TEXT NOT NULL DEFAULT '[]';
         ALTER TABLE settings ADD COLUMN trajectory_retention_days INTEGER NOT NULL DEFAULT 30;
         ALTER TABLE settings ADD COLUMN trajectory_max_bytes INTEGER NOT NULL DEFAULT 2147483648;
         ALTER TABLE settings ADD COLUMN trajectory_notice_seen INTEGER NOT NULL DEFAULT 0;",
    )
}

/// The global route (`api`) gets its own recording switch, since `Client` has no such variant.
pub fn v5() -> M<'static> {
    M::up("ALTER TABLE settings ADD COLUMN trajectory_disabled_global INTEGER NOT NULL DEFAULT 0;")
}

/// Standard models move from wire protocols to vendors: Chat and Responses share
/// OpenAI's catalog, so rows that only differ by that protocol are merged (the
/// Responses row wins) and provider models are re-pointed at the survivor. Provider
/// models gain an optional protocol override, `NULL` meaning "follow the provider".
///
/// The table is rebuilt because SQLite cannot alter the protocol CHECK constraint.
/// Model links are parked in a temp table first so that the RESTRICT foreign key
/// does not block the drop.
pub fn v6() -> M<'static> {
    M::up(
        "CREATE TABLE standard_models_v6 (
             id           TEXT PRIMARY KEY,
             name         TEXT NOT NULL,
             company      TEXT NOT NULL CHECK (company IN ('openai', 'anthropic')),
             brand        TEXT NOT NULL DEFAULT '',
             capabilities TEXT NOT NULL DEFAULT '{}',
             UNIQUE (name, company)
         );
         INSERT INTO standard_models_v6 (id, name, company, brand, capabilities)
           SELECT id, name, company, brand, capabilities FROM (
             SELECT id, name, brand, capabilities,
               CASE protocol WHEN 'anthropic' THEN 'anthropic' ELSE 'openai' END AS company,
               ROW_NUMBER() OVER (
                 PARTITION BY name, CASE protocol WHEN 'anthropic' THEN 'anthropic' ELSE 'openai' END
                 ORDER BY CASE protocol WHEN 'responses' THEN 0 ELSE 1 END, rowid
               ) AS rn
             FROM standard_models
           ) WHERE rn = 1;
         CREATE TEMP TABLE standard_remap AS
           SELECT s.id AS old_id, v.id AS new_id FROM standard_models s
           JOIN standard_models_v6 v ON v.name = s.name
             AND v.company = CASE s.protocol WHEN 'anthropic' THEN 'anthropic' ELSE 'openai' END;
         CREATE TEMP TABLE model_standard AS
           SELECT m.id AS model_id, r.new_id AS standard_id FROM models m
           JOIN standard_remap r ON r.old_id = m.standard_model_id;
         UPDATE models SET standard_model_id = NULL WHERE standard_model_id IS NOT NULL;
         DROP TABLE standard_models;
         ALTER TABLE standard_models_v6 RENAME TO standard_models;
         UPDATE models SET standard_model_id = (
           SELECT standard_id FROM model_standard WHERE model_id = models.id
         ) WHERE id IN (SELECT model_id FROM model_standard);
         DROP TABLE model_standard;
         DROP TABLE standard_remap;
         ALTER TABLE models ADD COLUMN protocol TEXT
           CHECK (protocol IS NULL OR protocol IN ('anthropic', 'openai_chat', 'responses'));",
    )
}

pub fn migrations() -> Migrations<'static> {
    Migrations::new(vec![v1(), v2(), v3(), v4(), v5(), v6()])
}

pub fn run(conn: &mut Connection) -> CoreResult<()> {
    migrations()
        .to_latest(conn)
        .map_err(|e| CoreError::Database(format!("数据库迁移失败：{e}")))
}
