//! Schema migrations for the trajectory database (`PRAGMA user_version`, starting at 1).

use crate::error::{CoreError, CoreResult};
use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};

const V1_SQL: &str = "
PRAGMA auto_vacuum = INCREMENTAL;
CREATE TABLE sessions (
    id               TEXT PRIMARY KEY,
    client           TEXT NOT NULL,
    client_key       TEXT,
    key_source       TEXT NOT NULL CHECK (key_source IN ('client', 'chain', 'new')),
    title            TEXT NOT NULL DEFAULT '',
    started_at       INTEGER NOT NULL,
    last_at          INTEGER NOT NULL,
    step_count       INTEGER NOT NULL DEFAULT 0,
    input_tokens     INTEGER NOT NULL DEFAULT 0,
    output_tokens    INTEGER NOT NULL DEFAULT 0,
    bytes            INTEGER NOT NULL DEFAULT 0,
    UNIQUE (client, client_key)
);
CREATE INDEX idx_sessions_last_at ON sessions(last_at);
CREATE INDEX idx_sessions_client_last_at ON sessions(client, last_at);

CREATE TABLE threads (
    id               TEXT PRIMARY KEY,
    session_id       TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    kind             TEXT NOT NULL CHECK (kind IN ('main', 'subagent', 'side', 'pending')),
    parent_thread_id TEXT REFERENCES threads(id) ON DELETE SET NULL,
    spawn_call_id    TEXT,
    reset            INTEGER NOT NULL DEFAULT 0,
    started_at       INTEGER NOT NULL
);
CREATE INDEX idx_threads_session ON threads(session_id);

CREATE TABLE steps (
    id                TEXT PRIMARY KEY,
    seq               INTEGER NOT NULL UNIQUE,
    session_id        TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    thread_id         TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    parent_step_id    TEXT REFERENCES steps(id) ON DELETE SET NULL,
    prefix_len        INTEGER NOT NULL,
    checkpoint        INTEGER NOT NULL DEFAULT 0,
    depth             INTEGER NOT NULL DEFAULT 0,
    turn_no           INTEGER NOT NULL,
    client_turn_id    TEXT,
    requested_at      INTEGER NOT NULL,
    first_token_at    INTEGER,
    finished_at       INTEGER,
    status            INTEGER,
    stop_reason       TEXT NOT NULL,
    error             TEXT,
    inbound_protocol  TEXT NOT NULL,
    upstream_protocol TEXT NOT NULL,
    provider_name     TEXT NOT NULL,
    model             TEXT NOT NULL,
    upstream_model    TEXT NOT NULL,
    route_mode        TEXT,
    input_tokens      INTEGER NOT NULL DEFAULT 0,
    output_tokens     INTEGER NOT NULL DEFAULT 0,
    cached_tokens     INTEGER NOT NULL DEFAULT 0,
    reasoning_tokens  INTEGER NOT NULL DEFAULT 0,
    system_blob       TEXT,
    tools_blob        TEXT,
    options_json      TEXT NOT NULL DEFAULT '{}',
    tail_fp           TEXT NOT NULL,
    capture           TEXT NOT NULL CHECK (capture IN ('full', 'metadata_only'))
);
CREATE INDEX idx_steps_session_turn ON steps(session_id, turn_no);
CREATE INDEX idx_steps_thread ON steps(thread_id, seq);
CREATE INDEX idx_steps_tail_fp ON steps(tail_fp);
CREATE INDEX idx_steps_system_blob ON steps(system_blob) WHERE system_blob IS NOT NULL;
CREATE INDEX idx_steps_tools_blob ON steps(tools_blob) WHERE tools_blob IS NOT NULL;

CREATE TABLE step_items (
    step_id   TEXT NOT NULL REFERENCES steps(id) ON DELETE CASCADE,
    side      TEXT NOT NULL CHECK (side IN ('input', 'output')),
    ord       INTEGER NOT NULL,
    blob      TEXT NOT NULL,
    kind      TEXT NOT NULL,
    role      TEXT,
    call_id   TEXT,
    tool_name TEXT,
    PRIMARY KEY (step_id, side, ord)
);
CREATE INDEX idx_step_items_call ON step_items(call_id) WHERE call_id IS NOT NULL;
CREATE INDEX idx_step_items_blob ON step_items(blob);

CREATE TABLE blobs (
    hash      TEXT PRIMARY KEY,
    kind      TEXT NOT NULL,
    raw_bytes INTEGER NOT NULL,
    data      BLOB NOT NULL,
    truncated INTEGER NOT NULL DEFAULT 0
);
";

/// v2: a monotonic sequence counter, so `steps.seq` never reuses a number after a delete.
/// Existing databases start from their current maximum.
const V2_SQL: &str = "
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value INTEGER NOT NULL
);
INSERT INTO meta (key, value) VALUES ('last_seq', COALESCE((SELECT MAX(seq) FROM steps), 0));
";

/// Migrations for the trajectory database, in order.
/// Titles taken from injected context (for example Codex's `<environment_context>`) are
/// cleared. The writer fills an empty title again from the next request, which carries the
/// conversation, so the correct title comes back without rewriting anything else.
const V3_SQL: &str = "
UPDATE sessions SET title = ''
WHERE title LIKE '<%' OR title LIKE '# AGENTS.md%' OR title LIKE '# CLAUDE.md%';
";

pub fn migrations() -> Migrations<'static> {
    Migrations::new(vec![M::up(V1_SQL), M::up(V2_SQL), M::up(V3_SQL)])
}

/// Applies every pending trajectory migration. The first statement of v1 sets `auto_vacuum`
/// before any table exists.
pub fn run(conn: &mut Connection) -> CoreResult<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version == 0 {
        // The pooled connections already run `journal_mode=WAL`, which writes page 1, and then
        // a plain `auto_vacuum` pragma is silently ignored. VACUUM on the still-empty database
        // rebuilds the file so the setting takes effect. It must run outside a transaction.
        conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL; VACUUM;")?;
    }
    migrations()
        .to_latest(conn)
        .map_err(|e| CoreError::Database(format!("轨迹库迁移失败：{e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v3_clears_titles_taken_from_injected_context() {
        let mut conn = Connection::open_in_memory().unwrap();
        Migrations::new(vec![M::up(V1_SQL), M::up(V2_SQL)])
            .to_latest(&mut conn)
            .unwrap();
        conn.execute_batch(
            "INSERT INTO sessions (id, client, key_source, title, started_at, last_at) VALUES
               ('bad', 'codex', 'new', '<environment_context>', 1, 1),
               ('good', 'codex', 'new', '修复登录', 1, 1);",
        )
        .unwrap();
        migrations().to_latest(&mut conn).unwrap();
        let titles: Vec<(String, String)> = conn
            .prepare("SELECT id, title FROM sessions ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            titles,
            [("bad".into(), String::new()), ("good".into(), "修复登录".into())]
        );
    }
}
