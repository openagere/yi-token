//! The single settings row.

use super::{enum_str, parse_enum, parse_json};
use crate::domain::{Client, Settings, TrajectorySettings};
use crate::error::{CoreError, CoreResult};
use rusqlite::{Connection, params};

const TRAJECTORY_RETENTION_DAYS: std::ops::RangeInclusive<u32> = 1..=3650;
const TRAJECTORY_MAX_BYTES: std::ops::RangeInclusive<u64> =
    64 * 1024 * 1024..=1024 * 1024 * 1024 * 1024;

pub fn get(conn: &Connection) -> CoreResult<Settings> {
    let (host, port, level): (String, u16, String) = conn.query_row(
        "SELECT host, port, log_level FROM settings WHERE id = 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    Ok(Settings {
        host,
        port,
        log_level: parse_enum(&level)?,
    })
}

pub fn save(conn: &Connection, settings: &Settings) -> CoreResult<()> {
    if settings.host.trim().is_empty() {
        return Err(CoreError::Validation("监听地址不能为空".into()));
    }
    if settings.port == 0 {
        return Err(CoreError::Validation("端口必须在 1 到 65535 之间".into()));
    }
    conn.execute(
        "UPDATE settings SET host = ?1, port = ?2, log_level = ?3 WHERE id = 1",
        params![
            settings.host.trim(),
            settings.port,
            enum_str(&settings.log_level)?
        ],
    )?;
    Ok(())
}

pub fn get_trajectory(conn: &Connection) -> CoreResult<TrajectorySettings> {
    let (enabled, disabled, disabled_global, retention_days, max_bytes, notice_seen): (i64, String, i64, u32, i64, i64) =
        conn.query_row(
            "SELECT trajectory_enabled, trajectory_disabled_clients, trajectory_disabled_global, \
               trajectory_retention_days, trajectory_max_bytes, trajectory_notice_seen FROM settings WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )?;
    Ok(TrajectorySettings {
        enabled: enabled != 0,
        disabled_clients: parse_json::<Vec<Client>>(&disabled, "轨迹设置")?,
        disabled_global: disabled_global != 0,
        retention_days,
        max_bytes: u64::try_from(max_bytes)
            .map_err(|_| CoreError::Internal("轨迹设置数据已损坏：max_bytes 为负数".into()))?,
        notice_seen: notice_seen != 0,
    })
}

pub fn save_trajectory(conn: &Connection, settings: &TrajectorySettings) -> CoreResult<()> {
    if !TRAJECTORY_RETENTION_DAYS.contains(&settings.retention_days) {
        return Err(CoreError::Validation(
            "保留天数必须在 1 到 3650 之间".into(),
        ));
    }
    if !TRAJECTORY_MAX_BYTES.contains(&settings.max_bytes) {
        return Err(CoreError::Validation(
            "存储上限必须在 64 MiB 到 1 TiB 之间".into(),
        ));
    }
    let max_bytes = i64::try_from(settings.max_bytes)
        .map_err(|_| CoreError::Validation("存储上限超出范围".into()))?;
    conn.execute(
        "UPDATE settings SET trajectory_enabled = ?1, trajectory_disabled_clients = ?2, \
           trajectory_disabled_global = ?3, trajectory_retention_days = ?4, trajectory_max_bytes = ?5, \
           trajectory_notice_seen = ?6 \
         WHERE id = 1",
        params![
            settings.enabled,
            serde_json::to_string(&settings.disabled_clients)?,
            settings.disabled_global,
            settings.retention_days,
            max_bytes,
            settings.notice_seen,
        ],
    )?;
    Ok(())
}
