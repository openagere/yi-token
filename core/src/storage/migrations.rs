//! Schema of the main database.
//!
//! There is a single baseline. v0.2.0 is a fresh start: databases written by earlier versions
//! are not upgraded, and the app opens a new file instead (see `paths`). Future changes are
//! appended as new migrations after this one.

use crate::error::{CoreError, CoreResult};
use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};

const BASELINE_SQL: &str = "
CREATE TABLE providers (
    id           TEXT PRIMARY KEY,
    type         TEXT NOT NULL CHECK (type IN ('anthropic', 'openai_chat', 'responses')),
    name         TEXT NOT NULL,
    base_url     TEXT NOT NULL,
    api_key      TEXT NOT NULL DEFAULT '',
    enabled      INTEGER NOT NULL DEFAULT 1,
    thinking     TEXT CHECK (thinking IS NULL OR thinking IN ('minimal', 'low', 'medium', 'high', 'xhigh')),
    is_default   INTEGER NOT NULL DEFAULT 0,
    sort_order   INTEGER NOT NULL DEFAULT 0,
    extra        TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE standard_models (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    company      TEXT NOT NULL CHECK (company IN ('openai', 'anthropic')),
    brand        TEXT NOT NULL DEFAULT '',
    capabilities TEXT NOT NULL DEFAULT '{}',
    UNIQUE (name, company)
);

CREATE TABLE models (
    id                TEXT PRIMARY KEY,
    provider_id       TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
    standard_model_id TEXT REFERENCES standard_models(id) ON DELETE RESTRICT,
    upstream_model    TEXT NOT NULL,
    exposed_name      TEXT NOT NULL UNIQUE,
    protocol          TEXT CHECK (protocol IS NULL OR protocol IN ('anthropic', 'openai_chat', 'responses')),
    capabilities      TEXT NOT NULL DEFAULT '{}',
    non_standard      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_models_provider ON models(provider_id);
CREATE INDEX idx_models_standard ON models(standard_model_id);

CREATE TABLE terminal_proxy_profiles (
    client         TEXT PRIMARY KEY,
    protocol       TEXT NOT NULL CHECK (protocol IN ('anthropic', 'openai_chat', 'responses')),
    route_mode     TEXT NOT NULL CHECK (route_mode IN ('fixed', 'auto')),
    models         TEXT NOT NULL DEFAULT '[]',
    default_model  TEXT,
    applied_engine TEXT
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
    id                          INTEGER PRIMARY KEY CHECK (id = 1),
    host                        TEXT NOT NULL DEFAULT '127.0.0.1',
    port                        INTEGER NOT NULL DEFAULT 11435,
    log_level                   TEXT NOT NULL DEFAULT 'info' CHECK (log_level IN ('error', 'warn', 'info', 'debug', 'trace')),
    trajectory_enabled          INTEGER NOT NULL DEFAULT 1,
    trajectory_disabled_clients TEXT NOT NULL DEFAULT '[]',
    trajectory_retention_days   INTEGER NOT NULL DEFAULT 30,
    trajectory_max_bytes        INTEGER NOT NULL DEFAULT 2147483648,
    trajectory_notice_seen      INTEGER NOT NULL DEFAULT 0,
    trajectory_disabled_global  INTEGER NOT NULL DEFAULT 0
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
    reasoning_tokens INTEGER NOT NULL DEFAULT 0,
    step_id          TEXT,
    session_id       TEXT,
    ttft_ms          INTEGER,
    duration_ms      INTEGER,
    status           INTEGER
);
CREATE INDEX idx_usage_requested_at ON usage(requested_at);
CREATE INDEX idx_usage_client ON usage(client);
CREATE INDEX idx_usage_session ON usage(session_id);

CREATE TABLE run_monitor (
    id       INTEGER PRIMARY KEY CHECK (id = 1),
    state    TEXT NOT NULL,
    saved_at INTEGER NOT NULL
);
";

pub fn migrations() -> Migrations<'static> {
    Migrations::new(vec![M::up(BASELINE_SQL)])
}

pub fn run(conn: &mut Connection) -> CoreResult<()> {
    migrations()
        .to_latest(conn)
        .map_err(|e| CoreError::Database(format!("数据库迁移失败：{e}")))
}
