//! Provider connectivity test.

use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use serde_json::{Value, json};

use super::{ProviderInput, Services, TestResult};
use crate::domain::{Protocol, Provider};
use crate::error::{CoreError, CoreResult};
use crate::proxy::upstream::endpoint;

/// One-token request in the provider's native protocol.
fn probe_body(protocol: Protocol, model: &str) -> Value {
    match protocol {
        Protocol::Anthropic => json!({"model": model, "max_tokens": 1, "stream": false,
            "messages": [{"role": "user", "content": "hi"}]}),
        Protocol::OpenaiChat => json!({"model": model, "max_completion_tokens": 1,
            "messages": [{"role": "user", "content": "hi"}]}),
        Protocol::Responses => json!({"model": model, "input": "hi",
            "max_output_tokens": 1, "stream": false}),
    }
}

fn describe_failure(status: u16, model: &str, body: &str) -> String {
    let detail = body.to_lowercase();
    let model_problem = detail.contains("model")
        && [
            "unsupported",
            "not found",
            "does not exist",
            "not available",
            "invalid",
        ]
        .iter()
        .any(|hint| detail.contains(hint));
    if model_problem {
        format!(
            "HTTP {status}: 上游模型“{model}”不可用，请核对该服务商支持的模型名称。上游信息：{body}"
        )
    } else {
        format!("HTTP {status}: {body}")
    }
}

impl Services {
    /// Sends a minimal non-streaming request to verify credentials, address
    /// and model name. A failing upstream is a `TestResult` with `ok == false`;
    /// only an unusable input is an error.
    pub async fn test_provider(
        self: &Arc<Self>,
        input: ProviderInput,
        model: String,
    ) -> CoreResult<TestResult> {
        let model = model.trim().to_owned();
        if model.is_empty() {
            return Err(CoreError::Validation("请填写测试模型".into()));
        }
        let native = input.provider_type;
        let provider = Provider {
            id: input.id,
            provider_type: native,
            name: input.name,
            base_url: input.base_url,
            api_key: input.api_key,
            enabled: input.enabled,
            thinking: Some(input.thinking),
            is_default: input.is_default,
            sort_order: 0,
            extra: input.extra,
            protocols: input.protocol_support.to_protocols(native),
        };
        let body = Bytes::from(probe_body(native, &model).to_string());
        let started = Instant::now();
        let outcome = self
            .http
            .send(&provider, native, endpoint(native), body, false)
            .await;
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let (ok, message) = match outcome {
            Ok(resp) => (true, format!("HTTP {}", resp.status().as_u16())),
            Err(CoreError::Upstream(e)) => (
                false,
                describe_failure(e.status, &model, &String::from_utf8_lossy(&e.body)),
            ),
            Err(e) => (false, e.message()),
        };
        Ok(TestResult {
            ok,
            latency_ms,
            message,
        })
    }

    /// Lists models, with published metadata, from the provider's models API using the form's address and key.
    pub async fn discover_provider_models(
        self: &Arc<Self>,
        input: ProviderInput,
    ) -> CoreResult<Vec<crate::domain::DiscoveredModel>> {
        let base_url = input.base_url.trim().trim_end_matches('/').to_owned();
        if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
            return Err(CoreError::Validation(
                "Base URL 必须以 http:// 或 https:// 开头".into(),
            ));
        }
        let provider = Provider {
            id: "draft".into(),
            provider_type: input.provider_type,
            name: input.name,
            base_url,
            api_key: input.api_key,
            enabled: true,
            thinking: Some(input.thinking),
            is_default: false,
            sort_order: 0,
            extra: input.extra,
            protocols: input.protocol_support.to_protocols(input.provider_type),
        };
        self.http.list_model_entries(&provider).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_related_rejections_get_a_hint() {
        assert!(describe_failure(400, "m", "The model `m` does not exist").contains("不可用"));
        assert_eq!(describe_failure(400, "m", "bad key"), "HTTP 400: bad key");
    }
}
