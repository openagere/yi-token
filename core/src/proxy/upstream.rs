//! HTTP client for upstream providers (spec 5.4).

use std::time::Duration;

use axum::http::HeaderMap;
use bytes::Bytes;
use serde_json::{Value, json};

use super::headers;
use crate::domain::{DiscoveredModel, Protocol, Provider};
use crate::error::{CoreError, CoreResult, UpstreamError};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest silence tolerated between two chunks. There is no total timeout:
/// reasoning models legitimately stream for many minutes.
const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// Endpoint path appended to the provider's `base_url`.
pub fn endpoint(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Anthropic => "/v1/messages",
        Protocol::OpenaiChat => "/chat/completions",
        Protocol::Responses => "/responses",
    }
}

/// Model-list path appended to the provider's `base_url`.
pub fn models_endpoint(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Anthropic => "/v1/models",
        Protocol::OpenaiChat | Protocol::Responses => "/models",
    }
}

#[derive(Clone)]
pub struct UpstreamClient {
    http: reqwest::Client,
}

impl Default for UpstreamClient {
    fn default() -> Self {
        Self::new()
    }
}

impl UpstreamClient {
    /// Follows the system proxy settings (reqwest's default).
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_IDLE_TIMEOUT)
            .build()
            .unwrap_or_else(|e| {
                tracing::warn!("upstream client builder failed, using defaults: {e}");
                reqwest::Client::new()
            });
        Self { http }
    }

    /// Sends `body` to `provider` with provider credentials only.
    pub async fn send(
        &self,
        provider: &Provider,
        protocol: Protocol,
        path: &str,
        body: Bytes,
        stream: bool,
    ) -> CoreResult<reqwest::Response> {
        self.send_with(
            provider,
            path,
            body,
            stream,
            headers::build(provider, protocol),
        )
        .await
    }

    /// As [`send`](Self::send) with caller-built headers (see `headers::forward`).
    /// A non-2xx answer becomes [`CoreError::Upstream`] carrying the upstream
    /// status, content type and body untouched.
    pub async fn send_with(
        &self,
        provider: &Provider,
        path: &str,
        body: Bytes,
        stream: bool,
        mut headers: HeaderMap,
    ) -> CoreResult<reqwest::Response> {
        if stream {
            headers.insert(
                axum::http::header::ACCEPT,
                axum::http::HeaderValue::from_static("text/event-stream"),
            );
        }
        let url = format!("{}{path}", provider.base_url.trim_end_matches('/'));
        let resp = self
            .http
            .post(url)
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(unreachable_error)?;
        if resp.status().is_success() {
            return Ok(resp);
        }
        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let body = resp.bytes().await.unwrap_or_default();
        Err(CoreError::Upstream(UpstreamError {
            status,
            body,
            content_type,
        }))
    }

    /// Lists upstream model ids for a configured provider.
    pub async fn list_models(&self, provider: &Provider) -> CoreResult<Vec<String>> {
        Ok(self
            .list_model_entries(provider)
            .await?
            .into_iter()
            .map(|model| model.id)
            .collect())
    }

    /// Lists upstream models with the metadata the provider publishes.
    pub async fn list_model_entries(&self, provider: &Provider) -> CoreResult<Vec<DiscoveredModel>> {
        let url = format!(
            "{}{}",
            provider.base_url.trim_end_matches('/'),
            models_endpoint(provider.provider_type)
        );
        let resp = self
            .http
            .get(url)
            .headers(headers::build(provider, provider.provider_type))
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(unreachable_error)?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let content_type = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let body = resp.bytes().await.unwrap_or_default();
            return Err(CoreError::Upstream(UpstreamError {
                status,
                body,
                content_type,
            }));
        }
        let body = resp.bytes().await.map_err(unreachable_error)?;
        let parsed = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
        Ok(DiscoveredModel::parse_list(&parsed))
    }
}

/// Connection-level failures have no upstream body; they are rendered as a
/// 502 (504 on timeout) with a JSON error.
pub(super) fn unreachable_error(e: reqwest::Error) -> CoreError {
    let (status, what) = if e.is_timeout() {
        (504, "上游请求超时")
    } else {
        (502, "连接上游失败")
    };
    let body =
        json!({"error": {"type": "upstream_unreachable", "message": format!("{what}：{e}")}});
    CoreError::Upstream(UpstreamError {
        status,
        body: Bytes::from(body.to_string()),
        content_type: Some("application/json".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints() {
        assert_eq!(endpoint(Protocol::Anthropic), "/v1/messages");
        assert_eq!(endpoint(Protocol::OpenaiChat), "/chat/completions");
        assert_eq!(endpoint(Protocol::Responses), "/responses");
        assert_eq!(models_endpoint(Protocol::Anthropic), "/v1/models");
        assert_eq!(models_endpoint(Protocol::OpenaiChat), "/models");
    }
}
