//! Session trajectory support: request-shape diagnostics for identifying
//! which client and session a request belongs to.

use std::collections::BTreeSet;

use axum::http::HeaderMap;
use serde_json::Value;

pub mod keys;
pub mod link;
pub mod normalize;
pub mod record;

/// Describes the shape of an inbound request for diagnostics: the header names,
/// the top-level body keys, and the keys under `metadata`. Values are never
/// included, so secrets and user identifiers stay out of the log.
pub fn probe_shape(headers: &HeaderMap, body: &Value) -> String {
    let header_names: BTreeSet<&str> = headers.keys().map(|k| k.as_str()).collect();

    let mut body_keys: BTreeSet<String> = BTreeSet::new();
    if let Value::Object(top) = body {
        for (key, value) in top {
            body_keys.insert(key.clone());
            if key == "metadata"
                && let Value::Object(meta) = value
            {
                for meta_key in meta.keys() {
                    body_keys.insert(format!("metadata.{meta_key}"));
                }
            }
        }
    }

    let headers_part = header_names.into_iter().collect::<Vec<_>>().join(",");
    let body_part = body_keys.into_iter().collect::<Vec<_>>().join(",");
    format!("headers=[{headers_part}] body=[{body_part}]")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use serde_json::json;

    #[test]
    fn probe_lists_names_without_values() {
        let session_sentinel = "sentinel-session-value-7f3a";
        let auth_sentinel = "sentinel-auth-value-4b8e";
        let user_sentinel = "sentinel-user-id-value-9c1d";
        let mut headers = HeaderMap::new();
        headers.insert("session_id", HeaderValue::from_static(session_sentinel));
        headers.insert("authorization", HeaderValue::from_static(auth_sentinel));
        let body = json!({"model": "m", "metadata": {"user_id": user_sentinel}});

        let shape = probe_shape(&headers, &body);

        assert!(shape.contains("session_id"), "{shape}");
        assert!(shape.contains("authorization"), "{shape}");
        assert!(shape.contains("metadata.user_id"), "{shape}");
        assert!(!shape.contains(session_sentinel), "{shape}");
        assert!(!shape.contains(auth_sentinel), "{shape}");
        assert!(!shape.contains(user_sentinel), "{shape}");
    }
}
