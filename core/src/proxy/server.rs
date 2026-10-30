//! Proxy listener lifecycle: start / stop / restart with graceful shutdown.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::monitor::{Monitor, TrackedListener};
use crate::{CoreError, CoreResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ProxyPhase {
    Starting,
    Running,
    Stopping,
    #[default]
    Stopped,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProxyStatus {
    pub running: bool,
    pub addr: Option<SocketAddr>,
    /// Unix milliseconds.
    pub started_at: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProxySnapshot {
    pub phase: ProxyPhase,
    pub desired_running: bool,
    pub status: ProxyStatus,
}

struct Running {
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

#[derive(Default)]
struct State {
    status: ProxyStatus,
    running: Option<Running>,
}

type RouterFactory = dyn Fn() -> axum::Router + Send + Sync + 'static;

/// Cheaply clonable handle owning the proxy listener. The router factory is
/// reused on every (re)start.
#[derive(Clone)]
pub struct ProxyHandle {
    factory: Arc<RouterFactory>,
    state: Arc<tokio::sync::Mutex<State>>,
    snapshot: Arc<Mutex<ProxySnapshot>>,
    monitor: Monitor,
}

impl ProxyHandle {
    pub fn new(router_factory: impl Fn() -> axum::Router + Send + Sync + 'static) -> Self {
        Self::with_monitor(router_factory, Monitor::default())
    }

    pub fn with_monitor(
        router_factory: impl Fn() -> axum::Router + Send + Sync + 'static,
        monitor: Monitor,
    ) -> Self {
        Self {
            factory: Arc::new(router_factory),
            state: Arc::new(tokio::sync::Mutex::new(State::default())),
            snapshot: Arc::new(Mutex::new(ProxySnapshot::default())),
            monitor,
        }
    }

    pub fn snapshot(&self) -> ProxySnapshot {
        crate::lock::lock_unpoisoned(&self.snapshot).clone()
    }

    pub fn status(&self) -> ProxyStatus {
        self.snapshot().status
    }

    pub fn phase(&self) -> ProxyPhase {
        self.snapshot().phase
    }

    pub fn desired_running(&self) -> bool {
        self.snapshot().desired_running
    }

    fn update_snapshot(&self, f: impl FnOnce(&mut ProxySnapshot)) {
        let mut snap = crate::lock::lock_unpoisoned(&self.snapshot);
        f(&mut snap);
    }

    /// Start listening. If already running, the current listener is stopped first.
    pub async fn start(&self, addr: SocketAddr) -> CoreResult<()> {
        self.update_snapshot(|s| {
            s.desired_running = true;
            s.phase = ProxyPhase::Starting;
        });
        let mut st = self.state.lock().await;
        Self::shutdown(&mut st).await;
        st.status = ProxyStatus::default();
        self.update_snapshot(|s| s.status = st.status.clone());
        match tokio::net::TcpListener::bind(addr).await {
            Err(err) => {
                let msg = format!("端口 {} 已被占用或无法监听：{err}", addr.port());
                st.status = ProxyStatus {
                    last_error: Some(msg.clone()),
                    ..ProxyStatus::default()
                };
                self.update_snapshot(|s| {
                    s.status = st.status.clone();
                    s.phase = ProxyPhase::Error;
                });
                Err(CoreError::Io(msg))
            }
            Ok(listener) => {
                let bound = listener.local_addr().unwrap_or(addr);
                let (tx, rx) = oneshot::channel();
                let app = (self.factory)();
                let tracked = TrackedListener {
                    listener,
                    monitor: self.monitor.clone(),
                };
                let task = tokio::spawn(async move {
                    let _ = axum::serve(tracked, app)
                        .with_graceful_shutdown(async move {
                            let _ = rx.await;
                        })
                        .await;
                });
                st.running = Some(Running { shutdown: tx, task });
                st.status = ProxyStatus {
                    running: true,
                    addr: Some(bound),
                    started_at: Some(
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or_default(),
                    ),
                    last_error: None,
                };
                self.update_snapshot(|s| {
                    s.status = st.status.clone();
                    s.phase = ProxyPhase::Running;
                });
                Ok(())
            }
        }
    }

    pub async fn stop(&self) {
        self.update_snapshot(|s| {
            s.desired_running = false;
            s.phase = ProxyPhase::Stopping;
        });
        let mut st = self.state.lock().await;
        Self::shutdown(&mut st).await;
        st.status.running = false;
        st.status.addr = None;
        st.status.started_at = None;
        st.status.last_error = None;
        self.update_snapshot(|s| {
            s.status = st.status.clone();
            s.phase = ProxyPhase::Stopped;
        });
    }

    pub async fn restart(&self, addr: SocketAddr) -> CoreResult<()> {
        self.update_snapshot(|s| s.phase = ProxyPhase::Starting);
        self.start(addr).await
    }

    async fn shutdown(st: &mut State) {
        if let Some(r) = st.running.take() {
            let _ = r.shutdown.send(());
            let _ = r.task.await;
        }
    }
}
