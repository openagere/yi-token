//! Tauri commands: thin wrappers over `yi_core::services::Services`.
//!
//! Each command delegates to an internal `*_impl` function taking the services
//! handle, which is what the tests call (no window needed).

use crate::state::AppState;
use serde::Serialize;
use std::sync::Arc;
use tauri::State;
use yi_core::backup::{FILE_EXTENSION, FILE_FORMAT_NAME};
use yi_core::domain::{Client, DirectProfile, LogLevel};
use yi_core::services::{
    ActiveEngineView, CatalogInfo, LogsView, ModelInput, MonitorView, ProviderInput, ProviderView,
    ProxyRuntimeView, Services, SettingsView, StandardModelView, TerminalApplyResult,
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

fn get_proxy_status_impl(s: &Svc) -> bool {
    s.get_proxy_status()
}

async fn get_proxy_runtime_impl(s: &Svc) -> CoreResult<ProxyRuntimeView> {
    s.get_proxy_runtime().await
}

async fn get_proxy_monitor_impl(s: &Svc, minutes: Option<u32>) -> CoreResult<MonitorView> {
    Ok(s.get_proxy_monitor(minutes))
}

async fn control_proxy_impl(s: &Svc, action: String) -> CoreResult<()> {
    s.control_proxy(&action).await
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
) -> CoreResult<UsageReportView> {
    s.get_usage(UsageQueryInput {
        days,
        model,
        start_date,
        end_date,
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

async fn list_provider_models_impl(s: &Svc, id: String) -> CoreResult<Vec<String>> {
    s.list_provider_models(id).await
}

async fn select_terminal_model_impl(
    s: &Svc,
    client: Client,
    provider_id: String,
    model: String,
) -> CoreResult<ActiveEngineView> {
    s.select_terminal_model(client, provider_id, model).await
}

async fn discover_provider_models_impl(
    s: &Svc,
    provider: ProviderInput,
) -> CoreResult<Vec<yi_core::domain::DiscoveredModel>> {
    s.discover_provider_models(provider).await
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
pub fn get_proxy_status(state: State<'_, AppState>) -> bool {
    get_proxy_status_impl(&state.0)
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
pub async fn control_proxy(state: State<'_, AppState>, action: String) -> CoreResult<()> {
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
) -> CoreResult<UsageReportView> {
    get_usage_impl(&state.0, days, model, start_date, end_date).await
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
pub async fn list_provider_models(
    state: State<'_, AppState>,
    id: String,
) -> CoreResult<Vec<String>> {
    list_provider_models_impl(&state.0, id).await
}

#[tauri::command]
pub async fn select_terminal_model(
    state: State<'_, AppState>,
    client: Client,
    provider_id: String,
    model: String,
) -> CoreResult<ActiveEngineView> {
    select_terminal_model_impl(&state.0, client, provider_id, model).await
}

#[tauri::command]
pub async fn discover_provider_models(
    state: State<'_, AppState>,
    provider: ProviderInput,
) -> CoreResult<Vec<yi_core::domain::DiscoveredModel>> {
    discover_provider_models_impl(&state.0, provider).await
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
        assert!(get_usage_impl(&s, Some(7), None, None, None).await.is_ok());
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
}
