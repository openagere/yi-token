use super::Services;
use crate::backup::{self, import::ImportReport, payload::Payload};
use crate::error::CoreResult;
use secrecy::SecretString;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupExport {
    pub contents: Vec<u8>,
    pub providers: usize,
    pub models: usize,
    pub standard_models: usize,
}

impl Services {
    pub async fn export_config(self: &Arc<Self>, passphrase: String) -> CoreResult<BackupExport> {
        self.run(move |s| {
            let payload = Payload::read(&s.db)?;
            let counts = (
                payload.providers.len(),
                payload
                    .providers
                    .iter()
                    .map(|entry| entry.models.len())
                    .sum(),
                payload.standard_models.len(),
            );
            let contents = backup::encode(&payload, &SecretString::from(passphrase))?;
            Ok(BackupExport {
                contents,
                providers: counts.0,
                models: counts.1,
                standard_models: counts.2,
            })
        })
        .await
    }

    pub async fn import_config(
        self: &Arc<Self>,
        passphrase: String,
        contents: String,
    ) -> CoreResult<ImportReport> {
        self.run(move |s| {
            let payload = backup::decode(&contents, &SecretString::from(passphrase))?;
            let report = s.db.with_tx(|tx| backup::import::merge(tx, payload))?;
            s.routes_changed();
            Ok(report)
        })
        .await
    }
}
