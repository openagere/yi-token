//! Trajectory database: a separate SQLite file recording proxied sessions.
//!
//! It has its own migration numbering, independent of the main database.

pub mod blobs;
pub mod migrations;
pub mod repo;
pub mod retention;
pub mod writer;

use super::pool::Db;
use crate::error::CoreResult;
use std::path::Path;

/// Handle to the trajectory database. Wraps the shared pool so repositories can
/// reuse the same connection helpers as the main database.
pub struct TrajectoryDb(pub Db);

impl TrajectoryDb {
    /// Opens (creating if needed) the trajectory database file at `path`.
    pub fn open(path: &Path) -> CoreResult<TrajectoryDb> {
        Db::open_with(path, migrations::run).map(TrajectoryDb)
    }

    /// A private in-memory trajectory database.
    pub fn open_in_memory() -> CoreResult<TrajectoryDb> {
        Db::open_in_memory_with(migrations::run).map(TrajectoryDb)
    }
}
