//! Route snapshot and model resolution (spec 5.1, 5.6).

use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::domain::{AUTO_MODEL, Client, Model, Protocol, Provider, ProxyProfile, RouteMode};
use crate::error::{CoreError, CoreResult};

/// An enabled provider with its models indexed by upstream model name.
#[derive(Debug, Clone)]
pub struct ProviderRoute {
    pub provider: Provider,
    pub models_by_upstream: HashMap<String, Model>,
}

/// Immutable view of everything routing needs; swapped atomically on change.
#[derive(Debug, Clone, Default)]
pub struct RouteSnapshot {
    /// Enabled providers only, by provider id.
    pub providers: HashMap<String, ProviderRoute>,
    /// exposed_name -> (provider_id, model_id), enabled providers only.
    pub by_exposed: HashMap<String, (String, String)>,
    pub default_provider: Option<String>,
    pub proxy_profiles: HashMap<Client, ProxyProfile>,
    /// Per-client selected engine (model id).
    pub engines: HashMap<Client, Option<String>>,
    /// Models of enabled providers, by model id.
    pub models: HashMap<String, Model>,
}

pub trait RouteSource: Send + Sync {
    fn snapshot(&self) -> CoreResult<RouteSnapshot>;
}

/// Holds the current snapshot; readers get a stable `Arc` that survives refreshes.
pub struct Routes {
    current: ArcSwap<RouteSnapshot>,
    source: Arc<dyn RouteSource>,
}

impl Routes {
    /// Loads the first snapshot from `source`.
    pub fn new(source: Arc<dyn RouteSource>) -> CoreResult<Routes> {
        let first = source.snapshot()?;
        Ok(Routes {
            current: ArcSwap::from_pointee(first),
            source,
        })
    }

    /// Rebuilds the snapshot and swaps it in. On failure the old one stays.
    pub fn refresh(&self) -> CoreResult<()> {
        let next = self.source.snapshot()?;
        self.current.store(Arc::new(next));
        Ok(())
    }

    pub fn load(&self) -> Arc<RouteSnapshot> {
        self.current.load_full()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    Client(Client),
}

#[derive(Debug, Clone)]
pub struct Resolved {
    pub provider: Provider,
    pub model: Option<Model>,
    pub upstream_model: String,
    /// `None` on global routes.
    pub route_mode: Option<RouteMode>,
}

pub fn resolve(
    snap: &RouteSnapshot,
    scope: Scope,
    inbound: Protocol,
    requested_model: &str,
) -> CoreResult<Resolved> {
    match scope {
        Scope::Global => resolve_global(snap, requested_model),
        Scope::Client(client) => resolve_client(snap, client, inbound, requested_model),
    }
}

fn resolve_global(snap: &RouteSnapshot, name: &str) -> CoreResult<Resolved> {
    if let Some((pid, mid)) = snap.by_exposed.get(name)
        && let Some(route) = snap.providers.get(pid)
        && let Some(model) = snap.models.get(mid)
    {
        return Ok(Resolved {
            provider: route.provider.clone(),
            upstream_model: model.upstream_model.clone(),
            model: Some(model.clone()),
            route_mode: None,
        });
    }
    let default = snap
        .default_provider
        .as_ref()
        .and_then(|id| snap.providers.get(id));
    match default {
        Some(route) => Ok(Resolved {
            provider: route.provider.clone(),
            model: route.models_by_upstream.get(name).cloned(),
            upstream_model: name.to_owned(),
            route_mode: None,
        }),
        None => Err(CoreError::NotFound(format!(
            "未找到模型 {name}，且没有默认服务商"
        ))),
    }
}

fn no_engine(client: Client) -> CoreError {
    CoreError::Conflict(format!(
        "请先在 yi-token 中为终端 {} 选择当前引擎",
        client.as_str()
    ))
}

fn resolve_auto(snap: &RouteSnapshot, client: Client) -> CoreResult<Resolved> {
    let model = snap
        .engines
        .get(&client)
        .and_then(Option::as_ref)
        .and_then(|id| snap.models.get(id))
        .ok_or_else(|| no_engine(client))?;
    let route = snap
        .providers
        .get(&model.provider_id)
        .ok_or_else(|| no_engine(client))?;
    Ok(Resolved {
        provider: route.provider.clone(),
        upstream_model: model.upstream_model.clone(),
        model: Some(model.clone()),
        route_mode: Some(RouteMode::Auto),
    })
}

fn resolve_client(
    snap: &RouteSnapshot,
    client: Client,
    inbound: Protocol,
    name: &str,
) -> CoreResult<Resolved> {
    let profile = snap
        .proxy_profiles
        .get(&client)
        .ok_or_else(|| CoreError::NotFound(format!("终端 {} 尚未配置代理", client.as_str())))?;
    if profile.protocol != inbound {
        return Err(CoreError::Validation("该终端入口不支持请求的协议".into()));
    }
    if name == AUTO_MODEL || profile.route_mode == RouteMode::Auto {
        return resolve_auto(snap, client);
    }
    // `ProfileModel.model` is the exposed name (unique across providers); the
    // bare upstream name is never an alias.
    for pm in profile.models.iter().filter(|pm| pm.model == name) {
        if let Some((pid, mid)) = snap.by_exposed.get(name)
            && *pid == pm.provider_id
            && let Some(route) = snap.providers.get(pid)
            && let Some(model) = snap.models.get(mid)
        {
            return Ok(Resolved {
                provider: route.provider.clone(),
                upstream_model: model.upstream_model.clone(),
                model: Some(model.clone()),
                route_mode: Some(RouteMode::Fixed),
            });
        }
    }
    Err(CoreError::NotFound(format!(
        "终端 {} 的代理配置中没有可用的模型 {name}（模型或服务商可能已被删除或停用）",
        client.as_str()
    )))
}
