//! r2d2 connection pool over SQLite.

use super::migrations;
use crate::error::CoreResult;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{Connection, Transaction};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const MAX_CONNECTIONS: u32 = 8;
const PRAGMAS: &str = "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;";

static MEMORY_SEQ: AtomicU64 = AtomicU64::new(0);

/// Shared SQLite handle. Cheap to clone; every clone draws from the same pool.
#[derive(Clone)]
pub struct Db {
    pool: Pool<SqliteConnectionManager>,
}

impl Db {
    /// Opens (creating if needed) the database file and applies pending migrations.
    pub fn open(path: &Path) -> CoreResult<Db> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        Self::build(SqliteConnectionManager::file(path), MAX_CONNECTIONS)
    }

    /// A private in-memory database (shared-cache URI so every pooled connection sees it).
    pub fn open_in_memory() -> CoreResult<Db> {
        let seq = MEMORY_SEQ.fetch_add(1, Ordering::Relaxed);
        let uri = format!(
            "file:yi-token-mem-{}-{seq}?mode=memory&cache=shared",
            std::process::id()
        );
        Self::build(SqliteConnectionManager::file(uri), 4)
    }

    fn build(manager: SqliteConnectionManager, max_size: u32) -> CoreResult<Db> {
        let manager = manager.with_init(|conn| conn.execute_batch(PRAGMAS));
        let pool = Pool::builder()
            .max_size(max_size)
            .connection_timeout(Duration::from_secs(10))
            .build(manager)?;
        {
            let mut conn = pool.get()?;
            migrations::run(&mut conn)?;
        }
        Ok(Db { pool })
    }

    pub fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> CoreResult<T>) -> CoreResult<T> {
        let conn = self.pool.get()?;
        f(&conn)
    }

    /// Runs `f` in a transaction: committed on `Ok`, rolled back on `Err`.
    pub fn with_tx<T>(&self, f: impl FnOnce(&Transaction) -> CoreResult<T>) -> CoreResult<T> {
        let mut conn = self.pool.get()?;
        let tx = conn.transaction()?;
        let value = f(&tx)?;
        tx.commit()?;
        Ok(value)
    }
}
