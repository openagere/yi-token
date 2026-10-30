//! Tauri commands: thin wrappers over `yi_core::services::Services`.
//!
//! Each command delegates to an internal `*_impl` function taking the services
//! handle, which is what the tests call (no window needed).

use crate::state::AppState;
use serde::Serialize;
use std::sync::Arc;
use tauri::State;
use yi_core::backup::{FILE_EXTENSION, FILE_FORMAT_NAME};
use yi_core::domain::{
    BlobView, Client, DirectProfile, LiveSnapshot, LogLevel, SessionChanges, SessionDetail,
    SessionListInput, SessionPage, StepContext, TrajectoryInput, TrajectoryPage,
    TrajectorySettings, TrajectoryStatus, UsageGroupBy,
};
use yi_core::services::{
    ActiveEngineView, CatalogInfo, LogsView, ModelInput, MonitorView, ProviderInput, ProviderView,
    ProxyAction, ProxyRuntimeView, Services, SettingsView, StandardModelView, TerminalApplyResult,
    TerminalPreview, TerminalProfileView, TerminalProfilesView, UsageQueryInput, UsageReportView,
};
use yi_core::{CoreError, CoreResult};

type Svc = Arc<Services>;

#[derive(Debug, Clone, Serialize)]
pub struct ProviderConfigTransferResult {
    pub path: String,
    pub providers: usize,
    pub models: usize,
    pub standard_models: usize,
    pub terminal_config_warnings: Vec<String>,
}

async fn list_providers_impl(s: &Svc) -> CoreResult<Vec<ProviderView>> {
    s.list_providers().await
}

async fn save_provider_impl(
    s: &Svc,
    provider: ProviderInput,
    models: Vec<ModelInput>,
) -> CoreResult<ProviderView> {
    s.save_provider(provider, models).await
}

async fn delete_provider_impl(s: &Svc, id: String) -> CoreResult<()> {
    s.delete_provider(id).await
}

async fn test_provider_impl(
    s: &Svc,
    provider: ProviderInput,
    _models: Vec<ModelInput>,
    model: String,
) -> CoreResult<String> {
    let result = s.test_provider(provider, model).await?;
    if result.ok {
        Ok(result.message)
    } else {
        Err(CoreError::Validation(result.message))
    }
}

async fn get_proxy_runtime_impl(s: &Svc) -> CoreResult<ProxyRuntimeView> {
    s.get_proxy_runtime().await
}

async fn get_proxy_monitor_impl(s: &Svc, minutes: Option<u32>) -> CoreResult<MonitorView> {
    Ok(s.get_proxy_monitor(minutes))
}

async fn control_proxy_impl(s: &Svc, action: ProxyAction) -> CoreResult<()> {
    s.control_proxy(action).await
}

async fn list_standard_models_impl(s: &Svc) -> CoreResult<Vec<StandardModelView>> {
    s.list_standard_models().await
}

async fn get_model_catalog_info_impl(s: &Svc) -> CoreResult<CatalogInfo> {
    s.get_model_catalog_info().await
}

async fn save_standard_model_impl(
    s: &Svc,
    model: StandardModelView,
) -> CoreResult<StandardModelView> {
    s.save_standard_model(model).await
}

async fn delete_standard_model_impl(s: &Svc, id: String) -> CoreResult<()> {
    s.delete_standard_model(id).await
}

async fn get_settings_impl(s: &Svc) -> CoreResult<SettingsView> {
    s.get_settings().await
}

async fn save_settings_impl(
    s: &Svc,
    host: String,
    port: u16,
    log_level: LogLevel,
) -> CoreResult<()> {
    s.save_settings(host, port, log_level).await
}

async fn get_logs_impl(s: &Svc) -> CoreResult<LogsView> {
    s.get_logs().await
}

async fn get_usage_impl(
    s: &Svc,
    days: Option<u32>,
    model: Option<String>,
    start_date: Option<String>,
    end_date: Option<String>,
    client: Option<Client>,
    group_by: Option<UsageGroupBy>,
) -> CoreResult<UsageReportView> {
    s.get_usage(UsageQueryInput {
        days,
        model,
        start_date,
        end_date,
        client,
        group_by,
    })
    .await
}

async fn get_terminal_profiles_impl(s: &Svc) -> CoreResult<TerminalProfilesView> {
    s.get_terminal_profiles().await
}

async fn preview_terminal_config_impl(
    s: &Svc,
    profile: TerminalProfileView,
) -> CoreResult<TerminalPreview> {
    s.preview_terminal_config(profile).await
}

async fn apply_terminal_config_impl(
    s: &Svc,
    profile: TerminalProfileView,
) -> CoreResult<TerminalApplyResult> {
    s.apply_terminal_config(profile).await
}

async fn preview_terminal_direct_config_impl(
    s: &Svc,
    profile: DirectProfile,
) -> CoreResult<TerminalPreview> {
    s.preview_terminal_direct_config(profile).await
}

async fn apply_terminal_direct_config_impl(
    s: &Svc,
    profile: DirectProfile,
) -> CoreResult<TerminalApplyResult> {
    s.apply_terminal_direct_config(profile).await
}

async fn disconnect_terminal_impl(s: &Svc, client: Client) -> CoreResult<TerminalApplyResult> {
    s.disconnect_terminal(client).await
}

async fn set_active_engine_impl(
    s: &Svc,
    client: Client,
    model_id: Option<String>,
) -> CoreResult<()> {
    s.set_active_engine(client, model_id).await
}

async fn get_active_engines_impl(s: &Svc) -> CoreResult<Vec<ActiveEngineView>> {
    s.get_active_engines().await
}

async fn discover_provider_models_impl(
    s: &Svc,
    provider: ProviderInput,
) -> CoreResult<Vec<yi_core::domain::DiscoveredModel>> {
    s.discover_provider_models(provider).await
}

async fn list_sessions_impl(s: &Svc, input: SessionListInput) -> CoreResult<SessionPage> {
    s.list_sessions(input).await
}

async fn get_session_impl(s: &Svc, id: String) -> CoreResult<SessionDetail> {
    s.get_session(id).await
}

async fn get_trajectory_impl(s: &Svc, input: TrajectoryInput) -> CoreResult<TrajectoryPage> {
    s.get_trajectory(input).await
}

async fn get_step_context_impl(s: &Svc, step_id: String) -> CoreResult<StepContext> {
    s.get_step_context(step_id).await
}

async fn get_blob_impl(s: &Svc, hash: String) -> CoreResult<BlobView> {
    s.get_blob(hash).await
}

async fn get_session_changes_impl(s: &Svc, since_seq: u64) -> CoreResult<SessionChanges> {
    s.session_changes(since_seq).await
}

async fn delete_session_impl(s: &Svc, id: String) -> CoreResult<()> {
    s.delete_session(id).await
}

async fn clear_sessions_impl(s: &Svc, client: Option<Client>) -> CoreResult<u32> {
    s.clear_sessions(client).await
}

async fn get_trajectory_status_impl(s: &Svc) -> CoreResult<TrajectoryStatus> {
    s.trajectory_status().await
}

async fn save_trajectory_settings_impl(s: &Svc, settings: TrajectorySettings) -> CoreResult<()> {
    s.save_trajectory_settings(settings).await
}

async fn rebuild_trajectory_db_impl(s: &Svc) -> CoreResult<()> {
    s.rebuild_trajectory_db().await
}

fn get_live_requests_impl(s: &Svc, since_seq: u64) -> LiveSnapshot {
    s.live_requests(since_seq)
}

async fn export_provider_config_impl(
    s: &Svc,
    passphrase: String,
    title: String,
) -> CoreResult<Option<ProviderConfigTransferResult>> {
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title(title)
        .set_file_name(format!("yi-token-backup.{FILE_EXTENSION}"))
        .add_filter(FILE_FORMAT_NAME, &[FILE_EXTENSION])
        .save_file()
        .await
    else {
        return Ok(None);
    };
    let exported = s.export_config(passphrase).await?;
    let path = file.path().to_path_buf();
    tokio::fs::write(&path, &exported.contents)
        .await
        .map_err(|error| CoreError::Io(format!("写入备份文件失败：{error}")))?;
    Ok(Some(ProviderConfigTransferResult {
        path: path.display().to_string(),
        providers: exported.providers,
        models: exported.models,
        standard_models: exported.standard_models,
        terminal_config_warnings: Vec::new(),
    }))
}

async fn import_provider_config_impl(
    s: &Svc,
    passphrase: String,
    contents: String,
    filename: String,
) -> CoreResult<ProviderConfigTransferResult> {
    let report = s.import_config(passphrase, contents).await?;
    Ok(ProviderConfigTransferResult {
        path: filename,
        providers: report.providers,
        models: report.models,
        standard_models: report.standard_models,
        terminal_config_warnings: report.skipped,
    })
}

#[tauri::command]
pub async fn export_provider_config(
    state: State<'_, AppState>,
    passphrase: String,
    title: String,
) -> CoreResult<Option<ProviderConfigTransferResult>> {
    export_provider_config_impl(&state.0, passphrase, title).await
}

#[tauri::command]
pub async fn import_provider_config(
    state: State<'_, AppState>,
    passphrase: String,
    contents: String,
    filename: String,
) -> CoreResult<ProviderConfigTransferResult> {
    import_provider_config_impl(&state.0, passphrase, contents, filename).await
}

#[tauri::command]
pub async fn list_providers(state: State<'_, AppState>) -> CoreResult<Vec<ProviderView>> {
    list_providers_impl(&state.0).await
}

#[tauri::command]
pub async fn save_provider(
    state: State<'_, AppState>,
    provider: ProviderInput,
    models: Vec<ModelInput>,
) -> CoreResult<ProviderView> {
    save_provider_impl(&state.0, provider, models).await
}

#[tauri::command]
pub async fn delete_provider(state: State<'_, AppState>, id: String) -> CoreResult<()> {
    delete_provider_impl(&state.0, id).await
}

#[tauri::command]
pub async fn get_proxy_runtime(state: State<'_, AppState>) -> CoreResult<ProxyRuntimeView> {
    get_proxy_runtime_impl(&state.0).await
}

#[tauri::command]
pub async fn get_proxy_monitor(
    state: State<'_, AppState>,
    minutes: Option<u32>,
) -> CoreResult<MonitorView> {
    get_proxy_monitor_impl(&state.0, minutes).await
}

#[tauri::command]
pub async fn control_proxy(state: State<'_, AppState>, action: ProxyAction) -> CoreResult<()> {
    control_proxy_impl(&state.0, action).await
}

#[tauri::command]
pub async fn test_provider(
    state: State<'_, AppState>,
    provider: ProviderInput,
    models: Vec<ModelInput>,
    model: String,
) -> CoreResult<String> {
    test_provider_impl(&state.0, provider, models, model).await
}

#[tauri::command]
pub async fn list_standard_models(
    state: State<'_, AppState>,
) -> CoreResult<Vec<StandardModelView>> {
    list_standard_models_impl(&state.0).await
}

#[tauri::command]
pub async fn get_model_catalog_info(state: State<'_, AppState>) -> CoreResult<CatalogInfo> {
    get_model_catalog_info_impl(&state.0).await
}

#[tauri::command]
pub async fn save_standard_model(
    state: State<'_, AppState>,
    model: StandardModelView,
) -> CoreResult<StandardModelView> {
    save_standard_model_impl(&state.0, model).await
}

#[tauri::command]
pub async fn delete_standard_model(state: State<'_, AppState>, id: String) -> CoreResult<()> {
    delete_standard_model_impl(&state.0, id).await
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> CoreResult<SettingsView> {
    get_settings_impl(&state.0).await
}

#[tauri::command]
pub async fn save_settings(
    state: State<'_, AppState>,
    host: String,
    port: u16,
    log_level: LogLevel,
) -> CoreResult<()> {
    save_settings_impl(&state.0, host, port, log_level).await
}

#[tauri::command]
pub async fn get_logs(state: State<'_, AppState>) -> CoreResult<LogsView> {
    get_logs_impl(&state.0).await
}

#[tauri::command]
pub async fn get_usage(
    state: State<'_, AppState>,
    days: Option<u32>,
    model: Option<String>,
    start_date: Option<String>,
    end_date: Option<String>,
    client: Option<Client>,
    group_by: Option<UsageGroupBy>,
) -> CoreResult<UsageReportView> {
    get_usage_impl(&state.0, days, model, start_date, end_date, client, group_by).await
}

#[tauri::command]
pub async fn get_terminal_profiles(state: State<'_, AppState>) -> CoreResult<TerminalProfilesView> {
    get_terminal_profiles_impl(&state.0).await
}

#[tauri::command]
pub async fn preview_terminal_config(
    state: State<'_, AppState>,
    profile: TerminalProfileView,
) -> CoreResult<TerminalPreview> {
    preview_terminal_config_impl(&state.0, profile).await
}

#[tauri::command]
pub async fn apply_terminal_config(
    state: State<'_, AppState>,
    profile: TerminalProfileView,
) -> CoreResult<TerminalApplyResult> {
    apply_terminal_config_impl(&state.0, profile).await
}

#[tauri::command]
pub async fn preview_terminal_direct_config(
    state: State<'_, AppState>,
    profile: DirectProfile,
) -> CoreResult<TerminalPreview> {
    preview_terminal_direct_config_impl(&state.0, profile).await
}

#[tauri::command]
pub async fn disconnect_terminal(
    state: State<'_, AppState>,
    client: Client,
) -> CoreResult<TerminalApplyResult> {
    disconnect_terminal_impl(&state.0, client).await
}

#[tauri::command]
pub async fn apply_terminal_direct_config(
    state: State<'_, AppState>,
    profile: DirectProfile,
) -> CoreResult<TerminalApplyResult> {
    apply_terminal_direct_config_impl(&state.0, profile).await
}

#[tauri::command]
pub async fn set_active_engine(
    state: State<'_, AppState>,
    client: Client,
    model_id: Option<String>,
) -> CoreResult<()> {
    set_active_engine_impl(&state.0, client, model_id).await
}

#[tauri::command]
pub async fn get_active_engines(state: State<'_, AppState>) -> CoreResult<Vec<ActiveEngineView>> {
    get_active_engines_impl(&state.0).await
}

#[tauri::command]
pub async fn discover_provider_models(
    state: State<'_, AppState>,
    provider: ProviderInput,
) -> CoreResult<Vec<yi_core::domain::DiscoveredModel>> {
    discover_provider_models_impl(&state.0, provider).await
}

#[tauri::command]
pub async fn list_sessions(
    state: State<'_, AppState>,
    input: SessionListInput,
) -> CoreResult<SessionPage> {
    list_sessions_impl(&state.0, input).await
}

#[tauri::command]
pub async fn get_session(state: State<'_, AppState>, id: String) -> CoreResult<SessionDetail> {
    get_session_impl(&state.0, id).await
}

#[tauri::command]
pub async fn get_trajectory(
    state: State<'_, AppState>,
    input: TrajectoryInput,
) -> CoreResult<TrajectoryPage> {
    get_trajectory_impl(&state.0, input).await
}

#[tauri::command]
pub async fn get_step_context(
    state: State<'_, AppState>,
    step_id: String,
) -> CoreResult<StepContext> {
    get_step_context_impl(&state.0, step_id).await
}

#[tauri::command]
pub async fn get_blob(state: State<'_, AppState>, hash: String) -> CoreResult<BlobView> {
    get_blob_impl(&state.0, hash).await
}

#[tauri::command]
pub async fn get_session_changes(
    state: State<'_, AppState>,
    since_seq: u64,
) -> CoreResult<SessionChanges> {
    get_session_changes_impl(&state.0, since_seq).await
}

#[tauri::command]
pub async fn delete_session(state: State<'_, AppState>, id: String) -> CoreResult<()> {
    delete_session_impl(&state.0, id).await
}

#[tauri::command]
pub async fn clear_sessions(state: State<'_, AppState>, client: Option<Client>) -> CoreResult<u32> {
    clear_sessions_impl(&state.0, client).await
}

#[tauri::command]
pub async fn get_trajectory_status(state: State<'_, AppState>) -> CoreResult<TrajectoryStatus> {
    get_trajectory_status_impl(&state.0).await
}

#[tauri::command]
pub async fn save_trajectory_settings(
    state: State<'_, AppState>,
    settings: TrajectorySettings,
) -> CoreResult<()> {
    save_trajectory_settings_impl(&state.0, settings).await
}

#[tauri::command]
pub async fn rebuild_trajectory_db(state: State<'_, AppState>) -> CoreResult<()> {
    rebuild_trajectory_db_impl(&state.0).await
}

#[tauri::command]
pub fn get_live_requests(state: State<'_, AppState>, since_seq: u64) -> LiveSnapshot {
    get_live_requests_impl(&state.0, since_seq)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> ProviderInput {
        serde_json::from_value(serde_json::json!({
            "provider_type": "openai_chat",
            "name": "P",
            "base_url": "https://example.com/v1",
            "api_key": "sk-test",
            "enabled": true
        }))
        .expect("provider input")
    }

    #[tokio::test]
    async fn commands_delegate_to_services() {
        let home = tempfile::tempdir().unwrap();
        let s = Services::for_test(home.path());
        assert!(list_providers_impl(&s).await.unwrap().is_empty());
        let saved = save_provider_impl(&s, input(), vec![]).await.unwrap();
        assert!(!saved.provider.id.is_empty());
        assert_eq!(list_providers_impl(&s).await.unwrap().len(), 1);
        delete_provider_impl(&s, saved.provider.id).await.unwrap();
        assert!(list_providers_impl(&s).await.unwrap().is_empty());

        assert!(!get_settings_impl(&s).await.unwrap().host.is_empty());
        assert!(get_logs_impl(&s).await.is_ok());
        assert!(get_usage_impl(&s, Some(7), None, None, None, None, None).await.is_ok());
        assert!(!list_standard_models_impl(&s).await.unwrap().is_empty());
        assert!(get_model_catalog_info_impl(&s).await.is_ok());
    }

    #[tokio::test]
    async fn save_provider_passes_models() {
        let home = tempfile::tempdir().unwrap();
        let s = Services::for_test(home.path());
        let model: ModelInput =
            serde_json::from_value(serde_json::json!({"name": "gpt-x"})).expect("model input");
        let saved = save_provider_impl(&s, input(), vec![model]).await.unwrap();
        assert_eq!(saved.models.len(), 1);
        assert_eq!(saved.models[0].name, "gpt-x");
        let listed = list_providers_impl(&s).await.unwrap();
        assert_eq!(listed[0].models.len(), 1);
    }

    #[tokio::test]
    async fn errors_are_core_errors() {
        let home = tempfile::tempdir().unwrap();
        let s = Services::for_test(home.path());
        let err = save_settings_impl(&s, "nope".into(), 1, LogLevel::Info)
            .await
            .unwrap_err();
        let v = serde_json::to_value(&err).unwrap();
        assert!(v.get("code").is_some() && v.get("message").is_some());
    }

    fn all_sessions() -> SessionListInput {
        SessionListInput {
            client: None,
            query: None,
            model: None,
            start_ms: None,
            end_ms: None,
            cursor: None,
            limit: None,
        }
    }

    #[tokio::test]
    async fn session_commands_delegate_to_services() {
        let home = tempfile::tempdir().unwrap();
        let s = Services::for_test(home.path());
        let page = list_sessions_impl(&s, all_sessions()).await.unwrap();
        assert!(page.items.is_empty());
        assert!(page.next_cursor.is_none());
    }

    #[tokio::test]
    async fn get_trajectory_reports_missing_session() {
        let home = tempfile::tempdir().unwrap();
        let s = Services::for_test(home.path());
        let input = TrajectoryInput {
            session_id: "missing".into(),
            thread_id: None,
            before_turn: None,
            limit_turns: None,
        };
        let err = get_trajectory_impl(&s, input).await.unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)), "{err:?}");
    }
}
