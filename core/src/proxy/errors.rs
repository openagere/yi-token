//! HTTP rendering of [`CoreError`] (spec 5.4 / B5).

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::{CoreError, ErrorCode};

impl IntoResponse for CoreError {
    fn into_response(self) -> Response {
        if let CoreError::Upstream(e) = self {
            // Verbatim upstream status, content-type and body.
            let status = StatusCode::from_u16(e.status).unwrap_or(StatusCode::BAD_GATEWAY);
            let mut resp = Response::new(axum::body::Body::from(e.body));
            *resp.status_mut() = status;
            if let Some(ct) = e.content_type.and_then(|c| HeaderValue::from_str(&c).ok()) {
                resp.headers_mut().insert(header::CONTENT_TYPE, ct);
            }
            return resp;
        }
        let code = self.code();
        let status = match code {
            ErrorCode::Validation => StatusCode::BAD_REQUEST,
            ErrorCode::NotFound => StatusCode::NOT_FOUND,
            ErrorCode::Conflict => StatusCode::CONFLICT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let code_str = serde_json::to_value(code)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "internal".into());
        let body = json!({ "error": { "type": code_str, "message": self.message() } });
        (status, Json(body)).into_response()
    }
}
