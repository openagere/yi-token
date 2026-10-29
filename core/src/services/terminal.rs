//! Terminal profile status, previews and all-or-nothing applies.

use super::Services;
use super::dto::{
    ActiveEngineView, TerminalAccessMode, TerminalApplyResult, TerminalProfileView,
    TerminalProfilesView, TerminalStatusView,
};
use crate::domain::{
    Client, DirectProfile, Modality, Model, ModelSource, Provider, ProxyProfile, RouteMode,
    Settings, StandardModel,
};
use crate::error::{CoreError, CoreResult};
use crate::storage::{engines, models, providers, settings, standard_models, terminal};
use crate::terminal::clients::{input_for, output_for, spec_for};
use crate::terminal::commit;
use crate::terminal::spec::{ClientCtx, ClientSpec, build_plan};
use crate::terminal::{TerminalMode, TerminalPreview};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;

struct TerminalSnapshot {
    proxy_base: String,
    providers: Vec<Provider>,
    models: Vec<Model>,
    standard_models: Vec<StandardModel>,
    proxy_profiles: HashMap<Client, ProxyProfile>,
    direct_profiles: HashMap<Client, DirectProfile>,
    engines: HashMap<Client, Option<String>>,
    applied_engines: HashMap<Client, String>,
}

impl Services {
    pub async fn get_terminal_profiles(
        self: &std::sync::Arc<Self>,
    ) -> CoreResult<TerminalProfilesView> {
        self.run(|s| {
            s.refresh_catalog();
            let snap = s.terminal_snapshot()?;
            Client::ALL
                .into_iter()
                .map(|client| s.terminal_status_from(&snap, client))
                .collect()
        })
        .await
    }

    pub async fn preview_terminal_config(
        self: &std::sync::Arc<Self>,
        profile: TerminalProfileView,
    ) -> CoreResult<TerminalPreview> {
        self.run(move |s| {
            s.refresh_catalog();
            let (ctx, _) = s.terminal_context(profile.client)?;
            let profile = profile.into_proxy()?;
            validate_proxy(&profile, &ctx)?;
            let plan = build_plan(
                spec_for(profile.client),
                TerminalMode::Proxy(&profile),
                &ctx,
            )?;
            Ok(masked(plan.preview()))
        })
        .await
    }

    pub async fn apply_terminal_config(
        self: &std::sync::Arc<Self>,
        profile: TerminalProfileView,
    ) -> CoreResult<TerminalApplyResult> {
        self.run(move |s| {
            s.refresh_catalog();
            let (ctx, _) = s.terminal_context(profile.client)?;
            let profile = profile.into_proxy()?;
            validate_proxy(&profile, &ctx)?;
            let plan = build_plan(
                spec_for(profile.client),
                TerminalMode::Proxy(&profile),
                &ctx,
            )?;
            let applied = commit::apply(&plan, || {
                s.db.with_conn(|conn| {
                    terminal::save_proxy(conn, &profile)?;
                    terminal::save_proxy_applied_engine(
                        conn,
                        profile.client,
                        ctx.active_engine.as_ref().map(|model| model.id.as_str()),
                    )
                })?;
                s.routes_changed();
                Ok(())
            })?;
            Ok(TerminalApplyResult {
                config_path: plan.config_path.display().to_string(),
                endpoint: redact_endpoint(&plan.endpoint),
                backup_paths: applied.backup_paths,
                model_count: if profile.route_mode == RouteMode::Auto || profile.models.is_empty() {
                    1
                } else {
                    profile.models.len()
                },
            })
        })
        .await
    }

    pub async fn preview_terminal_direct_config(
        self: &std::sync::Arc<Self>,
        profile: DirectProfile,
    ) -> CoreResult<TerminalPreview> {
        self.run(move |s| {
            let (ctx, _) = s.terminal_context(profile.client)?;
            validate_direct(&profile, &ctx)?;
            let plan = build_plan(
                spec_for(profile.client),
                TerminalMode::Direct(&profile),
                &ctx,
            )?;
            Ok(masked(plan.preview()))
        })
        .await
    }

    pub async fn apply_terminal_direct_config(
        self: &std::sync::Arc<Self>,
        profile: DirectProfile,
    ) -> CoreResult<TerminalApplyResult> {
        self.run(move |s| {
            let (ctx, _) = s.terminal_context(profile.client)?;
            let provider = validate_direct(&profile, &ctx)?;
            let model_count = ctx
                .models
                .iter()
                .filter(|model| model.provider_id == provider.id)
                .count();
            let plan = build_plan(
                spec_for(profile.client),
                TerminalMode::Direct(&profile),
                &ctx,
            )?;
            let applied = commit::apply(&plan, || {
                s.db.with_conn(|conn| terminal::save_direct(conn, &profile))?;
                Ok(())
            })?;
            Ok(TerminalApplyResult {
                config_path: plan.config_path.display().to_string(),
                endpoint: redact_endpoint(&plan.endpoint),
                backup_paths: applied.backup_paths,
                model_count,
            })
        })
        .await
    }

    /// Removes yi-token from the terminal files. The saved profile and selected model stay,
    /// so reconnecting restores the same access mode and default selection.
    pub async fn disconnect_terminal(
        self: &std::sync::Arc<Self>,
        client: Client,
    ) -> CoreResult<TerminalApplyResult> {
        self.run(move |s| {
            let (ctx, _) = s.terminal_context(client)?;
            let plan = build_plan(spec_for(client), TerminalMode::Clear(client), &ctx)?;
            let applied = commit::apply(&plan, || Ok(()))?;
            Ok(TerminalApplyResult {
                config_path: plan.config_path.display().to_string(),
                endpoint: String::new(),
                backup_paths: applied.backup_paths,
                model_count: 0,
            })
        })
        .await
    }

    fn terminal_snapshot(&self) -> CoreResult<TerminalSnapshot> {
        self.db.with_conn(|conn| {
            let settings = settings::get(conn)?;
            Ok(TerminalSnapshot {
                proxy_base: proxy_base(&settings)?,
                providers: providers::list(conn)?,
                models: models::list(conn)?,
                standard_models: standard_models::list(conn)?,
                proxy_profiles: terminal::list_proxy(conn)?
                    .into_iter()
                    .map(|p| (p.client, p))
                    .collect(),
                direct_profiles: terminal::list_direct(conn)?
                    .into_iter()
                    .map(|p| (p.client, p))
                    .collect(),
                engines: engines::list(conn)?.into_iter().collect(),
                applied_engines: terminal::list_proxy_applied_engines(conn)?,
            })
        })
    }

    fn terminal_status_from(
        &self,
        snap: &TerminalSnapshot,
        client: Client,
    ) -> CoreResult<TerminalStatusView> {
        let (ctx, active_engine) = self.build_client_ctx(
            client,
            snap.proxy_base.clone(),
            snap.providers.clone(),
            snap.models.clone(),
            snap.standard_models.clone(),
            snap.engines.get(&client).and_then(Option::as_deref),
        );
        let spec = spec_for(client);
        let config_path = spec
            .files(&ctx)
            .first()
            .map(|(path, _)| path.clone())
            .ok_or_else(|| CoreError::Internal("终端配置未声明任何文件".into()))?;
        let exists = config_path.exists();
        let profile = snap.proxy_profiles.get(&client).cloned();
        let direct_profile = snap.direct_profiles.get(&client).cloned();

        // Active means "the file matches what was written". Judge it against the engine recorded at
        // apply time: switching the proxy engine later only changes routing, not the written file.
        let applied_ctx;
        let applied_engine = snap.applied_engines.get(&client);
        let current_engine = ctx
            .active_engine
            .as_ref()
            .map_or("", |model| model.id.as_str());
        let proxy_ctx = match applied_engine {
            Some(engine_id) if engine_id != current_engine => {
                applied_ctx = self
                    .build_client_ctx(
                        client,
                        snap.proxy_base.clone(),
                        snap.providers.clone(),
                        snap.models.clone(),
                        snap.standard_models.clone(),
                        (!engine_id.is_empty()).then_some(engine_id.as_str()),
                    )
                    .0;
                &applied_ctx
            }
            _ => &ctx,
        };
        let (proxy_active, proxy_error) = profile.as_ref().map_or((false, None), |profile| {
            active_for(spec, TerminalMode::Proxy(profile), proxy_ctx)
        });
        let (direct_active, direct_error) =
            direct_profile.as_ref().map_or((false, None), |profile| {
                active_for(spec, TerminalMode::Direct(profile), &ctx)
            });

        let (active_mode, config_error) = if direct_active {
            (TerminalAccessMode::Direct, None)
        } else if proxy_active {
            (TerminalAccessMode::Proxy, None)
        } else {
            (TerminalAccessMode::None, direct_error.or(proxy_error))
        };
        let profile = match profile {
            Some(profile) => profile_view(&profile, active_engine),
            None => default_profile(client, active_engine),
        };

        Ok(TerminalStatusView {
            client,
            config_path: config_path.display().to_string(),
            exists,
            active: direct_active || proxy_active,
            active_mode,
            profile,
            direct_profile,
            config_error,
        })
    }

    fn terminal_context(
        &self,
        client: Client,
    ) -> CoreResult<(ClientCtx, Option<ActiveEngineView>)> {
        self.db.with_conn(|conn| {
            let settings = settings::get(conn)?;
            let providers = providers::list(conn)?;
            let models = models::list(conn)?;
            let standard_models = standard_models::list(conn)?;
            let engine_id = engines::get(conn, client)?;
            Ok(self.build_client_ctx(
                client,
                proxy_base(&settings)?,
                providers,
                models,
                standard_models,
                engine_id.as_deref(),
            ))
        })
    }

    fn build_client_ctx(
        &self,
        client: Client,
        proxy_base: String,
        providers: Vec<Provider>,
        models: Vec<Model>,
        standard_models: Vec<StandardModel>,
        engine_id: Option<&str>,
    ) -> (ClientCtx, Option<ActiveEngineView>) {
        let engine = engine_id.and_then(|id| models.iter().find(|model| model.id == id).cloned());
        let active_engine = engine.as_ref().map(|model| ActiveEngineView {
            client,
            model_id: model.id.clone(),
            exposed_name: model.exposed_name.clone(),
            provider_name: providers
                .iter()
                .find(|provider| provider.id == model.provider_id)
                .map_or_else(String::new, |provider| provider.name.clone()),
        });
        let ctx = ClientCtx {
            proxy_base,
            providers,
            models,
            standard_models,
            active_engine: engine,
            home: self.terminal_home.clone(),
            env: self.terminal_env.clone(),
        };
        (ctx, active_engine)
    }
}

impl TerminalProfileView {
    fn into_proxy(self) -> CoreResult<ProxyProfile> {
        let client = self.client;
        let protocol = self.protocol.unwrap_or_else(|| client.default_protocol());
        if !client.protocols().contains(&protocol) {
            return Err(CoreError::Validation("该终端不支持所选协议".into()));
        }
        Ok(ProxyProfile {
            client,
            protocol,
            route_mode: self.route_mode,
            models: self.models,
            default_model: (!self.default_model.is_empty()).then_some(self.default_model),
        })
    }
}

fn default_profile(client: Client, active_engine: Option<ActiveEngineView>) -> TerminalProfileView {
    TerminalProfileView {
        client,
        models: Vec::new(),
        default_model: String::new(),
        protocol: (client.protocols().len() > 1).then(|| client.default_protocol()),
        route_mode: RouteMode::Fixed,
        active_engine,
    }
}

fn profile_view(
    profile: &ProxyProfile,
    active_engine: Option<ActiveEngineView>,
) -> TerminalProfileView {
    TerminalProfileView {
        client: profile.client,
        models: profile.models.clone(),
        default_model: profile.default_model.clone().unwrap_or_default(),
        protocol: (profile.client.protocols().len() > 1).then_some(profile.protocol),
        route_mode: profile.route_mode,
        active_engine,
    }
}

fn active_for(
    spec: &dyn ClientSpec,
    mode: TerminalMode<'_>,
    ctx: &ClientCtx,
) -> (bool, Option<String>) {
    match build_plan(spec, mode, ctx) {
        Ok(plan) => (plan.changes.is_empty(), None),
        Err(error) => (false, Some(error.message())),
    }
}

fn validate_proxy(profile: &ProxyProfile, ctx: &ClientCtx) -> CoreResult<()> {
    profile.validate()?;
    if profile.models.len() > 500 {
        return Err(CoreError::Validation("最多配置 500 个模型".into()));
    }
    let mut seen = HashSet::new();
    for selection in &profile.models {
        if selection.model.trim().is_empty() || !seen.insert(selection.model.as_str()) {
            return Err(CoreError::Validation("模型名为空或重复".into()));
        }
        let provider = ctx
            .providers
            .iter()
            .find(|provider| provider.id == selection.provider_id)
            .ok_or_else(|| {
                CoreError::NotFound(format!("模型 {} 的 Provider 已删除", selection.model))
            })?;
        if !provider.enabled {
            return Err(CoreError::Validation(format!(
                "Provider {} 未启用",
                provider.name
            )));
        }
        let model = ctx
            .models
            .iter()
            .find(|model| {
                model.provider_id == selection.provider_id
                    && (model.exposed_name == selection.model || model.id == selection.model)
            })
            .ok_or_else(|| {
                CoreError::NotFound(format!(
                    "模型 {} 已被删除或重新映射，请更新终端配置",
                    selection.model
                ))
            })?;
        let capabilities = &model.capabilities;
        let upstream = model.upstream_protocol(provider);
        if !input_for(capabilities, profile.protocol, upstream).contains(&Modality::Text)
            || !output_for(capabilities, profile.protocol, upstream).contains(&Modality::Text)
        {
            return Err(CoreError::Validation(format!(
                "终端模型 {} 必须支持文字输入和输出",
                selection.model
            )));
        }
    }
    Ok(())
}

fn validate_direct<'a>(profile: &DirectProfile, ctx: &'a ClientCtx) -> CoreResult<&'a Provider> {
    let provider = ctx
        .providers
        .iter()
        .find(|provider| provider.id == profile.provider_id)
        .ok_or_else(|| CoreError::NotFound("直连 Provider 不存在，请重新选择".into()))?;
    if !provider.enabled {
        return Err(CoreError::Validation("直连 Provider 未启用".into()));
    }
    if !profile.client.protocols().contains(&provider.provider_type) {
        return Err(CoreError::Validation(format!(
            "{} 直连不支持 {} 原生协议；可改用代理接入",
            profile.client.as_str(),
            provider.provider_type.as_str()
        )));
    }
    match profile.model_source {
        ModelSource::Native => {
            if !matches!(profile.client, Client::Codex | Client::ClaudeCode) {
                return Err(CoreError::Validation("该终端不支持所选直连模型模式".into()));
            }
            if profile
                .model
                .as_deref()
                .is_some_and(|model| !model.trim().is_empty())
            {
                return Err(CoreError::Validation(
                    "跟随终端模型时不应指定 Provider 模型".into(),
                ));
            }
        }
        ModelSource::Provider => {
            let has_models = ctx.models.iter().any(|model| {
                model.provider_id == provider.id
                    && model.protocol.is_none_or(|p| p == provider.provider_type)
            });
            let has_explicit = profile
                .model
                .as_deref()
                .is_some_and(|model| !model.trim().is_empty());
            if !has_models && !has_explicit {
                return Err(CoreError::Validation(
                    "该 Provider 尚未维护上游模型，无法使用 Provider 模型".into(),
                ));
            }
        }
    }
    Ok(provider)
}

fn proxy_base(settings: &Settings) -> CoreResult<String> {
    let ip = settings
        .host
        .parse::<IpAddr>()
        .map_err(|error| CoreError::Validation(format!("无效监听地址: {error}")))?;
    let ip = if ip.is_unspecified() {
        if ip.is_ipv4() {
            IpAddr::from([127, 0, 0, 1])
        } else {
            IpAddr::from(std::net::Ipv6Addr::LOCALHOST)
        }
    } else {
        ip
    };
    let host = if ip.is_ipv6() {
        format!("[{ip}]")
    } else {
        ip.to_string()
    };
    Ok(format!("http://{host}:{}", settings.port))
}

fn masked(preview: TerminalPreview) -> TerminalPreview {
    TerminalPreview {
        endpoint: redact_endpoint(&preview.endpoint),
        ..preview
    }
}

fn redact_endpoint(value: &str) -> String {
    let query_at = value.find('?');
    let fragment_at = value.find('#');
    let end = [query_at, fragment_at]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(value.len());
    let mut safe = value[..end].to_owned();
    if let Some(scheme_end) = safe.find("://") {
        let authority_start = scheme_end + 3;
        let authority_end = safe[authority_start..]
            .find('/')
            .map_or(safe.len(), |offset| authority_start + offset);
        if let Some(userinfo_end) = safe[authority_start..authority_end].rfind('@') {
            safe.replace_range(authority_start..authority_start + userinfo_end + 1, "");
        }
    }
    if query_at.is_some() {
        safe.push_str("?…");
    }
    safe
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_redaction_drops_credentials_and_query() {
        assert_eq!(
            redact_endpoint("https://user:secret@example.com/v1?key=x#f"),
            "https://example.com/v1?…"
        );
        assert_eq!(
            redact_endpoint("https://example.com/v1"),
            "https://example.com/v1"
        );
    }

    #[test]
    fn proxy_base_uses_loopback_for_unspecified_hosts() {
        let settings = Settings {
            host: "0.0.0.0".into(),
            port: 11435,
            log_level: crate::domain::LogLevel::Info,
        };
        assert_eq!(proxy_base(&settings).unwrap(), "http://127.0.0.1:11435");
        let settings = Settings {
            host: "::".into(),
            ..settings
        };
        assert_eq!(proxy_base(&settings).unwrap(), "http://[::1]:11435");
    }
}
