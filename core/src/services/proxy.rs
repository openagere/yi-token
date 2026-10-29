//! Proxy status, monitor snapshots and listener controls.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use super::{ProxyRuntimeView, Services, SettingsView};
use crate::domain::{LiveSnapshot, Settings};
use crate::error::{CoreError, CoreResult};
use crate::proxy::monitor::MonitorView;
use crate::storage::settings;

pub(crate) fn listen_addr(settings: &Settings) -> CoreResult<SocketAddr> {
    let ip = settings
        .host
        .parse::<IpAddr>()
        .map_err(|_| CoreError::Validation("监听地址必须是有效的 IP 地址".into()))?;
    if settings.port == 0 {
        return Err(CoreError::Validation("端口必须在 1 到 65535 之间".into()));
    }
    Ok(SocketAddr::new(ip, settings.port))
}

fn settings_view(settings: Settings) -> SettingsView {
    SettingsView {
        host: settings.host,
        port: settings.port,
        log_level: settings.log_level,
    }
}

impl Services {
    fn spawn_on_proxy_rt<F, T>(&self, fut: F) -> CoreResult<tokio::task::JoinHandle<T>>
    where
        F: std::future::Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let runtime = self
            .proxy_rt
            .as_ref()
            .ok_or_else(|| CoreError::Internal("代理运行时已停止".into()))?;
        Ok(runtime.spawn(fut))
    }

    async fn run_on_proxy_rt<F>(&self, fut: F) -> CoreResult<()>
    where
        F: std::future::Future<Output = CoreResult<()>> + Send + 'static,
    {
        let task = self.spawn_on_proxy_rt(fut)?;
        task.await
            .map_err(|e| CoreError::Internal(format!("代理任务失败：{e}")))?
    }

    /// Starts the listener on the saved address from a synchronous caller (the
    /// app setup), on the dedicated proxy runtime.
    pub(crate) fn auto_start_proxy(&self) -> CoreResult<()> {
        let settings = self.db.with_conn(settings::get)?;
        let addr = listen_addr(&settings)?;
        let proxy = self.proxy.clone();
        let task = self.spawn_on_proxy_rt(async move { proxy.start(addr).await })?;
        match futures::executor::block_on(task) {
            Ok(result) => result,
            Err(e) => Err(CoreError::Internal(format!("代理启动任务失败：{e}"))),
        }
    }

    pub fn get_proxy_status(&self) -> bool {
        self.proxy.status().running
    }

    pub async fn get_proxy_runtime(self: &Arc<Self>) -> CoreResult<ProxyRuntimeView> {
        self.run(|s| {
            let settings = s.db.with_conn(settings::get)?;
            let snap = s.proxy.snapshot();
            let totals = s.monitor.totals();
            Ok(ProxyRuntimeView {
                phase: snap.phase,
                desired_running: snap.desired_running,
                started_at: snap.status.started_at,
                address: snap.status.addr.map(|addr| addr.to_string()),
                last_error: snap.status.last_error,
                active_requests: totals.active_requests,
                active_connections: totals.connections,
                settings: settings_view(settings),
            })
        })
        .await
    }

    pub fn get_proxy_monitor(&self, minutes: Option<u32>) -> MonitorView {
        self.monitor.snapshot(minutes)
    }

    /// In-flight requests, plus completed ones with a sequence number above `since_seq`.
    pub fn live_requests(&self, since_seq: u64) -> LiveSnapshot {
        self.live.snapshot(since_seq)
    }

    /// Applies a `start` / `stop` / `restart` action to the listener.
    pub async fn control_proxy(self: &Arc<Self>, action: &str) -> CoreResult<()> {
        let action = action.trim();
        if !matches!(action, "start" | "stop" | "restart") {
            return Err(CoreError::Validation("无效的代理操作".into()));
        }
        let settings = self.run(|s| s.db.with_conn(settings::get)).await?;
        let addr = listen_addr(&settings)?;
        let proxy = self.proxy.clone();
        match action {
            "start" => {
                if proxy.status().running {
                    return Ok(());
                }
                self.run_on_proxy_rt(async move { proxy.start(addr).await })
                    .await
            }
            "stop" => {
                self.run_on_proxy_rt(async move {
                    proxy.stop().await;
                    Ok(())
                })
                .await
            }
            _ => {
                self.run_on_proxy_rt(async move { proxy.restart(addr).await })
                    .await
            }
        }
    }

    /// Restarts the listener after a listen-settings change. A failed bind is
    /// reported through the runtime status, as it is for the automatic start.
    pub(crate) async fn restart_proxy(&self, settings: &Settings) -> CoreResult<()> {
        let addr = listen_addr(settings)?;
        let proxy = self.proxy.clone();
        self.run_on_proxy_rt(async move { proxy.start(addr).await })
            .await
    }
}
