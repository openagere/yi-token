//! SQLite storage: connection pool, migrations and repositories.
//!
//! Repository functions are free functions grouped by module and take the
//! connection (or transaction) as their first argument.

pub mod engines;
pub mod migrations;
pub mod models;
pub mod pool;
pub mod providers;
pub mod run_state;
pub mod settings;
pub mod standard_models;
pub mod terminal;
pub mod trajectory;
pub mod usage;
pub mod usage_writer;

pub use pool::Db;

use crate::error::{CoreError, CoreResult};
use rusqlite::{Connection, Params};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Runs `sql` and maps every row to `T` with `serde_rusqlite`.
pub(crate) fn query_as<T: DeserializeOwned>(
    conn: &Connection,
    sql: &str,
    params: impl Params,
) -> CoreResult<Vec<T>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = serde_rusqlite::from_rows::<T>(stmt.query(params)?);
    rows.map(|row| row.map_err(|e| CoreError::Database(e.to_string())))
        .collect()
}

/// Parses a unit-variant enum from its serde string form.
pub(crate) fn parse_enum<T: DeserializeOwned>(s: &str) -> CoreResult<T> {
    serde_json::from_value(serde_json::Value::String(s.to_owned()))
        .map_err(|_| CoreError::Internal(format!("数据库中存在无法识别的值：{s}")))
}

/// Serializes a unit-variant enum to its serde string form.
pub(crate) fn enum_str<T: Serialize>(v: &T) -> CoreResult<String> {
    match serde_json::to_value(v)? {
        serde_json::Value::String(s) => Ok(s),
        _ => Err(CoreError::Internal("枚举序列化结果不是字符串".into())),
    }
}

pub(crate) fn parse_json<T: DeserializeOwned>(raw: &str, what: &str) -> CoreResult<T> {
    serde_json::from_str(raw).map_err(|e| CoreError::Internal(format!("{what} 数据已损坏：{e}")))
}

/// Maps constraint failures to user-facing errors; everything else stays a database error.
pub(crate) fn map_sqlite(e: rusqlite::Error) -> CoreError {
    if let rusqlite::Error::SqliteFailure(f, msg) = &e
        && f.code == rusqlite::ErrorCode::ConstraintViolation
    {
        let msg = msg.as_deref().unwrap_or_default();
        return match f.extended_code {
            rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
            | rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY => {
                let what = if msg.contains("models.exposed_name") {
                    "模型名称已存在"
                } else if msg.contains("standard_models.name") {
                    "同一协议下的标准模型名称不能重复"
                } else {
                    "数据已存在"
                };
                CoreError::Conflict(what.into())
            }
            rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY => {
                CoreError::Conflict("操作违反引用约束：被引用的对象不存在或仍被使用".into())
            }
            _ => CoreError::Validation(format!("数据不满足约束：{msg}")),
        };
    }
    e.into()
}
