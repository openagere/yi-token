//! Saved run page state: the proxy monitor, stored as one JSON row.

use super::map_sqlite;
use crate::error::CoreResult;
use rusqlite::{Connection, OptionalExtension, params};

/// The saved monitor state as JSON, if any has been saved.
pub fn load(conn: &Connection) -> CoreResult<Option<String>> {
    conn.query_row("SELECT state FROM run_monitor WHERE id = 1", [], |r| {
        r.get(0)
    })
    .optional()
    .map_err(map_sqlite)
}

/// Replaces the saved monitor state.
pub fn save(conn: &Connection, state: &str, saved_at: i64) -> CoreResult<()> {
    conn.execute(
        "INSERT INTO run_monitor (id, state, saved_at) VALUES (1, ?1, ?2) \
         ON CONFLICT(id) DO UPDATE SET state = excluded.state, saved_at = excluded.saved_at",
        params![state, saved_at],
    )
    .map_err(map_sqlite)?;
    Ok(())
}
