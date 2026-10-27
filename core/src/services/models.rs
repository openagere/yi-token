use super::{CatalogInfo, Services, StandardModelView};
use crate::error::CoreResult;
use crate::storage::{models, standard_models};
use std::collections::HashMap;
use std::sync::Arc;

impl Services {
    pub async fn list_standard_models(self: &Arc<Self>) -> CoreResult<Vec<StandardModelView>> {
        self.run(|s| {
            s.refresh_catalog();
            s.db.with_conn(|c| {
                let mut counts: HashMap<String, u32> = HashMap::new();
                for m in models::list(c)? {
                    if let Some(id) = m.standard_model_id {
                        *counts.entry(id).or_default() += 1;
                    }
                }
                Ok(standard_models::list(c)?
                    .into_iter()
                    .map(|m| {
                        let n = counts.get(&m.id).copied().unwrap_or(0);
                        StandardModelView::new(m, n)
                    })
                    .collect())
            })
        })
        .await
    }

    pub async fn get_model_catalog_info(self: &Arc<Self>) -> CoreResult<CatalogInfo> {
        self.run(|s| {
            s.refresh_catalog();
            Ok(s.catalog_info())
        })
        .await
    }

    /// Creates or updates a standard model. An empty or unknown id is replaced
    /// by a fresh UUID v4; an existing id is kept.
    pub async fn save_standard_model(
        self: &Arc<Self>,
        model: StandardModelView,
    ) -> CoreResult<StandardModelView> {
        self.run(move |s| {
            let saved = s.db.with_tx(|tx| {
                let mut model = model.into_model();
                if model.id.is_empty() || standard_models::get(tx, &model.id)?.is_none() {
                    model.id = uuid::Uuid::new_v4().to_string();
                }
                standard_models::upsert(tx, &model)?;
                let count = models::list(tx)?
                    .iter()
                    .filter(|m| m.standard_model_id.as_deref() == Some(model.id.as_str()))
                    .count();
                let stored = standard_models::get(tx, &model.id)?.unwrap_or(model);
                Ok(StandardModelView::new(stored, count as u32))
            })?;
            s.routes_changed();
            Ok(saved)
        })
        .await
    }

    pub async fn delete_standard_model(self: &Arc<Self>, id: String) -> CoreResult<()> {
        self.run(move |s| {
            s.db.with_conn(|c| standard_models::delete(c, &id))?;
            s.routes_changed();
            Ok(())
        })
        .await
    }
}
