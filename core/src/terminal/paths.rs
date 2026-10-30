//! Config path resolution for terminal clients.
//!
//! `ClientCtx.home` stands in for the user's home directory and every
//! environment variable is read from `ClientCtx.env`, which keeps resolution
//! deterministic in tests.

use super::spec::ClientCtx;
use std::path::PathBuf;

/// The value of `name` in the injected environment, when non-empty.
pub fn env_path(ctx: &ClientCtx, name: &str) -> Option<PathBuf> {
    ctx.env
        .get(name)
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
}

/// `~/.config` unless `XDG_CONFIG_HOME` points elsewhere.
pub fn config_home(ctx: &ClientCtx) -> PathBuf {
    env_path(ctx, "XDG_CONFIG_HOME").unwrap_or_else(|| ctx.home.join(".config"))
}
