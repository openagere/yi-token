//! File logging with a level that can be changed at runtime.

use crate::domain::LogLevel;
use crate::error::{CoreError, CoreResult};
use crate::storage::enum_str;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, Registry, prelude::*, reload};

/// Base name of the daily-rolling log file (`yi-token.log.<date>`).
pub const LOG_FILE: &str = "yi-token.log";

static HANDLE: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();
static LEVEL: Mutex<LogLevel> = Mutex::new(LogLevel::Info);
static INIT: Mutex<()> = Mutex::new(());

/// Keeps the background log writer alive; drop it to flush.
pub struct LoggingGuard {
    _worker: Option<WorkerGuard>,
}

fn filter_for(level: LogLevel) -> EnvFilter {
    EnvFilter::new(enum_str(&level).unwrap_or_else(|_| "info".into()))
}

/// Installs the global subscriber writing to a daily-rolling file in `log_dir`.
///
/// Idempotent: the global subscriber is installed once per process; later calls
/// return a guard that owns nothing and leave the current level untouched.
pub fn init(log_dir: &Path, level: LogLevel) -> CoreResult<LoggingGuard> {
    let _serial = INIT.lock().unwrap_or_else(|e| e.into_inner());
    if HANDLE.get().is_some() {
        return Ok(LoggingGuard { _worker: None });
    }
    std::fs::create_dir_all(log_dir)?;
    let appender = tracing_appender::rolling::daily(log_dir, LOG_FILE);
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let (layer, handle) = reload::Layer::new(filter_for(level));
    let installed = tracing_subscriber::registry()
        .with(layer)
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(writer),
        )
        .try_init()
        .is_ok();
    if !installed {
        return Ok(LoggingGuard { _worker: None });
    }
    set_stored(level);
    let _ = HANDLE.set(handle);
    Ok(LoggingGuard {
        _worker: Some(guard),
    })
}

fn set_stored(level: LogLevel) {
    *LEVEL.lock().unwrap_or_else(|e| e.into_inner()) = level;
}

/// Changes the active log level. Fails (leaving the recorded level unchanged)
/// when our subscriber is not installed or the reload fails.
pub fn set_level(level: LogLevel) -> CoreResult<()> {
    let handle = HANDLE
        .get()
        .ok_or_else(|| CoreError::Internal("日志系统未初始化，无法调整日志级别".into()))?;
    handle
        .reload(filter_for(level))
        .map_err(|e| CoreError::Internal(format!("调整日志级别失败：{e}")))?;
    set_stored(level);
    Ok(())
}

/// The level most recently applied successfully.
pub fn current_level() -> LogLevel {
    *LEVEL.lock().unwrap_or_else(|e| e.into_inner())
}

/// The last `max_lines` lines (reading at most `max_bytes` from the end) of the
/// newest `yi-token.log*` file in `log_dir`; empty when there is none.
pub fn tail(log_dir: &Path, max_bytes: u64, max_lines: usize) -> CoreResult<Vec<String>> {
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return Ok(Vec::new());
    };
    let newest = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter(|e| e.file_name().to_string_lossy().starts_with(LOG_FILE))
        .max_by_key(|e| (e.metadata().and_then(|m| m.modified()).ok(), e.file_name()));
    match newest {
        Some(entry) => Ok(read_tail(&entry.path(), max_bytes, max_lines)?),
        None => Ok(Vec::new()),
    }
}

fn read_tail(path: &Path, max_bytes: u64, max_lines: usize) -> std::io::Result<Vec<String>> {
    let mut file = File::open(path)?;
    let start = file.metadata()?.len().saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start))?;
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    if start > 0 {
        // Drop the (probably partial) first line.
        reader.read_until(b'\n', &mut line)?;
    }
    let mut lines = VecDeque::with_capacity(max_lines.min(1024));
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        if max_lines == 0 {
            continue;
        }
        if lines.len() == max_lines {
            lines.pop_front();
        }
        lines.push_back(
            String::from_utf8_lossy(&line)
                .trim_end_matches(['\r', '\n'])
                .to_owned(),
        );
    }
    Ok(lines.into())
}
