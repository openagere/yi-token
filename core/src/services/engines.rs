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
}
