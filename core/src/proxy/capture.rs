//! Capture side of the stream tap and the stop-reason rules (spec 4.5).

use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::Value;

use crate::protocol::EventTap;
use crate::protocol::ir::{self, InputItem, StreamEvent};
use crate::protocol::responses::stream::Collector;
use crate::trajectory::record::StopReason;

/// Collects the translated stream's IR events for capture and times the first token.
pub struct CaptureTap {
    inner: Arc<Mutex<TapState>>,
}

#[derive(Default)]
struct TapState {
    /// `None` when only timing is wanted (capture disabled).
    collector: Option<Collector>,
    first_token_at: Option<i64>,
    saw_error: Option<String>,
}

impl Default for CaptureTap {
    fn default() -> Self {
        Self::new()
    }
}

impl CaptureTap {
    /// Collects the response and times the first token.
    pub fn new() -> Self {
        Self::with_collector(true)
    }

    /// Times the first token and notes errors, without collecting the response.
    pub fn timing_only() -> Self {
        Self::with_collector(false)
    }

    fn with_collector(collect: bool) -> Self {
        let state = TapState {
            collector: collect.then(Collector::new),
            ..TapState::default()
        };
        Self {
            inner: Arc::new(Mutex::new(state)),
        }
    }

    /// Observer to attach to a stream translator. It shares state with `self`.
    pub fn tap(&self) -> EventTap {
        let inner = Arc::clone(&self.inner);
        Box::new(move |event: &StreamEvent| {
            let mut st = lock(&inner);
            match event {
                StreamEvent::OutputTextDelta { .. }
                | StreamEvent::FunctionCallArgumentsDelta { .. }
                | StreamEvent::ReasoningSummaryTextDelta { .. } => {
                    if st.first_token_at.is_none() {
                        st.first_token_at = Some(now_ms());
                    }
                }
                StreamEvent::Error { message, .. } => {
                    st.saw_error.get_or_insert_with(|| {
                        message
                            .clone()
                            .unwrap_or_else(|| "upstream stream error".into())
                    });
                }
                StreamEvent::Failed { .. } => {
                    st.saw_error
                        .get_or_insert_with(|| "response failed".to_owned());
                }
                _ => {}
            }
            if let Some(collector) = st.collector.as_mut() {
                collector.push(event);
            }
        })
    }

    /// Time of the first content delta so far, if any.
    pub fn first_token_at(&self) -> Option<i64> {
        lock(&self.inner).first_token_at
    }

    /// Collected response, first-token time and the first error seen.
    /// Collection stops after this call.
    pub fn take(&self) -> (Option<ir::Response>, Option<i64>, Option<String>) {
        let mut st = lock(&self.inner);
        let response = st.collector.take().and_then(Collector::finish);
        (response, st.first_token_at, st.saw_error.take())
    }
}

fn lock(inner: &Mutex<TapState>) -> MutexGuard<'_, TapState> {
    inner.lock().unwrap_or_else(|p| p.into_inner())
}

/// Milliseconds since the Unix epoch.
pub(super) fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// Normalises how a request ended (spec 4.5). The first matching rule wins.
pub fn stop_reason(
    resp: Option<&ir::Response>,
    status: Option<u16>,
    finished: bool,
    error: Option<&str>,
) -> StopReason {
    // An upstream failure sets `status` or `error` before the request is dropped
    // unfinished, so it is checked before the client-cancelled rule.
    if status.is_some_and(|s| s >= 400) || error.is_some() {
        return StopReason::Error;
    }
    if !finished {
        return StopReason::Cancelled;
    }
    let Some(resp) = resp else {
        return StopReason::Unknown;
    };
    let max_tokens = resp.status.as_deref() == Some("incomplete")
        && resp
            .extra
            .get("incomplete_details")
            .and_then(|d| d.get("reason"))
            .and_then(Value::as_str)
            == Some("max_output_tokens");
    if max_tokens {
        return StopReason::MaxTokens;
    }
    if resp
        .output
        .iter()
        .any(|item| matches!(item, InputItem::FunctionCall(_)))
    {
        return StopReason::ToolUse;
    }
    if resp
        .output
        .iter()
        .any(|item| matches!(item, InputItem::Message(m) if m.extra.contains_key("refusal")))
    {
        return StopReason::Refusal;
    }
    if resp.status.as_deref() == Some("completed") {
        StopReason::End
    } else {
        StopReason::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn response(v: Value) -> ir::Response {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn stop_reason_rules() {
        let completed = response(json!({"status": "completed", "output": [
            {"type": "message", "role": "assistant", "content": []}]}));
        let tool = response(json!({"status": "completed", "output": [
            {"type": "function_call", "call_id": "c", "name": "f", "arguments": "{}"}]}));
        let refusal = response(json!({"status": "completed", "output": [
            {"type": "message", "role": "assistant", "content": [], "refusal": "no"}]}));
        let max = response(json!({"status": "incomplete",
            "incomplete_details": {"reason": "max_output_tokens"}, "output": []}));
        let other = response(json!({"status": "incomplete",
            "incomplete_details": {"reason": "content_filter"}, "output": []}));

        // Client went away before the response finished.
        assert_eq!(
            stop_reason(None, Some(200), false, None),
            StopReason::Cancelled
        );
        assert_eq!(
            stop_reason(Some(&completed), Some(200), false, None),
            StopReason::Cancelled
        );
        // Upstream errors win over cancellation, even though nothing finished.
        assert_eq!(stop_reason(None, Some(500), false, None), StopReason::Error);
        assert_eq!(
            stop_reason(None, None, false, Some("boom")),
            StopReason::Error
        );
        assert_eq!(
            stop_reason(Some(&completed), Some(200), true, Some("mid-stream")),
            StopReason::Error
        );
        assert_eq!(stop_reason(None, Some(429), true, None), StopReason::Error);
        assert_eq!(
            stop_reason(Some(&max), Some(200), true, None),
            StopReason::MaxTokens
        );
        assert_eq!(
            stop_reason(Some(&tool), Some(200), true, None),
            StopReason::ToolUse
        );
        assert_eq!(
            stop_reason(Some(&refusal), Some(200), true, None),
            StopReason::Refusal
        );
        assert_eq!(
            stop_reason(Some(&completed), Some(200), true, None),
            StopReason::End
        );
        assert_eq!(
            stop_reason(Some(&other), Some(200), true, None),
            StopReason::Unknown
        );
        assert_eq!(
            stop_reason(None, Some(200), true, None),
            StopReason::Unknown
        );
    }
}
