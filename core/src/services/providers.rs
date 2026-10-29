use super::{ModelInput, ModelMapping, ProviderInput, ProviderView, Services};
use crate::domain::{Model, Provider, Thinking};
use crate::error::CoreResult;
use crate::storage::{models, providers};
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::Arc;

fn mapping(m: Model) -> ModelMapping {
    ModelMapping {
        id: m.id,
        provider_id: m.provider_id,
        route_id: m.exposed_name,
        name: m.upstream_model,
        standard_model_id: m.standard_model_id,
        protocol: m.protocol,
        capabilities: Some(m.capabilities),
        non_standard: m.non_standard,
    }
}

fn view(p: Provider, models: Vec<Model>) -> ProviderView {
    ProviderView {
        provider: ProviderInput {
            id: p.id,
            provider_type: p.provider_type,
            name: p.name,
            base_url: p.base_url,
            api_key: p.api_key,
            enabled: p.enabled,
            thinking: p.thinking.unwrap_or(Thinking::Medium),
            extra: p.extra,
            is_default: p.is_default,
        },
        models: models.into_iter().map(mapping).collect(),
    }
}

fn list_views(conn: &Connection) -> CoreResult<Vec<ProviderView>> {
    let mut by_provider: HashMap<String, Vec<Model>> = HashMap::new();
    for m in models::list(conn)? {
        by_provider
            .entry(m.provider_id.clone())
            .or_default()
            .push(m);
    }
    Ok(providers::list(conn)?
        .into_iter()
        .map(|p| {
            let ms = by_provider.remove(&p.id).unwrap_or_default();
            view(p, ms)
        })
        .collect())
}

impl Services {
    pub async fn list_providers(self: &Arc<Self>) -> CoreResult<Vec<ProviderView>> {
        self.run(|s| {
            s.refresh_catalog();
            s.db.with_conn(list_views)
        })
        .await
    }

    /// Creates or updates a provider with its model set. An empty or unknown id
    /// is replaced by a fresh UUID v4; an existing id is kept.
    pub async fn save_provider(
        self: &Arc<Self>,
        provider: ProviderInput,
        models_in: Vec<ModelInput>,
    ) -> CoreResult<ProviderView> {
        self.run(move |s| {
            s.refresh_catalog();
            let saved = s.db.with_tx(|tx| {
                let existing = if provider.id.is_empty() {
                    None
                } else {
                    providers::get(tx, &provider.id)?
                };
                let (id, sort_order) = match &existing {
                    Some(p) => (p.id.clone(), p.sort_order),
                    None => {
                        let next = providers::list(tx)?
                            .iter()
                            .map(|p| p.sort_order)
                            .max()
                            .map_or(0, |m| m + 1);
                        (uuid::Uuid::new_v4().to_string(), next)
                    }
                };
                let domain = Provider {
                    id: id.clone(),
                    provider_type: provider.provider_type,
                    name: provider.name.trim().to_owned(),
                    base_url: provider.base_url.trim().to_owned(),
                    api_key: provider.api_key,
                    enabled: provider.enabled,
                    thinking: Some(provider.thinking),
                    is_default: provider.is_default,
                    sort_order,
                    extra: provider.extra,
                };
                let ms: Vec<Model> = models_in
                    .into_iter()
                    .map(|m| Model {
                        id: m.id,
                        provider_id: id.clone(),
                        standard_model_id: m.standard_model_id,
                        upstream_model: m.name,
                        exposed_name: String::new(),
                        protocol: m.protocol,
                        capabilities: m.capabilities.unwrap_or_default(),
                        non_standard: m.non_standard,
                    })
                    .collect();
                providers::upsert(tx, &domain, &ms)?;
                let stored = providers::get(tx, &id)?.unwrap_or(domain);
                Ok(view(stored, models::list_by_provider(tx, &id)?))
            })?;
            s.routes_changed();
            Ok(saved)
        })
        .await
    }

    pub async fn delete_provider(self: &Arc<Self>, id: String) -> CoreResult<()> {
        self.run(move |s| {
            s.db.with_conn(|c| providers::delete(c, &id))?;
            s.routes_changed();
            Ok(())
        })
        .await
    }
}
