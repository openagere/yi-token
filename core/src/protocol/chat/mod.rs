//! OpenAI Chat Completions protocol: to/from the Responses IR (cross-protocol
//! only; the same-protocol path forwards the raw body in the proxy layer).
//!
//! Foreign (non-Chat) IR items and events, such as the Anthropic markers, have
//! no Chat representation and are dropped by the encoders.

pub mod inbound;
pub mod outbound;
pub mod stream;
pub mod types;

/// `InputItem::Unknown` type emitted by the Anthropic inbound for blocks it cannot model.
pub(crate) const ANTHROPIC_UNKNOWN_BLOCK: &str = "anthropic.unknown_block";
/// Prefix of `StreamEvent::Unknown.event` for Anthropic-only events.
pub(crate) const ANTHROPIC_EVENT_PREFIX: &str = "anthropic.";

/// Key under which the `reasoning_content` vendor extension rides on IR reasoning items.
pub(crate) const REASONING_CONTENT: &str = "reasoning_content";
