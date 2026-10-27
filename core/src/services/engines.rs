//! Per-terminal Auto engine selection.

use super::{ActiveEngineView, Services};
use crate::domain::Client;
use crate::error::{CoreError, CoreResult};
use crate::storage::{engines, models, providers};
use std::sync::Arc;

fn engine_view(
    client: Client,
    model: &crate::domain::Model,
    providers: &[crate::domain::Provider],
) -> ActiveEngineView {
    ActiveEngineView {
        client,
        model_id: model.id.clone(),
        exposed_name: model.exposed_name.clone(),
        provider_name: providers
            .iter()
            .find(|provider| provider.id == model.provider_id)
            .map_or_else(String::new, |provider| provider.name.clone()),
    }
}

impl Services {
    /// Selects (`Some`) or clears (`None`) the Auto engine of `client`.
    /// Terminal files are intentionally not touched.
    pub async fn set_active_engine(
        self: &Arc<Self>,
        client: Client,
        model_id: Option<String>,
    ) -> CoreResult<()> {
        self.run(move |s| {
            s.db.with_conn(|conn| {
                if let Some(id) = model_id.as_deref() {
                    let model = models::get(conn, id)?
                        .ok_or_else(|| CoreError::NotFound("所选引擎模型不存在".into()))?;
                    let provider = providers::get(conn, &model.provider_id)?
                        .ok_or_else(|| CoreError::NotFound("所选引擎的服务商不存在".into()))?;
                    if !provider.enabled {
                        return Err(CoreError::Validation("所选引擎的服务商未启用".into()));
                    }
                }
                engines::set(conn, client, model_id.as_deref())
            })?;
            s.routes_changed();
            Ok(())
        })
        .await
    }

    pub async fn get_active_engines(self: &Arc<Self>) -> CoreResult<Vec<ActiveEngineView>> {
        self.run(|s| {
            s.db.with_conn(|conn| {
                let providers = providers::list(conn)?;
                let mut views = Vec::new();
                for (client, model_id) in engines::list(conn)? {
                    let Some(model_id) = model_id else {
                        continue;
                    };
                    let Some(model) = models::get(conn, &model_id)? else {
                        continue;
                    };
                    views.push(engine_view(client, &model, &providers));
                }
                Ok(views)
            })
        })
        .await
    }

    /// Lists model ids advertised by the provider's own models API.
    pub async fn list_provider_models(self: &Arc<Self>, id: String) -> CoreResult<Vec<String>> {
        let provider = self
            .run(move |s| {
                s.db.with_conn(|conn| providers::get(conn, &id))?
                    .ok_or_else(|| CoreError::NotFound("服务商不存在".into()))
            })
            .await?;
        if !provider.enabled {
            return Err(CoreError::Validation("服务商未启用".into()));
        }
        self.http.list_models(&provider).await
    }

    /// Points `client` at `model` on `provider_id`, creating the local model row when needed.
    /// Terminal files are not touched. Later proxy requests record this real model.
    pub async fn select_terminal_model(
        self: &Arc<Self>,
        client: Client,
        provider_id: String,
        model_name: String,
    ) -> CoreResult<ActiveEngineView> {
        self.run(move |s| {
            let view = s.db.with_conn(|conn| {
                let provider = providers::get(conn, &provider_id)?
                    .ok_or_else(|| CoreError::NotFound("所选引擎的服务商不存在".into()))?;
                if !provider.enabled {
                    return Err(CoreError::Validation("所选引擎的服务商未启用".into()));
                }
                let model = models::ensure(conn, &provider, &model_name)?;
                engines::set(conn, client, Some(&model.id))?;
                let providers = providers::list(conn)?;
                Ok(engine_view(client, &model, &providers))
            })?;
            s.routes_changed();
            Ok(view)
        })
        .await
    }
}
