//! Anthropic Messages protocol: to/from the Responses IR (cross-protocol only;
//! the same-protocol path forwards the raw body in the proxy layer).

pub mod inbound;
pub mod outbound;
pub mod stream;
pub mod types;

/// Marker type used to wrap Anthropic-only items/events inside the IR so an
/// Anthropic outbound can re-emit them verbatim and other protocols skip them.
pub(crate) const UNKNOWN_BLOCK: &str = "anthropic.unknown_block";
/// Marker key set (to `true`) in `Reasoning.extra` when the reasoning item was
/// produced from an Anthropic thinking / redacted_thinking block, i.e. its
/// `encrypted_content` is a genuine Anthropic signature. The Anthropic request
/// outbound only sends a signature for tagged items: `encrypted_content` from
/// other protocols (e.g. Responses) is not a valid thinking signature.
pub(crate) const ANTHROPIC_SIGNATURE: &str = "anthropic_signature";
/// Prefix of `StreamEvent::Unknown.event` for Anthropic events with no IR equivalent.
pub(crate) const EVENT_PREFIX: &str = "anthropic.";

/// `type` field of a JSON object, for log fields.
pub(crate) fn type_name(v: &serde_json::Value) -> &str {
    v.get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("?")
}
