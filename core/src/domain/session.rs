//! Live request registry DTOs (spec 5.x live view) and the persisted session and
//! trajectory DTOs (spec 7, 8.3.1).

use crate::domain::client::Client;
use crate::domain::protocol::Protocol;
use crate::domain::settings::TrajectorySettings;
use crate::domain::terminal::RouteMode;
use serde::{Deserialize, Serialize};

/// How a step was attached to its session (spec 4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/generated/")]
pub enum KeySource {
    Client,
    Chain,
    New,
}

/// Phase of a request shown in the live view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/generated/")]
pub enum LivePhase {
    Waiting,
    Streaming,
    Done,
    Error,
}

/// One request as seen by the live registry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct LiveRequest {
    /// Registry sequence number, assigned when the request finishes (0 while in flight).
    #[ts(type = "number")]
    pub seq: u64,
    pub step_id: String,
    pub client: Option<Client>,
    pub model: String,
    pub provider_name: String,
    /// UTC milliseconds.
    #[ts(type = "number")]
    pub started_at: i64,
    #[ts(type = "number | null")]
    pub first_token_at: Option<i64>,
    #[ts(type = "number | null")]
    pub finished_at: Option<i64>,
    pub phase: LivePhase,
    pub status: Option<u16>,
    #[ts(type = "number")]
    pub input_tokens: u64,
    #[ts(type = "number")]
    pub output_tokens: u64,
    /// Filled in by the trajectory writer after the step is stored.
    pub session_id: Option<String>,
}

/// Registry view: requests in flight, recently completed ones, and the current sequence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct LiveSnapshot {
    pub in_flight: Vec<LiveRequest>,
    pub completed: Vec<LiveRequest>,
    #[ts(type = "number")]
    pub seq: u64,
}

// ------------------------------------------------------- persisted sessions

/// Role of a thread in its session (spec 4.4). `Pending` threads are projected as `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ThreadKindView {
    Main,
    Subagent,
    Side,
    Other,
}

/// One session in the list (no bodies).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct SessionSummary {
    pub id: String,
    /// `None` for requests on the global route.
    pub client: Option<Client>,
    pub title: String,
    pub key_source: KeySource,
    #[ts(type = "number")]
    pub started_at: i64,
    #[ts(type = "number")]
    pub last_at: i64,
    pub step_count: u32,
    #[ts(type = "number")]
    pub input_tokens: u64,
    #[ts(type = "number")]
    pub output_tokens: u64,
    #[ts(type = "number")]
    pub bytes: u64,
    /// Distinct exposed model names used in the session.
    pub models: Vec<String>,
    /// True when the live registry has a request in flight for this session.
    pub live: bool,
}

/// Filters and keyset cursor for `list_sessions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct SessionListInput {
    pub client: Option<Client>,
    /// Case-insensitive substring of the title.
    pub query: Option<String>,
    pub model: Option<String>,
    #[ts(type = "number | null")]
    pub start_ms: Option<i64>,
    #[ts(type = "number | null")]
    pub end_ms: Option<i64>,
    /// Opaque cursor from the previous page: `"{last_at}:{id}"`.
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

/// One page of sessions, newest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct SessionPage {
    pub items: Vec<SessionSummary>,
    /// Cursor for the next page, `None` on the last page.
    pub next_cursor: Option<String>,
}

/// A thread as listed in the session detail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct ThreadView {
    pub id: String,
    pub kind: ThreadKindView,
    pub parent_thread_id: Option<String>,
    pub spawn_call_id: Option<String>,
    pub reset: bool,
    pub step_count: u32,
    #[ts(type = "number")]
    pub started_at: i64,
}

/// Session metadata, its threads and its summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct SessionDetail {
    pub summary: SessionSummary,
    pub threads: Vec<ThreadView>,
}

/// Record kind in the trajectory ledger (spec 8.3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/generated/")]
pub enum RecordKind {
    System,
    User,
    Message,
    Tool,
    Subtool,
    Compacted,
}

/// One ledger row. Bodies are summaries; full text is loaded through `body_blob` / `result_blob`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct TrajectoryRecord {
    /// Stable id: `"{step_id}:{side}:{ord}"`, or `"call:{call_id}"` for tool rows.
    pub id: String,
    pub kind: RecordKind,
    pub step_id: String,
    /// Single-line summary, at most 200 characters.
    pub text: String,
    /// Body summary, at most 2 KiB. `None` when the record has no body.
    pub body: Option<String>,
    /// Blob hash of the full body, present when the body was truncated.
    pub body_blob: Option<String>,
    pub truncated: bool,
    /// Reasoning summary of a message, at most 2 KiB.
    pub thinking: Option<String>,
    pub call_id: Option<String>,
    pub tool_name: Option<String>,
    /// Tool arguments, at most 2 KiB.
    pub args: Option<String>,
    /// Single-line summary of the tool result.
    pub result: Option<String>,
    pub result_body: Option<String>,
    pub result_blob: Option<String>,
    /// The step that carried the tool result.
    pub result_step_id: Option<String>,
    pub is_error: bool,
    #[ts(type = "number | null")]
    pub started_at: Option<i64>,
    #[ts(type = "number | null")]
    pub duration_ms: Option<i64>,
    /// Tool durations are estimates: the agent runs the tool, the proxy only sees the gap.
    pub duration_estimated: bool,
    /// Subagent thread spawned by this tool call.
    pub sub_thread_id: Option<String>,
    /// For `system` rows: the prompt or tool list differs from the previous step in the thread.
    pub system_changed: bool,
}

/// One proxied request, with its projected records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct TrajectoryStep {
    pub id: String,
    /// "Request #N": 1-based position of the step in the session, across all threads.
    pub number: u32,
    pub thread_id: String,
    pub turn_no: u32,
    #[ts(type = "number")]
    pub requested_at: i64,
    #[ts(type = "number | null")]
    pub first_token_at: Option<i64>,
    #[ts(type = "number | null")]
    pub finished_at: Option<i64>,
    pub status: Option<u16>,
    pub stop_reason: String,
    pub error: Option<String>,
    pub model: String,
    pub upstream_model: String,
    pub provider_name: String,
    pub inbound_protocol: Protocol,
    pub upstream_protocol: Protocol,
    pub route_mode: Option<RouteMode>,
    #[ts(type = "number")]
    pub input_tokens: u64,
    #[ts(type = "number")]
    pub output_tokens: u64,
    #[ts(type = "number")]
    pub cached_tokens: u64,
    #[ts(type = "number")]
    pub reasoning_tokens: u64,
    pub capture: String,
    /// Phase from the live registry, when the request is in flight or recently completed.
    pub live_phase: Option<LivePhase>,
    pub records: Vec<TrajectoryRecord>,
}

/// The steps of one turn within one thread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct TrajectoryTurn {
    pub turn_no: u32,
    pub thread_id: String,
    pub steps: Vec<TrajectoryStep>,
}

/// Request for one page of a thread's turns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct TrajectoryInput {
    pub session_id: String,
    /// `None` selects the main thread (or the busiest one when there is no main thread).
    pub thread_id: Option<String>,
    /// Exclusive upper bound on `turn_no`.
    pub before_turn: Option<u32>,
    /// Defaults to 20.
    pub limit_turns: Option<u32>,
}

/// A page of turns, in ascending order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct TrajectoryPage {
    pub turns: Vec<TrajectoryTurn>,
    pub has_older: bool,
    /// Largest `steps.seq` in this page.
    #[ts(type = "number")]
    pub seq: u64,
}

/// One item of a reconstructed request context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct ContextItem {
    pub kind: String,
    pub role: Option<String>,
    /// Full canonical JSON of the item.
    pub json: String,
    pub truncated: bool,
}

/// A single step with its reconstructed request context (spec 4.4, 5.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct StepContext {
    pub step: TrajectoryStep,
    pub system: Option<String>,
    pub tools: Option<String>,
    pub input: Vec<ContextItem>,
    pub output: Vec<ContextItem>,
    #[ts(type = "Record<string, unknown>")]
    pub options: serde_json::Value,
    /// Number of parent links walked back to the checkpoint.
    pub chain_depth: u32,
}

/// A stored blob with its decompressed body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct BlobView {
    pub kind: String,
    #[ts(type = "number")]
    pub raw_bytes: u64,
    pub truncated: bool,
    pub json: String,
}

/// Sessions with a step written after the given cursor, and the new cursor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct SessionChanges {
    pub session_ids: Vec<String>,
    #[ts(type = "number")]
    pub seq: u64,
}

/// Health and size of the trajectory database.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct TrajectoryStatus {
    pub available: bool,
    pub error: Option<String>,
    /// Compressed bytes held by blobs.
    #[ts(type = "number")]
    pub bytes: u64,
    pub sessions: u32,
    pub settings: TrajectorySettings,
}
