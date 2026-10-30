//! Builds route snapshots from the database.

use std::collections::HashMap;

use crate::error::CoreResult;
use crate::proxy::resolve::{ProviderRoute, RouteSnapshot, RouteSource};
use crate::storage::{Db, engines, models, providers, terminal};

pub struct DbRouteSource {
    pub db: Db,
}

impl RouteSource for DbRouteSource {
    fn snapshot(&self) -> CoreResult<RouteSnapshot> {
        self.db.with_conn(|c| {
            let mut snap = RouteSnapshot::default();
            for provider in providers::list(c)?.into_iter().filter(|p| p.enabled) {
                if provider.is_default {
                    snap.default_provider = Some(provider.id.clone());
                }
                snap.providers.insert(
                    provider.id.clone(),
                    ProviderRoute {
                        provider,
                        models_by_upstream: HashMap::new(),
                    },
                );
            }
            for model in models::list(c)? {
                let Some(route) = snap.providers.get_mut(&model.provider_id) else {
                    continue;
                };
                route
                    .models_by_upstream
                    .insert(model.upstream_model.clone(), model.clone());
                snap.by_exposed.insert(
                    model.exposed_name.clone(),
                    (model.provider_id.clone(), model.id.clone()),
                );
                snap.models.insert(model.id.clone(), model);
            }
            for profile in terminal::list_proxy(c)? {
                snap.proxy_profiles.insert(profile.client, profile);
            }
            snap.engines = engines::list(c)?.into_iter().collect();
            Ok(snap)
        })
    }
}
