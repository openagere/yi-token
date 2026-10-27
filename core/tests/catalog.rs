use std::time::{Duration, SystemTime};
use yi_core::catalog::{CatalogInfo, CatalogWatcher, file, sync};
use yi_core::domain::{Capabilities, Protocol, StandardModel};
use yi_core::storage::{Db, standard_models};

const OVERRIDE: &str = r#"{"version":1,"models":[
  {"id":"x1","name":"alpha","protocol":"responses","capabilities":{"context_window":1000}},
  {"name":"beta","protocol":"anthropic"}]}"#;

fn write_override(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let p = dir.path().join("catalog.json");
    std::fs::write(&p, OVERRIDE).unwrap();
    p
}

#[test]
fn builtin_catalog_parses() {
    let f = file::load(None).unwrap();
    assert!(!f.entries.is_empty());
    for e in &f.entries {
        e.capabilities.validate(e.protocol).unwrap();
    }
    let info = CatalogInfo::new(&f, None);
    assert_eq!(
        (info.source.as_str(), info.count),
        ("builtin", f.entries.len())
    );
}

#[test]
fn override_path_is_used() {
    let dir = tempfile::tempdir().unwrap();
    let p = write_override(&dir);
    let f = file::load(Some(&p)).unwrap();
    assert_eq!(f.entries.len(), 2);
    assert_eq!(f.entries[0].name, "alpha");
    assert_eq!(f.entries[1].id, "");
    assert_eq!(CatalogInfo::new(&f, Some(&p)).source, "override");
    assert!(file::load(Some(&dir.path().join("missing.json"))).is_err());
}

#[test]
fn malformed_override_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("catalog.json");
    std::fs::write(&p, r#"{"version":1,"models":[{"name":"alpha"}]}"#).unwrap();
    let err = file::load(Some(&p)).unwrap_err();
    assert_eq!(err.code(), yi_core::error::ErrorCode::Validation);
    std::fs::write(&p, r#"{"version":2,"models":[]}"#).unwrap();
    assert!(file::load(Some(&p)).is_err());
}

#[test]
fn sync_is_idempotent() {
    let db = Db::open_in_memory().unwrap();
    let f = file::load(None).unwrap();
    db.with_tx(|tx| sync::apply(tx, &f)).unwrap();
    let first = db.with_conn(standard_models::list).unwrap();
    assert_eq!(first.len(), f.entries.len());
    db.with_tx(|tx| sync::apply(tx, &f)).unwrap();
    assert_eq!(db.with_conn(standard_models::list).unwrap(), first);
}

#[test]
fn sync_preserves_user_entries() {
    let db = Db::open_in_memory().unwrap();
    let user = StandardModel {
        id: "user-1".into(),
        name: "my-own".into(),
        protocol: Protocol::OpenaiChat,
        brand: String::new(),
        capabilities: Capabilities::default(),
    };
    db.with_conn(|c| standard_models::upsert(c, &user)).unwrap();
    let f = file::load(None).unwrap();
    db.with_tx(|tx| sync::apply(tx, &f)).unwrap();
    let got = db.with_conn(|c| standard_models::get(c, "user-1")).unwrap();
    assert_eq!(got, Some(user));
}

#[test]
fn watcher_detects_mtime_change() {
    let dir = tempfile::tempdir().unwrap();
    let p = write_override(&dir);
    let mut w = CatalogWatcher::new(Some(p.clone()));
    assert!(!w.changed());
    let f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
    f.set_modified(SystemTime::now() + Duration::from_secs(60))
        .unwrap();
    assert!(w.changed());
    assert!(!w.changed());
    assert!(!CatalogWatcher::new(None).changed());
}
