//! Syncs a catalog file into the `standard_models` table.

use super::file::CatalogFile;
use crate::domain::StandardModel;
use crate::error::CoreResult;
use crate::storage::{enum_str, standard_models};
use rusqlite::{OptionalExtension, Transaction};

/// Inserts or updates catalog entries keyed by `(name, protocol)`.
///
/// Existing rows keep their id; new rows get a fresh UUID v4. Rows not in the
/// catalog (user-created) are never touched, so applying twice is a no-op.
pub fn apply(tx: &Transaction, file: &CatalogFile) -> CoreResult<()> {
    for entry in &file.entries {
        let protocol = enum_str(&entry.protocol)?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT id FROM standard_models WHERE name = ?1 AND protocol = ?2",
                [entry.name.as_str(), protocol.as_str()],
                |r| r.get(0),
            )
            .optional()?;
        let model = StandardModel {
            id: existing.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            ..entry.clone()
        };
        standard_models::upsert(tx, &model)?;
    }
    Ok(())
}
