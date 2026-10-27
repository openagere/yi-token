//! Runtime directory layout (`$YI_TOKEN_HOME`, default `~/.yi-token`).

use crate::error::{CoreError, CoreResult};
use std::ffi::OsString;
use std::path::PathBuf;

const HOME_VAR: &str = "YI_TOKEN_HOME";
const CATALOG_VAR: &str = "YI_TOKEN_MODEL_CATALOG";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub home: PathBuf,
    pub db: PathBuf,
    pub log_dir: PathBuf,
    pub catalog_override: Option<PathBuf>,
}

impl AppPaths {
    pub fn from_env() -> CoreResult<Self> {
        Self::from_lookup(|k| std::env::var_os(k))
    }

    /// Layout rooted at `home`, without a catalog override.
    pub fn under(home: PathBuf) -> Self {
        Self {
            db: home.join("yi-token.db"),
            log_dir: home.join("logs"),
            home,
            catalog_override: None,
        }
    }

    /// Environment-independent core of `from_env`; empty values count as unset.
    fn from_lookup(get: impl Fn(&str) -> Option<OsString>) -> CoreResult<Self> {
        let get = |k: &str| get(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        let home = match get(HOME_VAR) {
            Some(h) => h,
            None => get("HOME")
                .or_else(|| get("USERPROFILE"))
                .map(|h| h.join(".yi-token"))
                .ok_or_else(|| CoreError::Internal("无法确定用户主目录".into()))?,
        };
        let mut paths = Self::under(home);
        paths.catalog_override = get(CATALOG_VAR);
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // The workspace forbids unsafe code, so env vars cannot be mutated in
    // tests (`set_var` is unsafe in edition 2024); inject a lookup instead.
    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, OsString> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn paths_respect_yi_token_home() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_str().unwrap();
        let p = AppPaths::from_lookup(lookup(&[(HOME_VAR, home), ("HOME", "/ignored")])).unwrap();
        assert_eq!(p.home, tmp.path());
        assert_eq!(p.db, tmp.path().join("yi-token.db"));
        assert_eq!(p.log_dir, tmp.path().join("logs"));
        assert_eq!(p.catalog_override, None);
    }

    #[test]
    fn paths_default_to_dot_yi_token() {
        let p = AppPaths::from_lookup(lookup(&[("HOME", "/u"), (HOME_VAR, "")])).unwrap();
        assert_eq!(p.home, PathBuf::from("/u").join(".yi-token"));
        assert!(p.home.ends_with(".yi-token"));
        let p = AppPaths::from_lookup(lookup(&[("USERPROFILE", "C:/u")])).unwrap();
        assert!(p.home.ends_with(".yi-token"));
    }

    #[test]
    fn missing_home_is_internal_error() {
        assert!(AppPaths::from_lookup(lookup(&[])).is_err());
    }

    #[test]
    fn catalog_override_from_env() {
        let p =
            AppPaths::from_lookup(lookup(&[(HOME_VAR, "/h"), (CATALOG_VAR, "/c.json")])).unwrap();
        assert_eq!(p.catalog_override, Some(PathBuf::from("/c.json")));
    }
}
