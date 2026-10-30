//! OpenAI Chat Completions protocol: to/from the Responses IR (cross-protocol
//! only; the same-protocol path forwards the raw body in the proxy layer).
//!
//! Foreign (non-Chat) IR items and events, such as the Anthropic markers, have
//! no Chat representation and are dropped by the encoders.

pub mod inbound;
pub mod outbound;
pub mod stream;
pub mod types;

/// Anthropic-only markers, shared with the Anthropic protocol module.
pub(crate) use super::anthropic::{EVENT_PREFIX as ANTHROPIC_EVENT_PREFIX, UNKNOWN_BLOCK as ANTHROPIC_UNKNOWN_BLOCK};

/// Key under which the `reasoning_content` vendor extension rides on IR reasoning items.
pub(crate) const REASONING_CONTENT: &str = "reasoning_content";
