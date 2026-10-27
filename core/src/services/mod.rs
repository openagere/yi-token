//! The facade the command layer calls: providers, models, settings, usage, logs.

mod backup;
pub mod dto;
mod engines;
mod logs;
mod models;
mod probe;
mod providers;
mod proxy;
mod settings;
mod terminal;
mod usage;

pub use crate::backup::import::ImportReport;
pub use crate::proxy::monitor::MonitorView;
pub use crate::terminal::TerminalPreview;
pub use backup::BackupExport;
pub use dto::{
    ActiveEngineView, CatalogInfo, LogsView, ModelInput, ModelMapping, ProtocolSupport,
    ProviderInput, ProviderView, ProxyRuntimeView, SettingsView, StandardModelView,
    TerminalAccessMode, TerminalApplyResult, TerminalProfileView, TerminalProfilesView,
    TerminalStatusView, TestResult, UsageQueryInput, UsageReportView,
};

use crate::catalog::{self, CatalogWatcher, sync};
use crate::error::{CoreError, CoreResult};
use crate::logging::{self, LoggingGuard};
use crate::paths::AppPaths;
use crate::proxy::ProxyHandle;
use crate::proxy::monitor::Monitor;
use crate::proxy::resolve::Routes;
use crate::proxy::router::{ProxyState, build};
use crate::proxy::upstream::UpstreamClient;
use crate::storage::route_source::DbRouteSource;
use crate::storage::usage_writer::{UsageSink, UsageWriterHandle, spawn_usage_writer};
use crate::storage::{Db, settings as settings_repo};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

struct CatalogState {
    watcher: CatalogWatcher,
    info: CatalogInfo,
}

pub struct Services {
    pub(crate) db: Db,
    pub(crate) paths: AppPaths,
    pub(crate) terminal_home: PathBuf,
    pub(crate) terminal_env: HashMap<String, String>,
    sink: UsageSink,
    routes: Arc<Routes>,
    pub(crate) http: UpstreamClient,
    writer: Mutex<Option<UsageWriterHandle>>,
    catalog: Mutex<CatalogState>,
    monitor: Monitor,
    proxy: ProxyHandle,
    _logging: LoggingGuard,
    proxy_rt: Option<tokio::runtime::Runtime>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn user_home(paths: &AppPaths) -> PathBuf {
    for key in ["HOME", "USERPROFILE"] {
        if let Some(value) = std::env::var_os(key).filter(|value| !value.is_empty()) {
            return PathBuf::from(value);
        }
    }
    paths
        .home
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| paths.home.clone(), Path::to_path_buf)
}

impl Services {
    /// Opens the database, syncs the model catalog, starts the usage writer,
    /// initializes logging and automatically starts the local proxy. A failed
    /// bind is recorded in the runtime status but does not prevent startup.
    pub fn start(paths: AppPaths) -> CoreResult<Arc<Services>> {
        let terminal_home = user_home(&paths);
        let terminal_env = std::env::vars_os()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
            .collect();
        Self::start_inner(paths, true, terminal_home, terminal_env)
    }

    fn start_inner(
        paths: AppPaths,
        auto_start_proxy: bool,
        terminal_home: PathBuf,
        terminal_env: HashMap<String, String>,
    ) -> CoreResult<Arc<Services>> {
        let db = Db::open(&paths.db)?;
        let level = db.with_conn(settings_repo::get)?.log_level;
        // Logging first, so the catalog fallback warning below is recorded.
        let logging = logging::init(&paths.log_dir, level)?;
        let override_path = paths.catalog_override.as_deref();
        let (file, info) = match catalog::load(override_path) {
            Ok(file) => {
                let info = CatalogInfo::new(&file, override_path);
                (file, info)
            }
            Err(e) => {
                tracing::warn!("model catalog override unusable, using built-in catalog: {e}");
                let file = catalog::load(None)?;
                let info = CatalogInfo::new(&file, None);
                (file, info)
            }
        };
        db.with_tx(|tx| sync::apply(tx, &file))?;
        let routes = Arc::new(Routes::new(Arc::new(DbRouteSource { db: db.clone() }))?);
        let (sink, writer) = spawn_usage_writer(db.clone());
        let monitor = Monitor::default();
        let http = UpstreamClient::new();
        let proxy_state =
            ProxyState::with_http(routes.clone(), sink.clone(), http.clone(), monitor.clone());
        let proxy = ProxyHandle::with_monitor(move || build(proxy_state.clone()), monitor.clone());
        let proxy_rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .thread_name("yi-token-proxy")
            .build()
            .map_err(|e| CoreError::Io(format!("创建代理运行时失败：{e}")))?;
        let services = Arc::new(Services {
            db,
            terminal_home,
            terminal_env,
            sink,
            routes,
            http,
            writer: Mutex::new(Some(writer)),
            catalog: Mutex::new(CatalogState {
                watcher: CatalogWatcher::new(paths.catalog_override.clone()),
                info,
            }),
            monitor,
            proxy,
            paths,
            _logging: logging,
            proxy_rt: Some(proxy_rt),
        });
        if auto_start_proxy && let Err(e) = services.auto_start_proxy() {
            tracing::warn!("automatic proxy start failed: {e}");
        }
        Ok(services)
    }

    /// A fully started instance rooted at `home` for tests; the proxy is not
    /// started until a test asks for it.
    pub fn for_test(home: &Path) -> Arc<Services> {
        Self::start_inner(
            AppPaths::under(home.to_path_buf()),
            false,
            home.to_path_buf(),
            HashMap::new(),
        )
        .expect("start services for test")
    }

    /// Producer handle for usage events (used by the proxy).
    pub fn usage_sink(&self) -> UsageSink {
        self.sink.clone()
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    /// Flushes pending usage events and stops the writer thread.
    pub fn shutdown(&self) {
        if let Some(writer) = lock(&self.writer).take() {
            writer.shutdown();
        }
    }

    /// The live route snapshot holder shared with the proxy.
    pub fn routes(&self) -> Arc<Routes> {
        Arc::clone(&self.routes)
    }

    /// Called after every write that can change routing; swaps in a fresh snapshot.
    pub(crate) fn routes_changed(&self) {
        if let Err(e) = self.routes.refresh() {
            tracing::warn!("route snapshot refresh failed, keeping the previous one: {e}");
        }
    }

    /// Re-reads the override catalog when its file changed on disk.
    pub(crate) fn refresh_catalog(&self) {
        let mut state = lock(&self.catalog);
        if !state.watcher.changed() {
            return;
        }
        let path = self.paths.catalog_override.as_deref();
        let reloaded = catalog::load(path).and_then(|file| {
            self.db.with_tx(|tx| sync::apply(tx, &file))?;
            Ok(CatalogInfo::new(&file, path))
        });
        match reloaded {
            Ok(info) => {
                state.info = info;
                self.routes_changed();
            }
            Err(e) => tracing::warn!("model catalog reload failed: {e}"),
        }
    }

    pub(crate) fn catalog_info(&self) -> CatalogInfo {
        lock(&self.catalog).info.clone()
    }

    /// Runs blocking work on the blocking pool.
    pub(crate) async fn run<T: Send + 'static>(
        self: &Arc<Self>,
        f: impl FnOnce(&Services) -> CoreResult<T> + Send + 'static,
    ) -> CoreResult<T> {
        let me = Arc::clone(self);
        tokio::task::spawn_blocking(move || f(&me))
            .await
            .map_err(|e| CoreError::Internal(format!("后台任务失败：{e}")))?
    }
}

impl Drop for Services {
    fn drop(&mut self) {
        self.shutdown();
        if let Some(runtime) = self.proxy_rt.take() {
            runtime.shutdown_background();
        }
    }
}
