use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use rusqlite::types::ValueRef;
use secrecy::SecretString;
use serde_json::Value;
use yi_core::ErrorCode;
use yi_core::backup::payload::{BackupSettings, EngineSelection, Payload};
use yi_core::backup::{self, import::ImportReport};
use yi_core::domain::{
    Capabilities, Client, DirectProfile, LogLevel, Model, ModelSource, ProfileModel, Protocol,
    Provider, ProxyProfile, RouteMode, Settings, StandardModel,
};
use yi_core::services::{Services, TerminalProfileView};
use yi_core::storage::{engines, models, providers, settings, standard_models, terminal};

const PASSPHRASE: &str = "correct horse battery";
const ITERATIONS: u32 = 10_000;

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn provider(id: &str, _code: &str, native: Protocol) -> Provider {
    Provider {
        id: id.into(),
        provider_type: native,
        name: format!("name-{id}"),
        base_url: "https://example.com/v1".into(),
        api_key: format!("sk-{id}"),
        enabled: true,
        thinking: None,
        is_default: false,
        sort_order: 0,
        extra: serde_json::json!({"source": "backup-test"}),
        protocols: vec![native],
    }
}

fn model(id: &str, upstream: &str) -> Model {
    Model {
        id: id.into(),
        provider_id: String::new(),
        standard_model_id: None,
        upstream_model: upstream.into(),
        exposed_name: String::new(),
        capabilities: Capabilities::default(),
        non_standard: false,
    }
}

fn save_provider(services: &Services, provider: Provider, model: Model) -> Model {
    let id = model.id.clone();
    services
        .db()
        .with_tx(|tx| providers::upsert(tx, &provider, &[model]))
        .unwrap();
    services
        .db()
        .with_conn(|conn| models::get(conn, &id))
        .unwrap()
        .unwrap()
}

fn empty_payload() -> Payload {
    Payload {
        exported_at: 1_700_000_000,
        app_version: "0.1.0".into(),
        standard_models: Vec::new(),
        providers: Vec::new(),
        terminal_proxy_profiles: Vec::new(),
        terminal_direct_profiles: Vec::new(),
        terminal_engines: Vec::new(),
        settings: BackupSettings::default(),
    }
}

fn encode(payload: &Payload) -> String {
    let bytes = backup::encode_with_iterations(payload, &secret(PASSPHRASE), ITERATIONS).unwrap();
    String::from_utf8(bytes).unwrap()
}

fn decode(text: &str) -> yi_core::CoreResult<Payload> {
    backup::decode(text, &secret(PASSPHRASE))
}

fn mutate(text: &str, change: impl FnOnce(&mut Value)) -> String {
    let mut value: Value = serde_json::from_str(text).unwrap();
    change(&mut value);
    value.to_string()
}

fn table_snapshot(services: &Services) -> Vec<(&'static str, Vec<Vec<String>>)> {
    const TABLES: [&str; 7] = [
        "providers",
        "provider_protocols",
        "standard_models",
        "models",
        "terminal_proxy_profiles",
        "terminal_direct_profiles",
        "settings",
    ];
    services
        .db()
        .with_conn(|conn| {
            TABLES
                .iter()
                .map(|table| {
                    let mut stmt = conn.prepare(&format!("SELECT * FROM {table}"))?;
                    let columns = stmt.column_count();
                    let mut rows = stmt
                        .query_map([], |row| {
                            (0..columns)
                                .map(|index| {
                                    Ok(match row.get_ref(index)? {
                                        ValueRef::Null => "NULL".into(),
                                        ValueRef::Integer(value) => value.to_string(),
                                        ValueRef::Real(value) => value.to_string(),
                                        ValueRef::Text(value) => {
                                            String::from_utf8_lossy(value).into_owned()
                                        }
                                        ValueRef::Blob(value) => BASE64.encode(value),
                                    })
                                })
                                .collect::<rusqlite::Result<Vec<_>>>()
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    rows.sort();
                    Ok((*table, rows))
                })
                .collect::<yi_core::CoreResult<Vec<_>>>()
        })
        .unwrap()
}

fn seed_roundtrip(services: &Services) {
    let standard = StandardModel {
        id: "backup-standard".into(),
        name: "Backup Standard".into(),
        protocol: Protocol::OpenaiChat,
        brand: "test".into(),
        capabilities: Capabilities::default(),
    };
    services
        .db()
        .with_conn(|conn| standard_models::upsert(conn, &standard))
        .unwrap();
    let mut backup_model = model("m1", "gpt-up");
    backup_model.standard_model_id = Some("backup-standard".into());
    let stored = save_provider(
        services,
        provider("p1", "oa", Protocol::OpenaiChat),
        backup_model,
    );
    let fixed = ProxyProfile {
        client: Client::Pi,
        protocol: Protocol::OpenaiChat,
        route_mode: RouteMode::Fixed,
        models: vec![ProfileModel {
            provider_id: "p1".into(),
            model: stored.exposed_name.clone(),
        }],
        default_model: Some(stored.exposed_name.clone()),
    };
    let direct = DirectProfile {
        client: Client::Opencode,
        provider_id: "p1".into(),
        model_source: ModelSource::Provider,
        model: Some("gpt-up".into()),
    };
    services
        .db()
        .with_conn(|conn| {
            terminal::save_proxy(conn, &fixed)?;
            terminal::save_direct(conn, &direct)?;
            engines::set(conn, Client::Pi, Some("m1"))?;
            settings::save(
                conn,
                &Settings {
                    host: "0.0.0.0".into(),
                    port: 18_888,
                    log_level: LogLevel::Debug,
                },
            )
        })
        .unwrap();
}

#[tokio::test]
async fn export_import_roundtrip() {
    let home = tempfile::tempdir().unwrap();
    let services = Services::for_test(home.path());
    seed_roundtrip(&services);
    let before = snapshot(&services);
    let providers_before = services.db().with_conn(providers::list).unwrap();
    let models_before = services.db().with_conn(models::list).unwrap();
    let standards_before = services.db().with_conn(standard_models::list).unwrap();
    let proxy_before = services.db().with_conn(terminal::list_proxy).unwrap();
    let direct_before = services.db().with_conn(terminal::list_direct).unwrap();
    let engines_before = services.db().with_conn(engines::list).unwrap();

    let exported = services.export_config(PASSPHRASE.to_owned()).await.unwrap();
    assert_eq!(exported.providers, 1);
    assert_eq!(exported.models, 1);
    assert!(exported.standard_models >= standards_before.len());
    let text = String::from_utf8(exported.contents).unwrap();
    assert!(!text.contains("sk-p1"));
    assert_eq!(exported.standard_models, standards_before.len());

    services
        .db()
        .with_tx(|tx| {
            tx.execute("DELETE FROM terminal_engines", [])?;
            tx.execute("DELETE FROM terminal_direct_profiles", [])?;
            tx.execute("DELETE FROM terminal_proxy_profiles", [])?;
            tx.execute("DELETE FROM models", [])?;
            tx.execute("DELETE FROM providers", [])?;
            tx.execute("DELETE FROM standard_models", [])?;
            tx.execute(
                "UPDATE settings SET host = '127.0.0.1', port = 11435, log_level = 'info' \
                 WHERE id = 1",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    assert!(before != snapshot(&services));

    let report = services
        .import_config(PASSPHRASE.to_owned(), text)
        .await
        .unwrap();
    assert_eq!(
        report,
        ImportReport {
            providers: 1,
            standard_models: standards_before.len(),
            models: 1,
            terminal_profiles: 2,
            skipped: Vec::new(),
        }
    );
    assert_eq!(
        services.db().with_conn(providers::list).unwrap(),
        providers_before
    );
    assert_eq!(
        services.db().with_conn(models::list).unwrap(),
        models_before
    );
    assert_eq!(
        services.db().with_conn(standard_models::list).unwrap(),
        standards_before
    );
    assert_eq!(
        services.db().with_conn(terminal::list_proxy).unwrap(),
        proxy_before
    );
    assert_eq!(
        services.db().with_conn(terminal::list_direct).unwrap(),
        direct_before
    );
    assert_eq!(
        services.db().with_conn(engines::list).unwrap(),
        engines_before
    );
    let settings = services.db().with_conn(settings::get).unwrap();
    assert_eq!(settings.log_level, LogLevel::Debug);
    assert_eq!(
        (settings.host.as_str(), settings.port),
        ("127.0.0.1", 11435)
    );
}

fn snapshot(services: &Services) -> Vec<(&'static str, Vec<Vec<String>>)> {
    table_snapshot(services)
}

#[tokio::test]
async fn wrong_passphrase_fails() {
    let text = encode(&empty_payload());
    let error = backup::decode(&text, &secret("another passphrase")).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Validation);
    assert!(error.message().contains("口令错误或文件已损坏"));
}

#[tokio::test]
async fn tampered_ciphertext_fails() {
    let text = encode(&empty_payload());
    let tampered = mutate(&text, |value| {
        let raw = BASE64.decode(value["payload"].as_str().unwrap()).unwrap();
        let mut sealed = raw;
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        value["payload"] = Value::String(BASE64.encode(sealed));
    });
    let error = decode(&tampered).unwrap_err();
    assert!(error.message().contains("口令错误或文件已损坏"));
}

#[tokio::test]
async fn tampered_header_fails() {
    let text = encode(&empty_payload());
    let tampered = mutate(&text, |value| {
        value["kdf"]["iterations"] = Value::from(ITERATIONS + 1);
    });
    let error = decode(&tampered).unwrap_err();
    assert!(error.message().contains("口令错误或文件已损坏"));
}

#[tokio::test]
async fn short_passphrase_rejected() {
    let error = backup::encode(&empty_payload(), &secret("short")).unwrap_err();
    assert!(error.message().contains("至少需要"));
    let text = encode(&empty_payload());
    let error = backup::decode(&text, &secret("1234567")).unwrap_err();
    assert!(error.message().contains("至少需要"));
}

#[tokio::test]
async fn iterations_out_of_range_rejected() {
    let text = encode(&empty_payload());
    for iterations in [9_999, 10_000_001] {
        let tampered = mutate(&text, |value| {
            value["kdf"]["iterations"] = Value::from(iterations);
        });
        let error = decode(&tampered).unwrap_err();
        assert!(error.message().contains("迭代次数"), "{}", error.message());
    }
}

#[tokio::test]
async fn provider_name_conflict_rolls_back() {
    let local_home = tempfile::tempdir().unwrap();
    let local = Services::for_test(local_home.path());
    let mut local_provider = provider("local", "dup", Protocol::OpenaiChat);
    local_provider.name = "Shared".into();
    save_provider(&local, local_provider, model("m1", "local-up"));
    let remote_home = tempfile::tempdir().unwrap();
    let remote = Services::for_test(remote_home.path());
    let mut remote_provider = provider("remote", "DUP", Protocol::OpenaiChat);
    remote_provider.name = "shared".into();
    save_provider(&remote, remote_provider, model("m2", "remote-up"));
    let payload = Payload::read(remote.db()).unwrap();
    let text = encode(&payload);
    let before = table_snapshot(&local);
    let error = local
        .import_config(PASSPHRASE.to_owned(), text)
        .await
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Conflict);
    assert!(error.message().contains("已被其他服务商使用"));
    assert_eq!(before, table_snapshot(&local));
}

#[tokio::test]
async fn merge_keeps_unlisted_providers() {
    let local_home = tempfile::tempdir().unwrap();
    let local = Services::for_test(local_home.path());
    save_provider(
        &local,
        provider("local", "local", Protocol::OpenaiChat),
        model("m1", "local-up"),
    );
    let remote_home = tempfile::tempdir().unwrap();
    let remote = Services::for_test(remote_home.path());
    save_provider(
        &remote,
        provider("remote", "remote", Protocol::OpenaiChat),
        model("m2", "remote-up"),
    );
    let text = encode(&Payload::read(remote.db()).unwrap());
    let report = local
        .import_config(PASSPHRASE.to_owned(), text)
        .await
        .unwrap();
    assert_eq!(report.providers, 1);
    let ids: Vec<String> = local
        .db()
        .with_conn(providers::list)
        .unwrap()
        .into_iter()
        .map(|provider| provider.id)
        .collect();
    assert!(ids.contains(&"local".to_owned()));
    assert!(ids.contains(&"remote".to_owned()));
}

#[tokio::test]
async fn terminal_entries_with_missing_refs_are_skipped() {
    let home = tempfile::tempdir().unwrap();
    let services = Services::for_test(home.path());
    let mut payload = empty_payload();
    payload.terminal_proxy_profiles.push(ProxyProfile {
        client: Client::Pi,
        protocol: Protocol::OpenaiChat,
        route_mode: RouteMode::Fixed,
        models: vec![ProfileModel {
            provider_id: "missing".into(),
            model: "gone".into(),
        }],
        default_model: Some("gone".into()),
    });
    payload.terminal_direct_profiles.push(DirectProfile {
        client: Client::Opencode,
        provider_id: "missing".into(),
        model_source: ModelSource::Native,
        model: None,
    });
    payload.terminal_engines.push(EngineSelection {
        client: Client::Pi,
        model_id: Some("missing".into()),
    });
    let report = services
        .import_config(PASSPHRASE.to_owned(), encode(&payload))
        .await
        .unwrap();
    assert_eq!(report.terminal_profiles, 0);
    assert_eq!(report.skipped.len(), 3);
    assert!(report.skipped.iter().any(|item| item.contains("pi")));
    assert!(report.skipped.iter().any(|item| item.contains("opencode")));
    assert!(
        services
            .db()
            .with_conn(terminal::list_proxy)
            .unwrap()
            .is_empty()
    );
    assert!(
        services
            .db()
            .with_conn(terminal::list_direct)
            .unwrap()
            .is_empty()
    );
    assert!(services.db().with_conn(engines::list).unwrap().is_empty());
}

#[tokio::test]
async fn import_does_not_touch_terminal_files() {
    let home = tempfile::tempdir().unwrap();
    let services = Services::for_test(home.path());
    let stored = save_provider(
        &services,
        provider("p1", "rs", Protocol::Responses),
        model("m1", "resp-up"),
    );
    let profile = TerminalProfileView {
        client: Client::Codex,
        models: vec![ProfileModel {
            provider_id: "p1".into(),
            model: stored.exposed_name.clone(),
        }],
        default_model: stored.exposed_name.clone(),
        protocol: Some(Protocol::Responses),
        route_mode: RouteMode::Fixed,
        active_engine: None,
    };
    let applied = services.apply_terminal_config(profile).await.unwrap();
    let path = std::path::PathBuf::from(&applied.config_path);
    let contents = std::fs::read(&path).unwrap();
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    let text = encode(&Payload::read(services.db()).unwrap());
    services
        .import_config(PASSPHRASE.to_owned(), text)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), contents);
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        modified
    );
}
