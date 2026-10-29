use std::fs;
use std::sync::Arc;
use tempfile::TempDir;
use yi_core::domain::{
    AUTO_MODEL, Capabilities, Client, DirectProfile, Modality, ModelSource, ProfileModel, Protocol,
    RouteMode, Thinking,
};
use yi_core::error::ErrorCode;
use yi_core::services::{
    ModelInput, ProviderInput, ProviderView, Services, TerminalAccessMode, TerminalProfileView,
};
use yi_core::storage::terminal;

async fn fixture(upstreams: &[&str], enabled: bool) -> (TempDir, Arc<Services>, ProviderView) {
    let dir = tempfile::tempdir().unwrap();
    let svc = Services::for_test(dir.path());
    let provider = ProviderInput {
        id: String::new(),
        provider_type: Protocol::Responses,
        name: "OpenAI".into(),
        base_url: "https://example.com/v1".into(),
        api_key: "sk-test".into(),
        enabled,
        thinking: Thinking::Medium,
        extra: serde_json::json!({}),
        is_default: false,
    };
    let models = upstreams
        .iter()
        .map(|name| ModelInput {
            id: String::new(),
            provider_id: String::new(),
            route_id: String::new(),
            name: (*name).into(),
            standard_model_id: None,
            protocol: None,
            capabilities: None,
            non_standard: false,
        })
        .collect();
    let saved = svc.save_provider(provider, models).await.unwrap();
    (dir, svc, saved)
}

fn proxy_profile(provider: &ProviderView, index: usize) -> TerminalProfileView {
    let model = &provider.models[index];
    TerminalProfileView {
        client: Client::Codex,
        models: vec![ProfileModel {
            provider_id: provider.provider.id.clone(),
            model: model.route_id.clone(),
        }],
        default_model: model.route_id.clone(),
        protocol: None,
        route_mode: RouteMode::Fixed,
        active_engine: None,
    }
}

fn codex_config(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join(".codex").join("config.toml")
}

#[tokio::test]
async fn apply_proxy_writes_file_and_saves_profile() {
    let (dir, svc, provider) = fixture(&["gpt-5"], true).await;
    let path = codex_config(&dir);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "[user]\n# keep me\nvalue = 1\n").unwrap();

    let profile = proxy_profile(&provider, 0);
    let result = svc.apply_terminal_config(profile).await.unwrap();

    assert_eq!(result.config_path, path.display().to_string());
    assert_eq!(result.endpoint, "http://127.0.0.1:11435/clients/codex/v1");
    assert_eq!(result.model_count, 1);
    assert_eq!(result.backup_paths.len(), 1);

    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("# keep me"), "{content}");
    assert!(content.contains("[user]"), "{content}");
    assert!(content.contains("[model_providers.yitoken]"));
    assert!(content.contains("http://127.0.0.1:11435/clients/codex/v1"));
    assert!(content.contains(&provider.models[0].route_id));

    let stored = svc
        .db()
        .with_conn(|conn| terminal::get_proxy(conn, "codex"))
        .unwrap()
        .expect("saved proxy profile");
    assert_eq!(stored.client, Client::Codex);
    assert_eq!(stored.protocol, Protocol::Responses);
    assert_eq!(stored.route_mode, RouteMode::Fixed);
    assert_eq!(stored.models.len(), 1);
    assert_eq!(
        stored.default_model.as_deref(),
        Some(provider.models[0].route_id.as_str())
    );
    assert!(
        svc.routes()
            .load()
            .proxy_profiles
            .contains_key(&Client::Codex)
    );

    let status = svc
        .get_terminal_profiles()
        .await
        .unwrap()
        .into_iter()
        .find(|item| item.client == Client::Codex)
        .unwrap();
    assert!(status.active);
    assert_eq!(status.active_mode, TerminalAccessMode::Proxy);
    assert!(status.exists);
    assert_eq!(status.profile.models, stored.models);
    assert_eq!(status.profile.route_mode, RouteMode::Fixed);
    assert_eq!(status.direct_profile, None);
    assert_eq!(status.config_error, None);
}

#[tokio::test]
async fn preview_does_not_touch_disk() {
    let (dir, svc, provider) = fixture(&["gpt-5"], true).await;
    let path = codex_config(&dir);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let original = "# keep me\nmodel = \"user\"\n";
    fs::write(&path, original).unwrap();

    let preview = svc
        .preview_terminal_config(proxy_profile(&provider, 0))
        .await
        .unwrap();

    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert!(preview.files.iter().any(|file| {
        file.path == path.display().to_string() && file.content.contains("yitoken")
    }));
    assert!(
        svc.db()
            .with_conn(|conn| terminal::get_proxy(conn, "codex"))
            .unwrap()
            .is_none()
    );
    assert!(svc.routes().load().proxy_profiles.is_empty());
    let backups: Vec<_> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains(".bak-"))
        .collect();
    assert!(backups.is_empty());
}

#[tokio::test]
async fn apply_failure_leaves_db_unchanged() {
    let (dir, svc, provider) = fixture(&["gpt-5", "gpt-5-mini"], true).await;
    let path = codex_config(&dir);
    svc.apply_terminal_config(proxy_profile(&provider, 0))
        .await
        .unwrap();
    let before = fs::read_to_string(&path).unwrap();

    svc.db()
        .with_conn(|conn| {
            conn.execute_batch(
                "CREATE TRIGGER fail_terminal_proxy \
                 BEFORE UPDATE ON terminal_proxy_profiles \
                 BEGIN SELECT RAISE(FAIL, 'boom'); END;",
            )?;
            Ok(())
        })
        .unwrap();

    let error = svc
        .apply_terminal_config(proxy_profile(&provider, 1))
        .await
        .unwrap_err();
    assert!(error.message().contains("boom"));

    assert_eq!(fs::read_to_string(&path).unwrap(), before);
    let stored = svc
        .db()
        .with_conn(|conn| terminal::get_proxy(conn, "codex"))
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.default_model.as_deref(),
        Some(provider.models[0].route_id.as_str())
    );
}

#[tokio::test]
async fn apply_direct_for_disabled_provider_is_validation() {
    let (dir, svc, provider) = fixture(&["gpt-5"], false).await;
    let profile = DirectProfile {
        client: Client::Codex,
        provider_id: provider.provider.id,
        model_source: ModelSource::Native,
        model: None,
    };

    let error = svc.apply_terminal_direct_config(profile).await.unwrap_err();

    assert_eq!(error.code(), ErrorCode::Validation);
    assert!(error.message().contains("未启用"));
    assert!(!codex_config(&dir).exists());
    assert!(
        svc.db()
            .with_conn(|conn| terminal::get_direct(conn, "codex"))
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn profiles_report_defaults_for_unconfigured_clients() {
    let (dir, svc, _) = fixture(&["gpt-5"], true).await;
    let statuses = svc.get_terminal_profiles().await.unwrap();

    assert_eq!(statuses.len(), Client::ALL.len());
    let codex = statuses
        .iter()
        .find(|item| item.client == Client::Codex)
        .unwrap();
    assert!(!codex.active);
    assert_eq!(codex.active_mode, TerminalAccessMode::None);
    assert!(codex.profile.models.is_empty());
    assert_eq!(codex.profile.route_mode, RouteMode::Fixed);
    assert_eq!(codex.profile.protocol, None);
    assert_eq!(codex.config_path, codex_config(&dir).display().to_string());
    assert_eq!(codex.direct_profile, None);
    assert_eq!(codex.config_error, None);
}

#[tokio::test]
async fn switching_proxy_engine_keeps_the_terminal_connected() {
    let (dir, svc, provider) = fixture(&["gpt-5", "gpt-4"], true).await;
    let models = provider
        .models
        .iter()
        .enumerate()
        .map(|(index, model)| {
            let capabilities = Capabilities {
                context_window: Some(if index == 0 { 128_000 } else { 1_000_000 }),
                input_modalities: Some(vec![Modality::Text, Modality::Image]),
                ..Capabilities::default()
            };
            ModelInput {
                id: model.id.clone(),
                provider_id: provider.provider.id.clone(),
                route_id: model.route_id.clone(),
                name: model.name.clone(),
                standard_model_id: None,
                protocol: None,
                capabilities: Some(capabilities),
                non_standard: false,
            }
        })
        .collect();
    let provider = svc
        .save_provider(provider.provider.clone(), models)
        .await
        .unwrap();
    let path = codex_config(&dir);
    let catalog = dir.path().join(".codex").join("yi-token-models.json");
    let engine = |index: usize| provider.models[index].id.clone();
    svc.set_active_engine(Client::Codex, Some(engine(0)))
        .await
        .unwrap();
    let auto = TerminalProfileView {
        client: Client::Codex,
        models: Vec::new(),
        default_model: String::new(),
        protocol: None,
        route_mode: RouteMode::Auto,
        active_engine: None,
    };
    svc.apply_terminal_config(auto).await.unwrap();
    let status = |svc: Arc<Services>| async move {
        svc.get_terminal_profiles()
            .await
            .unwrap()
            .into_iter()
            .find(|item| item.client == Client::Codex)
            .unwrap()
    };
    assert!(status(svc.clone()).await.active);

    // Switching the engine only changes routing. The context written at connect stays.
    let before = fs::read_to_string(&path).unwrap();
    let written = fs::read_to_string(&catalog).unwrap();
    assert!(written.contains("128000"), "{written}");
    assert!(!written.contains("1000000"), "{written}");
    svc.set_active_engine(Client::Codex, Some(engine(1)))
        .await
        .unwrap();
    let after = status(svc.clone()).await;
    assert!(after.active, "switching the engine must not disconnect");
    assert_eq!(after.active_mode, TerminalAccessMode::Proxy);
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
    assert_eq!(fs::read_to_string(&catalog).unwrap(), written);
}

#[tokio::test]
async fn disconnect_keeps_the_proxy_selection_and_is_idempotent() {
    let (dir, svc, provider) = fixture(&["gpt-5", "gpt-4"], true).await;
    let path = codex_config(&dir);
    let engine = provider.models[0].id.clone();
    svc.set_active_engine(Client::Codex, Some(engine.clone()))
        .await
        .unwrap();
    let auto = TerminalProfileView {
        client: Client::Codex,
        models: Vec::new(),
        default_model: AUTO_MODEL.into(),
        protocol: None,
        route_mode: RouteMode::Auto,
        active_engine: None,
    };
    svc.apply_terminal_config(auto).await.unwrap();
    assert!(fs::read_to_string(&path).unwrap().contains("yitoken"));

    let disconnected = svc.disconnect_terminal(Client::Codex).await.unwrap();
    assert!(!disconnected.backup_paths.is_empty());
    let content = fs::read_to_string(&path).unwrap();
    assert!(!content.contains("yitoken"), "{content}");
    let status = svc
        .get_terminal_profiles()
        .await
        .unwrap()
        .into_iter()
        .find(|item| item.client == Client::Codex)
        .unwrap();
    assert!(!status.active);
    assert_eq!(status.active_mode, TerminalAccessMode::None);
    let engines = svc.get_active_engines().await.unwrap();
    assert_eq!(engines[0].model_id, engine);
    assert!(
        svc.db()
            .with_conn(|conn| terminal::get_proxy(conn, "codex"))
            .unwrap()
            .is_some(),
        "the proxy profile stays so reconnect can restore it"
    );

    let again = svc.disconnect_terminal(Client::Codex).await.unwrap();
    assert!(again.backup_paths.is_empty());
    assert_eq!(fs::read_to_string(&path).unwrap(), content);
}

#[tokio::test]
async fn disconnect_removes_a_direct_provider_connection() {
    let (dir, svc, provider) = fixture(&["gpt-5"], true).await;
    let path = codex_config(&dir);
    svc.apply_terminal_direct_config(DirectProfile {
        client: Client::Codex,
        provider_id: provider.provider.id.clone(),
        model_source: ModelSource::Provider,
        model: None,
    })
    .await
    .unwrap();
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("yitoken_direct")
    );

    svc.disconnect_terminal(Client::Codex).await.unwrap();
    let content = fs::read_to_string(&path).unwrap();
    assert!(!content.contains("yitoken"), "{content}");
    let status = svc
        .get_terminal_profiles()
        .await
        .unwrap()
        .into_iter()
        .find(|item| item.client == Client::Codex)
        .unwrap();
    assert!(!status.active);
    assert_eq!(status.active_mode, TerminalAccessMode::None);
    assert!(
        svc.db()
            .with_conn(|conn| terminal::get_direct(conn, "codex"))
            .unwrap()
            .is_some()
    );
}
