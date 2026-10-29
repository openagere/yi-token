//! Client-supplied session and turn keys (spec 4.3, first rule).

use axum::http::HeaderMap;
use serde_json::Value;

use super::record::ClientKeys;
use crate::domain::Client;

/// Headers that carry the client's session identifier, in lookup order.
const SESSION_HEADERS: [&str; 3] = ["session_id", "session-id", "x-session-id"];
const CODEX_TURN_HEADER: &str = "x-codex-turn-id";
const CLAUDE_CODE_MARKER: &str = "_session_";
const UUID_LEN: usize = 36;
/// Prefix for requests on the global route, which has no terminal client.
const GLOBAL_PREFIX: &str = "api";

/// Reads the client's own session and turn keys (spec 4.3, rule 1). Nothing
/// here parses or reads the body beyond one pointer lookup.
pub fn extract(client: Option<Client>, headers: &HeaderMap, body: &Value) -> ClientKeys {
    let raw_session = SESSION_HEADERS
        .iter()
        .find_map(|name| header_value(headers, name))
        .or_else(|| match client {
            Some(Client::ClaudeCode) => claude_code_session(body),
            _ => None,
        });
    let prefix = client.map_or(GLOBAL_PREFIX, |c| c.as_str());
    let turn_id = match client {
        Some(Client::Codex) => header_value(headers, CODEX_TURN_HEADER).map(str::to_owned),
        _ => None,
    };
    ClientKeys {
        session_key: raw_session.map(|s| format!("{prefix}:{s}")),
        turn_id,
    }
}

/// First non-empty header value, trimmed.
fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Claude Code puts `_session_<uuid>` inside `metadata.user_id`. The first
/// occurrence followed by a well-formed UUID wins. Hand-written on purpose:
/// this is a fixed substring plus a length check, not a pattern.
fn claude_code_session(body: &Value) -> Option<&str> {
    let user_id = body.pointer("/metadata/user_id")?.as_str()?;
    user_id
        .match_indices(CLAUDE_CODE_MARKER)
        .find_map(|(at, _)| {
            let start = at + CLAUDE_CODE_MARKER.len();
            let candidate = user_id.get(start..start + UUID_LEN)?;
            is_lower_uuid(candidate).then_some(candidate)
        })
}

/// `8-4-4-4-12` lowercase hexadecimal, matching `[0-9a-f-]{36}` with hyphens fixed.
fn is_lower_uuid(s: &str) -> bool {
    s.len() == UUID_LEN
        && s.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(&b),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderName, HeaderValue};
    use serde_json::json;

    const UUID: &str = "123e4567-e89b-12d3-a456-426614174000";

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(HeaderName::from_static(k), HeaderValue::from_static(v));
        }
        h
    }

    #[test]
    fn codex_header_and_turn_id() {
        let h = headers(&[("session_id", "sess-1"), ("x-codex-turn-id", "turn-9")]);
        let got = extract(Some(Client::Codex), &h, &json!({}));
        assert_eq!(got.session_key.as_deref(), Some("codex:sess-1"));
        assert_eq!(got.turn_id.as_deref(), Some("turn-9"));
    }

    #[test]
    fn claude_code_user_id_session_suffix() {
        let body = json!({"metadata": {"user_id": format!("user_abc_account_x_session_{UUID}")}});
        let got = extract(Some(Client::ClaudeCode), &HeaderMap::new(), &body);
        assert_eq!(got.session_key, Some(format!("claude-code:{UUID}")));
        assert_eq!(got.turn_id, None);
    }

    #[test]
    fn malformed_user_id_yields_none() {
        for user_id in [
            "user_abc_account_x_session_123".to_string(),
            "user_abc_session_ZZZZZZZZ-e89b-12d3-a456-426614174000".to_string(),
            "no-session-marker-here".to_string(),
            format!("user_x_session_{}", UUID.to_uppercase()),
        ] {
            let body = json!({"metadata": {"user_id": user_id}});
            let got = extract(Some(Client::ClaudeCode), &HeaderMap::new(), &body);
            assert_eq!(got.session_key, None, "{user_id}");
        }
    }

    #[test]
    fn header_wins_over_body() {
        let h = headers(&[("x-session-id", "h-1")]);
        let body = json!({"metadata": {"user_id": format!("u_session_{UUID}")}});
        let got = extract(Some(Client::ClaudeCode), &h, &body);
        assert_eq!(got.session_key.as_deref(), Some("claude-code:h-1"));
    }

    #[test]
    fn first_non_empty_session_header_wins() {
        let h = headers(&[
            ("session_id", "  "),
            ("session-id", "dash-2"),
            ("x-session-id", "x-3"),
        ]);
        let got = extract(Some(Client::Opencode), &h, &json!({}));
        assert_eq!(got.session_key.as_deref(), Some("opencode:dash-2"));
    }

    #[test]
    fn global_route_uses_api_prefix() {
        let h = headers(&[("session_id", "s")]);
        let got = extract(None, &h, &json!({}));
        assert_eq!(got.session_key.as_deref(), Some("api:s"));
    }
}
