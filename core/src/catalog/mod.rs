//! Model catalog: loading the built-in or overridden file and syncing it into storage.

pub mod file;
pub mod sync;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use file::{CatalogFile, load};

/// Environment variable naming a catalog file that replaces the built-in one.
pub const CATALOG_ENV: &str = "YI_TOKEN_MODEL_CATALOG";

/// Where the active catalog came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct CatalogInfo {
    pub source: String,
    pub path: Option<String>,
    pub version: String,
    pub count: usize,
    #[ts(type = "number")]
    pub loaded_at: i64,
}

impl CatalogInfo {
    pub fn new(file: &CatalogFile, path: Option<&Path>) -> Self {
        Self {
            source: if path.is_some() {
                "override"
            } else {
                "builtin"
            }
            .into(),
            path: path.map(|p| p.display().to_string()),
            version: file.version.clone(),
            count: file.entries.len(),
            loaded_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_secs() as i64,
        }
    }
}

/// Override path from `$YI_TOKEN_MODEL_CATALOG`, if set and non-empty.
pub fn override_path_from_env() -> Option<PathBuf> {
    std::env::var_os(CATALOG_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Detects changes of the override file by modification time.
///
/// Without an override path (built-in catalog) it never reports a change.
#[derive(Debug)]
pub struct CatalogWatcher {
    path: Option<PathBuf>,
    last_mtime: Option<SystemTime>,
}

impl CatalogWatcher {
    pub fn new(path: Option<PathBuf>) -> Self {
        let last_mtime = path.as_deref().and_then(mtime);
        Self { path, last_mtime }
    }

    /// True once per modification of the override file.
    pub fn changed(&mut self) -> bool {
        let Some(path) = &self.path else {
            return false;
        };
        let current = mtime(path);
        if current == self.last_mtime {
            return false;
        }
        self.last_mtime = current;
        true
    }
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}
