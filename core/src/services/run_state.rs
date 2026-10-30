//! Keeps the run page's proxy monitor across restarts: restored at startup, saved
//! periodically and once more on shutdown.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::error::CoreResult;
use crate::proxy::Monitor;
use crate::proxy::monitor::{MonitorState, now};
use crate::storage::{Db, run_state};

/// How often the monitor is saved while the app runs.
const SAVE_INTERVAL: Duration = Duration::from_secs(15);

/// Owns the saver thread. Dropping its sender makes the thread save once more and exit.
pub struct MonitorSaver {
    stop: Sender<()>,
    thread: JoinHandle<()>,
}

impl MonitorSaver {
    /// Saves the final state and waits for the thread to finish.
    pub fn shutdown(self) {
        drop(self.stop);
        if self.thread.join().is_err() {
            tracing::error!("run monitor saver thread panicked");
        }
    }
}

/// Loads the saved monitor state into `monitor`. A missing or unreadable state is ignored,
/// so the run page simply starts empty.
pub fn restore_monitor(db: &Db, monitor: &Monitor) -> CoreResult<()> {
    let Some(json) = db.with_conn(run_state::load)? else {
        return Ok(());
    };
    match serde_json::from_str::<MonitorState>(&json) {
        Ok(state) => monitor.restore(state),
        Err(e) => tracing::warn!("saved run monitor state is unreadable, starting empty: {e}"),
    }
    Ok(())
}

/// Starts saving `monitor` to `db` every [`SAVE_INTERVAL`].
pub fn spawn_monitor_saver(db: Db, monitor: Monitor) -> MonitorSaver {
    let (stop, rx) = mpsc::channel::<()>();
    let thread = std::thread::Builder::new()
        .name("run-monitor-saver".into())
        .spawn(move || {
            let mut saved = String::new();
            loop {
                // Anything but a timeout means the owner dropped its sender: save once more and stop.
                let last = !matches!(rx.recv_timeout(SAVE_INTERVAL), Err(RecvTimeoutError::Timeout));
                save_if_changed(&db, &monitor, &mut saved);
                if last {
                    break;
                }
            }
        })
        .expect("spawn run monitor saver thread");
    MonitorSaver { stop, thread }
}

fn save_if_changed(db: &Db, monitor: &Monitor, saved: &mut String) {
    let json = match serde_json::to_string(&monitor.export()) {
        Ok(json) => json,
        Err(e) => {
            tracing::warn!("encoding run monitor state failed: {e}");
            return;
        }
    };
    if json == *saved {
        return;
    }
    match db.with_conn(|c| run_state::save(c, &json, now() as i64)) {
        Ok(()) => *saved = json,
        Err(e) => tracing::warn!("saving run monitor state failed: {e}"),
    }
}
