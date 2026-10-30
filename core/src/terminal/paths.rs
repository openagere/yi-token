//! Config path resolution for terminal clients.
//!
//! `ClientCtx.home` stands in for the user's home directory and every
//! environment variable is read from `ClientCtx.env`, which keeps resolution
//! deterministic in tests.

use super::spec::ClientCtx;
use std::path::PathBuf;

/// Expands a leading `~` in an environment-provided path to the injected home directory.
pub fn expand_home(ctx: &ClientCtx, path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    let Some(rest) = text.strip_prefix('~') else {
        return path;
    };
    let rest = rest.trim_start_matches(['/', '\\']);
    if rest.is_empty() {
        ctx.home.clone()
    } else {
        ctx.home.join(rest)
    }
}

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
