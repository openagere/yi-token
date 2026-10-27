//! Auto-routing engine selection per terminal client.

use super::{enum_str, map_sqlite, parse_enum};
use crate::domain::Client;
use crate::error::CoreResult;
use rusqlite::{Connection, OptionalExtension, params};

/// Every client that has a row, with its current engine model id (if any).
pub fn list(conn: &Connection) -> CoreResult<Vec<(Client, Option<String>)>> {
    let rows = conn
        .prepare("SELECT client, model_id FROM terminal_engines ORDER BY rowid")?
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(client, model)| Ok((parse_enum(&client)?, model)))
        .collect()
}

/// The engine model id of `client`; `None` when unselected.
pub fn get(conn: &Connection, client: Client) -> CoreResult<Option<String>> {
    let model: Option<Option<String>> = conn
        .query_row(
            "SELECT model_id FROM terminal_engines WHERE client = ?1",
            [enum_str(&client)?],
            |r| r.get(0),
        )
        .optional()?;
    Ok(model.flatten())
}

/// Selects (`Some`) or clears (`None`) the engine; the model must exist.
pub fn set(conn: &Connection, client: Client, model_id: Option<&str>) -> CoreResult<()> {
    conn.execute(
        "INSERT INTO terminal_engines (client, model_id) VALUES (?1, ?2) \
         ON CONFLICT(client) DO UPDATE SET model_id = excluded.model_id",
        params![enum_str(&client)?, model_id],
    )
    .map_err(map_sqlite)?;
    Ok(())
}
