//! The single settings row.

use super::{enum_str, parse_enum};
use crate::domain::Settings;
use crate::error::{CoreError, CoreResult};
use rusqlite::{Connection, params};

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
