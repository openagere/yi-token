//! All-or-nothing application of a terminal plan.
//!
//! Every existing file is backed up first, then files are written through
//! same-directory temporary files and atomically renamed. A failed write or a
//! failed database save restores every touched file in reverse order.

use super::plan::{FileChange, TerminalPlan};
use crate::error::{CoreError, CoreResult};
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedPlan {
    pub backup_paths: Vec<String>,
}

pub fn apply(
    plan: &TerminalPlan,
    after_write: impl FnOnce() -> CoreResult<()>,
) -> CoreResult<AppliedPlan> {
    let backups = write_backups(&plan.changes)?;
    for (index, change) in plan.changes.iter().enumerate() {
        if let Err(error) = write_change(change) {
            return rollback(&plan.changes[..=index], error);
        }
    }
    if let Err(error) = after_write() {
        return rollback(&plan.changes, error);
    }
    Ok(AppliedPlan {
        backup_paths: backups,
    })
}

fn write_backups(changes: &[FileChange]) -> CoreResult<Vec<String>> {
    let stamp = timestamp();
    let mut backups = Vec::new();
    for change in changes {
        let Some(before) = &change.before else {
            continue;
        };
        let parent = change
            .path
            .parent()
            .ok_or_else(|| CoreError::Validation("配置路径无父目录".into()))?;
        let name = change
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let backup = unique_backup(parent, &name, &stamp);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup)
            .map_err(|error| {
                CoreError::Io(format!("创建备份 {} 失败：{error}", backup.display()))
            })?;
        file.write_all(before.as_bytes()).map_err(|error| {
            CoreError::Io(format!("写入备份 {} 失败：{error}", backup.display()))
        })?;
        file.sync_all().map_err(|error| {
            CoreError::Io(format!("同步备份 {} 失败：{error}", backup.display()))
        })?;
        backups.push(backup.display().to_string());
    }
    Ok(backups)
}

fn unique_backup(parent: &Path, name: &str, stamp: &str) -> PathBuf {
    let first = parent.join(format!("{name}.bak-{stamp}"));
    if !first.exists() {
        return first;
    }
    for index in 1.. {
        let candidate = parent.join(format!("{name}.bak-{stamp}-{index}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("an unused backup suffix always exists")
}

fn write_change(change: &FileChange) -> CoreResult<()> {
    match &change.after {
        Some(content) => atomic_write(&change.path, content.as_bytes()),
        None => remove_if_present(&change.path),
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> CoreResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| CoreError::Validation("配置路径无父目录".into()))?;
    fs::create_dir_all(parent)
        .map_err(|error| CoreError::Io(format!("创建目录 {} 失败：{error}", parent.display())))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| CoreError::Io(format!("创建临时文件失败：{error}")))?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    if let Ok(metadata) = fs::metadata(path) {
        temporary
            .as_file()
            .set_permissions(metadata.permissions())?;
    }
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn remove_if_present(path: &Path) -> CoreResult<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CoreError::Io(format!(
            "删除 {} 失败：{error}",
            path.display()
        ))),
    }
}

fn rollback<T>(changes: &[FileChange], error: CoreError) -> CoreResult<T> {
    let mut failures = Vec::new();
    for change in changes.iter().rev() {
        let result = match &change.before {
            Some(source) => atomic_write(&change.path, source.as_bytes()),
            None => remove_if_present(&change.path),
        };
        if let Err(failure) = result {
            failures.push(format!("{}: {failure}", change.path.display()));
        }
    }
    if failures.is_empty() {
        Err(augment(error, "；已恢复原配置"))
    } else {
        Err(CoreError::Internal(format!(
            "{error}；部分配置恢复失败: {}",
            failures.join(", ")
        )))
    }
}

fn augment(error: CoreError, suffix: &str) -> CoreError {
    match error {
        CoreError::Validation(message) => CoreError::Validation(format!("{message}{suffix}")),
        CoreError::NotFound(message) => CoreError::NotFound(format!("{message}{suffix}")),
        CoreError::Conflict(message) => CoreError::Conflict(format!("{message}{suffix}")),
        CoreError::Database(message) => CoreError::Database(format!("{message}{suffix}")),
        CoreError::Io(message) => CoreError::Io(format!("{message}{suffix}")),
        CoreError::Upstream(error) => CoreError::Internal(format!(
            "{}; {}",
            CoreError::Upstream(error).message(),
            suffix.trim_start_matches('；')
        )),
        CoreError::Internal(message) => CoreError::Internal(format!("{message}{suffix}")),
    }
}

fn timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default();
    let days = seconds.div_euclid(86_400);
    let time = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}{month:02}{day:02}{:02}{:02}{:02}",
        time / 3_600,
        (time % 3_600) / 60,
        time % 60
    )
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_shape_is_compact_utc() {
        let stamp = timestamp();
        assert_eq!(stamp.len(), 14);
        assert!(stamp.bytes().all(|byte| byte.is_ascii_digit()));
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }
}
