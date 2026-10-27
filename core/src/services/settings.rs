use super::{Services, SettingsView};
use crate::domain::{LogLevel, Settings};
use crate::error::{CoreError, CoreResult};
use crate::logging;
use crate::storage::settings;
use std::net::IpAddr;
use std::sync::Arc;

impl Services {
    pub async fn get_settings(self: &Arc<Self>) -> CoreResult<SettingsView> {
        self.run(|s| {
            let v = s.db.with_conn(settings::get)?;
            Ok(SettingsView {
                host: v.host,
                port: v.port,
                log_level: v.log_level,
            })
        })
        .await
    }

    /// Saves the settings and applies the log level immediately. When the
    /// listen address changed and the proxy is meant to run, it is restarted.
    pub async fn save_settings(
        self: &Arc<Self>,
        host: String,
        port: u16,
        log_level: LogLevel,
    ) -> CoreResult<()> {
        let (previous, next) = self
            .run(move |s| {
                let host = host.trim().to_owned();
                if host.parse::<IpAddr>().is_err() {
                    return Err(CoreError::Validation("监听地址必须是有效的 IP 地址".into()));
                }
                let previous = s.db.with_conn(settings::get)?;
                let next = Settings {
                    host,
                    port,
                    log_level,
                };
                s.db.with_conn(|c| settings::save(c, &next))?;
                logging::set_level(log_level)?;
                Ok((previous, next))
            })
            .await?;
        let listen_changed = previous.host != next.host || previous.port != next.port;
        let should_run = self.proxy.desired_running() || self.proxy.status().running;
        if listen_changed
            && should_run
            && let Err(e) = self.restart_proxy(&next).await
        {
            tracing::warn!("proxy restart on settings change failed: {e}");
        }
        Ok(())
    }
}
