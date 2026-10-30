use crate::domain::{
    Client, Company, DirectProfile, LogLevel, Model, Provider, ProxyProfile, StandardModel,
};
use crate::error::{CoreError, CoreResult};
use crate::storage::{self, Db};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderBackup {
    pub provider: Provider,
    pub models: Vec<Model>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineSelection {
    pub client: Client,
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupSettings {
    pub log_level: LogLevel,
}

impl Default for BackupSettings {
    fn default() -> Self {
        Self {
            log_level: LogLevel::Info,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Payload {
    pub exported_at: i64,
    pub app_version: String,
    #[serde(default)]
    pub standard_models: Vec<StandardModel>,
    #[serde(default)]
    pub providers: Vec<ProviderBackup>,
    #[serde(default)]
    pub terminal_proxy_profiles: Vec<ProxyProfile>,
    #[serde(default)]
    pub terminal_direct_profiles: Vec<DirectProfile>,
    #[serde(default)]
    pub terminal_engines: Vec<EngineSelection>,
    #[serde(default)]
    pub settings: BackupSettings,
}

impl Payload {
    pub fn read(db: &Db) -> CoreResult<Self> {
        db.with_conn(Self::from_db)
    }

    pub fn from_db(conn: &Connection) -> CoreResult<Self> {
        let providers = storage::providers::list(conn)?
            .into_iter()
            .map(|provider| {
                Ok(ProviderBackup {
                    models: storage::models::list_by_provider(conn, &provider.id)?,
                    provider,
                })
            })
            .collect::<CoreResult<Vec<_>>>()?;
        Ok(Self {
            exported_at: crate::clock::now_secs(),
            app_version: env!("CARGO_PKG_VERSION").into(),
            standard_models: storage::standard_models::list(conn)?,
            providers,
            terminal_proxy_profiles: storage::terminal::list_proxy(conn)?,
            terminal_direct_profiles: storage::terminal::list_direct(conn)?,
            terminal_engines: storage::engines::list(conn)?
                .into_iter()
                .map(|(client, model_id)| EngineSelection { client, model_id })
                .collect(),
            settings: BackupSettings {
                log_level: storage::settings::get(conn)?.log_level,
            },
        })
    }

    pub fn validate(&self) -> CoreResult<()> {
        self.validate_standard_models()?;
        self.validate_providers()?;
        self.validate_profiles()?;
        self.validate_engines()?;
        Ok(())
    }

    fn validate_standard_models(&self) -> CoreResult<()> {
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        for model in &self.standard_models {
            if model.id.trim().is_empty() {
                return Err(CoreError::Validation("标准模型标识不能为空".into()));
            }
            if !ids.insert(model.id.as_str()) {
                return Err(CoreError::Validation(format!(
                    "标准模型标识重复：{}",
                    model.id
                )));
            }
            if model.name.trim().is_empty() {
                return Err(CoreError::Validation("标准模型名称不能为空".into()));
            }
            if !names.insert((model.name.trim(), model.company)) {
                return Err(CoreError::Validation(format!(
                    "同一公司下的标准模型名称重复：{}",
                    model.name.trim()
                )));
            }
            model
                .capabilities
                .validate(model.company.protocol())
                .map_err(|error| {
                    CoreError::Validation(format!("标准模型「{}」：{error}", model.name.trim()))
                })?;
        }
        Ok(())
    }

    fn validate_providers(&self) -> CoreResult<()> {
        let mut provider_ids = HashSet::new();
        let mut names = HashSet::new();
        let mut model_ids = HashSet::new();
        let defaults = self
            .providers
            .iter()
            .filter(|entry| entry.provider.is_default)
            .count();
        if defaults > 1 {
            return Err(CoreError::Validation("备份中只能有一个默认服务商".into()));
        }
        for entry in &self.providers {
            let provider = &entry.provider;
            provider.validate()?;
            if provider.id.trim().is_empty() {
                return Err(CoreError::Validation("服务商标识不能为空".into()));
            }
            if provider.name.trim().is_empty() {
                return Err(CoreError::Validation("服务商名称不能为空".into()));
            }
            if !provider_ids.insert(provider.id.as_str()) {
                return Err(CoreError::Validation(format!(
                    "服务商标识重复：{}",
                    provider.id
                )));
            }
            if !names.insert(crate::domain::provider_slug(&provider.name).to_lowercase()) {
                return Err(CoreError::Validation(format!(
                    "服务商名称重复：{}",
                    provider.name.trim()
                )));
            }
            let mut upstreams = HashSet::new();
            for model in &entry.models {
                if model.id.trim().is_empty() {
                    return Err(CoreError::Validation(format!(
                        "服务商「{}」存在没有标识的模型",
                        provider.name.trim()
                    )));
                }
                if !model_ids.insert(model.id.as_str()) {
                    return Err(CoreError::Validation(format!("模型标识重复：{}", model.id)));
                }
                if model.provider_id != provider.id {
                    return Err(CoreError::Validation(format!(
                        "模型 {} 不属于服务商 {}",
                        model.id, provider.id
                    )));
                }
                let upstream = model.upstream_model.trim();
                if upstream.is_empty() {
                    return Err(CoreError::Validation(format!(
                        "服务商「{}」存在没有名称的模型",
                        provider.name.trim()
                    )));
                }
                if !upstreams.insert(upstream) {
                    return Err(CoreError::Validation(format!(
                        "服务商「{}」的模型名重复：{upstream}",
                        provider.name.trim()
                    )));
                }
                let upstream_protocol = model.upstream_protocol(provider);
                match &model.standard_model_id {
                    Some(id) => {
                        if let Some(standard) = self.standard_models.iter().find(|s| &s.id == id)
                            && standard.company != Company::of(upstream_protocol)
                        {
                            return Err(CoreError::Validation(format!(
                                "标准模型「{}」与 Provider「{}」的模型「{upstream}」协议不一致",
                                standard.name.trim(),
                                provider.name.trim()
                            )));
                        }
                    }
                    None => model
                        .capabilities
                        .validate(upstream_protocol)
                        .map_err(|error| {
                            CoreError::Validation(format!(
                                "Provider「{}」的模型「{upstream}」：{error}",
                                provider.name.trim()
                            ))
                        })?,
                }
            }
        }
        Ok(())
    }

    fn validate_profiles(&self) -> CoreResult<()> {
        let mut proxy_clients = HashSet::new();
        for profile in &self.terminal_proxy_profiles {
            profile.validate()?;
            if !proxy_clients.insert(profile.client) {
                return Err(CoreError::Validation(format!(
                    "终端代理配置重复：{}",
                    profile.client.as_str()
                )));
            }
        }
        let mut direct_clients = HashSet::new();
        for profile in &self.terminal_direct_profiles {
            if !direct_clients.insert(profile.client) {
                return Err(CoreError::Validation(format!(
                    "终端直连配置重复：{}",
                    profile.client.as_str()
                )));
            }
        }
        Ok(())
    }

    fn validate_engines(&self) -> CoreResult<()> {
        let mut clients = HashSet::new();
        for engine in &self.terminal_engines {
            if !clients.insert(engine.client) {
                return Err(CoreError::Validation(format!(
                    "终端引擎重复：{}",
                    engine.client.as_str()
                )));
            }
        }
        Ok(())
    }
}

