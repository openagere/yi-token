//! Upstream request headers (spec 5.1): provider credentials replace whatever
//! the client sent, and only a whitelist of client headers is forwarded.

use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};

use crate::domain::{Protocol, Provider};

/// Anthropic API version pinned for every Anthropic upstream.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Client headers that describe the client itself or opt into a beta feature.
/// Credentials, `host`, `content-length`, `cookie` and hop-by-hop headers are
/// deliberately absent: the whitelist is the strip list.
const FORWARDED: &[&str] = &[
    "user-agent",
    "originator",
    "session_id",
    "session-id",
    "x-session-id",
    "x-client-request-id",
    "x-codex-turn-id",
    "x-codex-turn-metadata",
    "x-stainless-lang",
    "x-stainless-package-version",
    "x-app",
];

/// Credentials and protocol headers for `provider`, nothing from the client.
pub fn build(provider: &Provider, protocol: Protocol) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let key = provider.api_key.trim();
    match protocol {
        Protocol::Anthropic => {
            // Accept a pasted "Bearer sk-..." too; Anthropic wants the bare key.
            let token = key
                .split_once(' ')
                .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
                .map_or(key, |(_, t)| t.trim());
            if !token.is_empty()
                && let Ok(v) = HeaderValue::from_str(token)
            {
                headers.insert("x-api-key", v);
            }
            headers.insert(
                "anthropic-version",
                HeaderValue::from_static(ANTHROPIC_VERSION),
            );
        }
        Protocol::OpenaiChat | Protocol::Responses => {
            if !key.is_empty()
                && let Ok(v) = HeaderValue::from_str(&format!("Bearer {key}"))
            {
                headers.insert(AUTHORIZATION, v);
            }
        }
    }
    apply_extra(&mut headers, &provider.extra);
    headers
}

/// `build` plus the whitelisted headers of the inbound request. Provider
/// headers win over client headers of the same name.
pub fn forward(inbound: &HeaderMap, provider: &Provider, protocol: Protocol) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (name, value) in inbound {
        if is_forwarded(name, protocol) {
            out.append(name.clone(), value.clone());
        }
    }
    out.extend(build(provider, protocol));
    out
}

fn is_forwarded(name: &HeaderName, protocol: Protocol) -> bool {
    let n = name.as_str();
    // Claude Code adds capabilities as new `anthropic-*` headers (including the
    // auto-mode safeguards beta). Forward the whole prefix when the upstream
    // speaks Anthropic; an allowlist would drop the next one.
    if protocol == Protocol::Anthropic && n.starts_with("anthropic-") {
        return true;
    }
    match n {
        "openai-beta" => protocol != Protocol::Anthropic,
        _ => FORWARDED.contains(&n),
    }
}

/// String entries of `provider.extra` are extra upstream headers.
fn apply_extra(headers: &mut HeaderMap, extra: &serde_json::Value) {
    let Some(map) = extra.as_object() else { return };
    for (k, v) in map {
        if let (Ok(name), Some(Ok(value))) = (
            HeaderName::from_bytes(k.as_bytes()),
            v.as_str().map(HeaderValue::from_str),
        ) {
            headers.insert(name, value);
        }
    }
}

/// Response headers worth relaying on a passthrough response.
pub fn response(upstream: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (name, value) in upstream {
        let n = name.as_str();
        let hop = matches!(
            n,
            "connection"
                | "proxy-connection"
                | "keep-alive"
                | "proxy-authenticate"
                | "te"
                | "trailer"
                | "transfer-encoding"
                | "upgrade"
        );
        // Bodies are re-framed by the server, so length and encoding go.
        if !hop && n != "content-length" && n != "content-encoding" {
            out.append(name.clone(), value.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn provider(key: &str) -> Provider {
        Provider {
            id: "p".into(),
            provider_type: Protocol::OpenaiChat,
            name: "n".into(),
            base_url: "http://x".into(),
            api_key: key.into(),
            enabled: true,
            thinking: None,
            is_default: false,
            sort_order: 0,
            extra: json!({"x-extra": "1", "not-a-header": 5}),
        }
    }

    fn inbound() -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in [
            ("authorization", "Bearer user"),
            ("x-api-key", "user"),
            ("cookie", "a=b"),
            ("host", "127.0.0.1"),
            ("content-length", "9"),
            ("connection", "keep-alive"),
            ("user-agent", "codex/1"),
            ("openai-beta", "b1"),
            ("anthropic-beta", "b2"),
            ("x-unknown", "dropped"),
        ] {
            h.insert(
                HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    #[test]
    fn openai_uses_bearer_and_strips_client_credentials() {
        let h = forward(&inbound(), &provider("sk-1"), Protocol::OpenaiChat);
        assert_eq!(h["authorization"], "Bearer sk-1");
        for gone in [
            "x-api-key",
            "cookie",
            "host",
            "content-length",
            "connection",
            "x-unknown",
            "anthropic-beta",
        ] {
            assert!(!h.contains_key(gone), "{gone} leaked");
        }
        assert_eq!(h["user-agent"], "codex/1");
        assert_eq!(h["openai-beta"], "b1");
        assert_eq!(h["x-extra"], "1");
        assert_eq!(h["content-type"], "application/json");
    }

    #[test]
    fn anthropic_uses_x_api_key_and_version() {
        let mut inbound = inbound();
        inbound.insert("anthropic-beta", HeaderValue::from_static("oauth-2025-04-20,safeguards-2026-01-01"));
        let h = forward(&inbound, &provider("Bearer sk-2"), Protocol::Anthropic);
        assert_eq!(h["x-api-key"], "sk-2");
        assert_eq!(h["anthropic-version"], "2023-06-01");
        assert!(!h.contains_key("authorization"));
        assert!(!h.contains_key("openai-beta"));
        assert_eq!(
            h["anthropic-beta"],
            "oauth-2025-04-20,safeguards-2026-01-01"
        );
    }

    #[test]
    fn empty_key_sends_no_credentials() {
        let h = build(&provider("  "), Protocol::OpenaiChat);
        assert!(!h.contains_key("authorization"));
    }

    #[test]
    fn response_drops_hop_and_framing_headers() {
        let mut up = HeaderMap::new();
        up.insert("retry-after", HeaderValue::from_static("3"));
        up.insert("content-length", HeaderValue::from_static("9"));
        up.insert("transfer-encoding", HeaderValue::from_static("chunked"));
        let out = response(&up);
        assert!(out.contains_key("retry-after"));
        assert!(!out.contains_key("content-length") && !out.contains_key("transfer-encoding"));
    }
}
