//! Core error type shared by every layer.

use bytes::Bytes;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};

/// Stable, machine-readable error category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/generated/", rename_all = "snake_case")]
pub enum ErrorCode {
    Validation,
    NotFound,
    Conflict,
    Database,
    Io,
    Upstream,
    Internal,
}

/// A non-success response relayed from an upstream provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamError {
    pub status: u16,
    pub body: Bytes,
    pub content_type: Option<String>,
}

impl UpstreamError {
    const MESSAGE_BODY_LIMIT: usize = 2000;

    pub fn message(&self) -> String {
        let mut end = self.body.len().min(Self::MESSAGE_BODY_LIMIT);
        while end > 0 && end < self.body.len() && (self.body[end] & 0b1100_0000) == 0b1000_0000 {
            end -= 1;
        }
        let text = String::from_utf8_lossy(&self.body[..end]);
        format!("上游返回 {}：{}", self.status, text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    Validation(String),
    NotFound(String),
    Conflict(String),
    Database(String),
    Io(String),
    Upstream(UpstreamError),
    Internal(String),
}

pub type CoreResult<T> = Result<T, CoreError>;

impl CoreError {
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Validation(_) => ErrorCode::Validation,
            Self::NotFound(_) => ErrorCode::NotFound,
            Self::Conflict(_) => ErrorCode::Conflict,
            Self::Database(_) => ErrorCode::Database,
            Self::Io(_) => ErrorCode::Io,
            Self::Upstream(_) => ErrorCode::Upstream,
            Self::Internal(_) => ErrorCode::Internal,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Validation(m)
            | Self::NotFound(m)
            | Self::Conflict(m)
            | Self::Database(m)
            | Self::Io(m)
            | Self::Internal(m) => m.clone(),
            Self::Upstream(e) => e.message(),
        }
    }
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for CoreError {}

impl Serialize for CoreError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("CoreError", 2)?;
        s.serialize_field("code", &self.code())?;
        s.serialize_field("message", &self.message())?;
        s.end()
    }
}

impl From<rusqlite::Error> for CoreError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Database(e.to_string())
    }
}

impl From<r2d2::Error> for CoreError {
    fn from(e: r2d2::Error) -> Self {
        Self::Database(e.to_string())
    }
}

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

impl From<serde_json::Error> for CoreError {
    fn from(e: serde_json::Error) -> Self {
        Self::Validation(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_serializes_code_and_message() {
        let v = serde_json::to_string(&CoreError::Conflict("x".into())).unwrap();
        assert_eq!(v, r#"{"code":"conflict","message":"x"}"#);
    }

    #[test]
    fn upstream_message_truncates_body() {
        let e = UpstreamError {
            status: 502,
            body: Bytes::from("a".repeat(5000)),
            content_type: None,
        };
        assert_eq!(e.message(), format!("上游返回 502：{}", "a".repeat(2000)));
        let short = UpstreamError {
            status: 429,
            body: Bytes::from_static(b"slow down"),
            content_type: Some("text/plain".into()),
        };
        assert_eq!(short.message(), "上游返回 429：slow down");
        assert_eq!(CoreError::Upstream(short).code(), ErrorCode::Upstream);
    }

    #[test]
    fn upstream_message_truncates_on_char_boundary() {
        let mut body = "a".repeat(1999);
        body.push('你');
        body.push_str(&"b".repeat(10));
        let e = UpstreamError {
            status: 500,
            body: Bytes::from(body),
            content_type: None,
        };
        let message = e.message();
        assert!(!message.contains('\u{FFFD}'));
        assert_eq!(message, format!("上游返回 500：{}", "a".repeat(1999)));
    }

    #[test]
    fn conversions_map_to_expected_codes() {
        let io: CoreError = std::io::Error::other("boom").into();
        assert_eq!(io.code(), ErrorCode::Io);
        let json: CoreError = serde_json::from_str::<u8>("x").unwrap_err().into();
        assert_eq!(json.code(), ErrorCode::Validation);
        let db: CoreError = rusqlite::Error::QueryReturnedNoRows.into();
        assert_eq!(db.code(), ErrorCode::Database);
    }
}
