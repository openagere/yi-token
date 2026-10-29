//! Projection of stored steps into the trajectory ledger (spec 8.3.1): turns, steps and
//! records, and the reconstructed request context of one step. Pure Rust over the rows the
//! repo returns; this module runs no SQL.

use std::collections::HashMap;
use std::str::FromStr;

use serde_json::Value;

use crate::domain::{
    ContextItem, LivePhase, LiveRequest, Protocol, RecordKind, RouteMode, StepContext,
    TrajectoryRecord, TrajectoryStep, TrajectoryTurn,
};
use crate::storage::trajectory::repo::{ItemData, StepRead};

/// Body summaries are cut at this many bytes (spec 5.1 / 8.3.1: at most 2 KiB).
const BODY_LIMIT: usize = 2 * 1024;
/// Single-line summaries hold at most this many characters.
const LINE_LIMIT: usize = 200;

/// The blobs that decide whether a system record is due.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heads {
    pub system_blob: Option<String>,
    pub tools_blob: Option<String>,
}

impl Heads {
    /// The heads of a stored step.
    pub fn of(step: &StepRead) -> Self {
        Self {
            system_blob: step.system_blob.clone(),
            tools_blob: step.tools_blob.clone(),
        }
    }
}

/// One step of a page with its items. `input` holds only the items the step added (for a
/// checkpoint, the items after the previous step's context).
#[derive(Debug, Clone)]
pub struct StepSource {
    pub step: StepRead,
    pub number: u32,
    pub input: Vec<ItemData>,
    pub output: Vec<ItemData>,
}

/// A tool result, taken from the first request that carries it.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub step_id: String,
    pub requested_at: i64,
    pub item: ItemData,
}

/// Everything the projection needs for one thread's page.
#[derive(Debug, Clone)]
pub struct PageSource {
    pub thread_id: String,
    /// The thread was opened by a context reset (spec 4.4).
    pub reset: bool,
    /// Steps of the page in `seq` order.
    pub steps: Vec<StepSource>,
    /// Heads of the step before the page. `None` when the page starts at the thread's first step.
    pub before: Option<Heads>,
    /// Tool results by call id.
    pub results: HashMap<String, ToolResult>,
    /// Subagent thread id by the call id that spawned it.
    pub subthreads: HashMap<String, String>,
    /// Blob bodies of system prompts by hash, as stored.
    pub system_json: HashMap<String, String>,
    /// Phase of recent requests by step id, from the live registry.
    pub live_phases: HashMap<String, LivePhase>,
    /// In-flight requests of this session to show as placeholders. Callers pass them
    /// only for the newest page.
    pub live: Vec<LiveRequest>,
    /// Steps in the session, for the placeholder numbers.
    pub session_steps: u32,
}

/// Projects a thread's page into turns, steps and records.
pub fn project_page(src: &PageSource) -> Vec<TrajectoryTurn> {
    let mut steps: Vec<TrajectoryStep> = Vec::with_capacity(src.steps.len() + src.live.len());
    let mut prev: Option<Heads> = src.before.clone();
    for s in &src.steps {
        let records = step_records(src, s, prev.as_ref());
        let mut step = step_header(&s.step, s.number, src.live_phases.get(&s.step.id).copied());
        step.records = records;
        steps.push(step);
        prev = Some(Heads::of(&s.step));
    }
    add_placeholders(src, &mut steps);
    group_turns(steps)
}

/// Step fields without records.
pub fn step_header(step: &StepRead, number: u32, live_phase: Option<LivePhase>) -> TrajectoryStep {
    TrajectoryStep {
        id: step.id.clone(),
        number,
        thread_id: step.thread_id.clone(),
        turn_no: u32::try_from(step.turn_no).unwrap_or(0),
        requested_at: step.requested_at,
        first_token_at: step.first_token_at,
        finished_at: step.finished_at,
        status: step.status.and_then(|v| u16::try_from(v).ok()),
        stop_reason: step.stop_reason.clone(),
        error: step.error.clone(),
        model: step.model.clone(),
        upstream_model: step.upstream_model.clone(),
        provider_name: step.provider_name.clone(),
        inbound_protocol: protocol_column(&step.inbound_protocol),
        upstream_protocol: protocol_column(&step.upstream_protocol),
        route_mode: step.route_mode.as_deref().and_then(route_mode_column),
        input_tokens: non_negative(step.input_tokens),
        output_tokens: non_negative(step.output_tokens),
        cached_tokens: non_negative(step.cached_tokens),
        reasoning_tokens: non_negative(step.reasoning_tokens),
        capture: step.capture.clone(),
        live_phase,
        records: Vec::new(),
    }
}

/// In-flight requests that are not stored yet appear as empty steps at the end of the thread.
fn add_placeholders(src: &PageSource, steps: &mut Vec<TrajectoryStep>) {
    let last = steps
        .last()
        .map(|s| (s.turn_no, s.inbound_protocol, s.upstream_protocol));
    let (turn_no, inbound, upstream) =
        last.unwrap_or((1, Protocol::Anthropic, Protocol::Anthropic));
    let mut placeholders = Vec::new();
    for (i, live) in src.live.iter().enumerate() {
        if steps.iter().any(|s| s.id == live.step_id) {
            continue;
        }
        placeholders.push(TrajectoryStep {
            id: live.step_id.clone(),
            number: src
                .session_steps
                .saturating_add(1 + u32::try_from(i).unwrap_or(0)),
            thread_id: src.thread_id.clone(),
            turn_no,
            requested_at: live.started_at,
            first_token_at: live.first_token_at,
            finished_at: None,
            status: None,
            stop_reason: String::new(),
            error: None,
            model: live.model.clone(),
            upstream_model: live.model.clone(),
            provider_name: live.provider_name.clone(),
            inbound_protocol: inbound,
            upstream_protocol: upstream,
            route_mode: None,
            input_tokens: live.input_tokens,
            output_tokens: live.output_tokens,
            cached_tokens: 0,
            reasoning_tokens: 0,
            capture: "full".into(),
            live_phase: Some(live.phase),
            records: Vec::new(),
        });
    }
    steps.extend(placeholders);
}

/// Groups consecutive steps of one turn. Steps arrive in `seq` order, so turn numbers never
/// repeat out of order.
fn group_turns(steps: Vec<TrajectoryStep>) -> Vec<TrajectoryTurn> {
    let mut turns: Vec<TrajectoryTurn> = Vec::new();
    for step in steps {
        match turns.last_mut() {
            Some(turn) if turn.turn_no == step.turn_no && turn.thread_id == step.thread_id => {
                turn.steps.push(step);
            }
            _ => turns.push(TrajectoryTurn {
                turn_no: step.turn_no,
                thread_id: step.thread_id.clone(),
                steps: vec![step],
            }),
        }
    }
    turns
}

/// All records of one step, in ledger order: compaction, system, user text, then output.
fn step_records(src: &PageSource, s: &StepSource, prev: Option<&Heads>) -> Vec<TrajectoryRecord> {
    let step_id = s.step.id.as_str();
    let mut out = Vec::new();
    if prev.is_none() && src.reset {
        out.push(compacted_record(step_id));
    }
    if let Some(rec) = system_record(src, s, prev) {
        out.push(rec);
    }
    out.extend(s.input.iter().filter_map(user_record));
    out.extend(output_records(src, s));
    out
}

fn blank(kind: RecordKind, id: String, step_id: &str) -> TrajectoryRecord {
    TrajectoryRecord {
        id,
        kind,
        step_id: step_id.to_owned(),
        text: String::new(),
        body: None,
        body_blob: None,
        truncated: false,
        thinking: None,
        call_id: None,
        tool_name: None,
        args: None,
        result: None,
        result_body: None,
        result_blob: None,
        result_step_id: None,
        is_error: false,
        started_at: None,
        duration_ms: None,
        duration_estimated: false,
        sub_thread_id: None,
        system_changed: false,
    }
}

fn compacted_record(step_id: &str) -> TrajectoryRecord {
    let mut rec = blank(
        RecordKind::Compacted,
        format!("{step_id}:compacted:0"),
        step_id,
    );
    rec.text = "上下文已重置".into();
    rec
}

/// A system record when this step's prompt or tools differ from the previous step of the
/// thread, or when it is the thread's first step and carries either.
fn system_record(
    src: &PageSource,
    s: &StepSource,
    prev: Option<&Heads>,
) -> Option<TrajectoryRecord> {
    let cur = Heads::of(&s.step);
    let due = match prev {
        None => cur.system_blob.is_some() || cur.tools_blob.is_some(),
        Some(p) => *p != cur,
    };
    if !due {
        return None;
    }
    let step_id = s.step.id.as_str();
    let mut rec = blank(RecordKind::System, format!("{step_id}:system:0"), step_id);
    rec.system_changed = prev.is_some();
    let instructions = cur.system_blob.as_ref().and_then(|hash| {
        src.system_json
            .get(hash)
            .map(|json| (hash, instructions_of(json)))
    });
    match instructions {
        Some((hash, Some(text))) => {
            rec.text = one_line(&text);
            let (body, cut) = bounded(&text);
            rec.truncated = cut;
            rec.body = Some(body);
            rec.body_blob = cut.then(|| hash.clone());
        }
        _ if cur.tools_blob.is_some() => rec.text = "工具定义".into(),
        _ => rec.text = "无系统提示词与工具定义".into(),
    }
    Some(rec)
}

/// A user record for a message with text. Only the new input items reach this function.
fn user_record(item: &ItemData) -> Option<TrajectoryRecord> {
    if item.read.kind != "message" || item.read.role.as_deref() != Some("user") {
        return None;
    }
    let text = message_text(&item.json);
    if text.trim().is_empty() {
        return None;
    }
    let step_id = item.read.step_id.as_str();
    let mut rec = blank(
        RecordKind::User,
        format!("{step_id}:{}:{}", item.read.side, item.read.ord),
        step_id,
    );
    rec.text = one_line(&text);
    let (body, cut) = bounded(&text);
    rec.truncated = cut || item.truncated;
    rec.body = Some(body);
    rec.body_blob = rec.truncated.then(|| item.read.blob.clone());
    Some(rec)
}

/// The records of a step's output: one message row for all message and reasoning items
/// (placed where the first of them appears), and one row per tool call.
fn output_records(src: &PageSource, s: &StepSource) -> Vec<TrajectoryRecord> {
    let mut out = Vec::new();
    let mut message_done = false;
    for item in &s.output {
        match item.read.kind.as_str() {
            "message" | "reasoning" if !message_done => {
                message_done = true;
                out.push(message_record(s));
            }
            "tool_call" => out.push(tool_record(src, s, item)),
            _ => {}
        }
    }
    out
}

fn message_record(s: &StepSource) -> TrajectoryRecord {
    let step_id = s.step.id.as_str();
    let first_ord = s
        .output
        .iter()
        .find(|i| i.read.kind == "message" || i.read.kind == "reasoning")
        .map_or(0, |i| i.read.ord);
    let mut rec = blank(
        RecordKind::Message,
        format!("{step_id}:output:{first_ord}"),
        step_id,
    );
    let texts: Vec<String> = s
        .output
        .iter()
        .filter(|i| i.read.kind == "message")
        .map(|i| message_text(&i.json))
        .filter(|t| !t.trim().is_empty())
        .collect();
    let full = texts.join("\n");
    rec.text = one_line(&full);
    let (body, cut) = bounded(&full);
    let blob_truncated = s
        .output
        .iter()
        .any(|i| i.read.kind == "message" && i.truncated);
    rec.truncated = cut || blob_truncated;
    rec.body = (!full.is_empty()).then_some(body);
    rec.body_blob = rec.truncated.then(|| {
        s.output
            .iter()
            .find(|i| i.read.kind == "message")
            .map_or_else(String::new, |i| i.read.blob.clone())
    });
    let reasoning: Vec<String> = s
        .output
        .iter()
        .filter(|i| i.read.kind == "reasoning")
        .map(|i| reasoning_text(&i.json))
        .filter(|t| !t.trim().is_empty())
        .collect();
    rec.thinking = (!reasoning.is_empty()).then(|| bounded(&reasoning.join("\n")).0);
    rec.started_at = Some(s.step.requested_at);
    rec.duration_ms = s.step.finished_at.map(|f| f - s.step.requested_at);
    rec
}

fn tool_record(src: &PageSource, s: &StepSource, item: &ItemData) -> TrajectoryRecord {
    let step_id = s.step.id.as_str();
    let call_id = item.read.call_id.clone().filter(|c| !c.is_empty());
    let id = match &call_id {
        Some(c) => format!("call:{c}"),
        None => format!("{step_id}:output:{}", item.read.ord),
    };
    let sub_thread = call_id
        .as_ref()
        .and_then(|c| src.subthreads.get(c))
        .cloned();
    let kind = if sub_thread.is_some() {
        RecordKind::Subtool
    } else {
        RecordKind::Tool
    };
    let mut rec = blank(kind, id, step_id);
    let name = item.read.tool_name.clone().unwrap_or_default();
    let raw_args = function_args(&item.json);
    rec.text = one_line(format!("{name} {raw_args}").trim());
    let (args, cut) = bounded(&raw_args);
    rec.args = Some(args);
    rec.truncated = cut || item.truncated;
    rec.call_id = call_id.clone();
    rec.tool_name = (!name.is_empty()).then_some(name);
    rec.sub_thread_id = sub_thread;
    rec.started_at = s.step.finished_at;
    rec.duration_estimated = true;
    rec.is_error = step_failed(&s.step);
    if let Some(result) = call_id.as_ref().and_then(|c| src.results.get(c)) {
        let output = output_text(&result.item.json);
        rec.result = Some(one_line(&output));
        let (body, result_cut) = bounded(&output);
        rec.result_body = Some(body);
        rec.result_blob =
            (result_cut || result.item.truncated).then(|| result.item.read.blob.clone());
        rec.result_step_id = Some(result.step_id.clone());
        rec.is_error |= is_error(&result.item.json);
        rec.duration_ms = s.step.finished_at.map(|f| result.requested_at - f);
    }
    rec
}

/// A step failed when the upstream answered 400 or more, or the step stopped on an error.
fn step_failed(step: &StepRead) -> bool {
    step.status.is_some_and(|code| code >= 400) || step.stop_reason == "error"
}

/// Builds the full request context of the last step of `chain` (spec 4.4, 5.1). `chain` runs
/// from the checkpoint down to the step itself. Each step's input is followed by its output,
/// since the next request in the chain carries that output as history.
pub fn build_context(
    header: TrajectoryStep,
    chain: &[(Vec<ItemData>, Vec<ItemData>)],
    system_json: Option<&str>,
    tools_json: Option<&str>,
    options: Value,
) -> StepContext {
    let mut input = Vec::new();
    for (idx, (step_input, step_output)) in chain.iter().enumerate() {
        input.extend(step_input.iter().map(context_item));
        if idx + 1 < chain.len() {
            input.extend(step_output.iter().map(context_item));
        }
    }
    let output = chain
        .last()
        .map(|(_, out)| out.iter().map(context_item).collect())
        .unwrap_or_default();
    StepContext {
        step: header,
        system: system_json.and_then(instructions_of),
        tools: tools_json.map(str::to_owned),
        input,
        output,
        options,
        chain_depth: u32::try_from(chain.len().saturating_sub(1)).unwrap_or(u32::MAX),
    }
}

fn context_item(item: &ItemData) -> ContextItem {
    ContextItem {
        kind: item.read.kind.clone(),
        role: item.read.role.clone(),
        json: item.json.clone(),
        truncated: item.truncated,
    }
}

// ------------------------------------------------------------ JSON helpers

fn parse(json: &str) -> Value {
    serde_json::from_str(json).unwrap_or(Value::Null)
}

/// Text of a message body. A body that is not valid JSON (a truncated blob) is used as is.
pub fn message_text(json: &str) -> String {
    match serde_json::from_str::<Value>(json) {
        Ok(v) => content_text(&v["content"]),
        Err(_) => json.to_owned(),
    }
}

fn content_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect(),
        _ => String::new(),
    }
}

/// Reasoning summary text of a reasoning item.
fn reasoning_text(json: &str) -> String {
    let v = parse(json);
    let mut parts: Vec<&str> = Vec::new();
    if let Some(summary) = v.get("summary").and_then(Value::as_array) {
        parts.extend(
            summary
                .iter()
                .filter_map(|s| s.get("text").and_then(Value::as_str)),
        );
    }
    if let Some(text) = v.get("text").and_then(Value::as_str) {
        parts.push(text);
    }
    parts.join("\n")
}

/// The `arguments` of a function call, as the client sent them.
fn function_args(json: &str) -> String {
    match parse(json).get("arguments") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// The `output` of a function call result.
fn output_text(json: &str) -> String {
    match parse(json).get("output") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn is_error(json: &str) -> bool {
    parse(json)
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// The `instructions` field of a stored system blob.
fn instructions_of(json: &str) -> Option<String> {
    parse(json)
        .get("instructions")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

// ------------------------------------------------------- text helpers

/// Collapses whitespace and cuts to [`LINE_LIMIT`] characters, marking the cut with `…`.
pub fn one_line(s: &str) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= LINE_LIMIT {
        return collapsed;
    }
    let mut cut: String = collapsed.chars().take(LINE_LIMIT - 1).collect();
    cut.push('…');
    cut
}

/// Cuts `s` to at most [`BODY_LIMIT`] bytes on a character boundary. Returns the text and
/// whether it was cut.
pub fn bounded(s: &str) -> (String, bool) {
    if s.len() <= BODY_LIMIT {
        return (s.to_owned(), false);
    }
    let mut end = BODY_LIMIT;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].to_owned(), true)
}

fn protocol_column(s: &str) -> Protocol {
    Protocol::from_str(s).unwrap_or(Protocol::Anthropic)
}

fn route_mode_column(s: &str) -> Option<RouteMode> {
    match s {
        "fixed" => Some(RouteMode::Fixed),
        "auto" => Some(RouteMode::Auto),
        _ => None,
    }
}

fn non_negative(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::trajectory::repo::ItemRead;

    #[test]
    fn one_line_collapses_and_caps_length() {
        assert_eq!(one_line("a\n  b\tc"), "a b c");
        let long = "x".repeat(300);
        let cut = one_line(&long);
        assert_eq!(cut.chars().count(), LINE_LIMIT);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn bounded_keeps_char_boundaries() {
        let text = "汉".repeat(1000); // 3000 bytes
        let (body, cut) = bounded(&text);
        assert!(cut);
        assert!(body.len() <= BODY_LIMIT);
        assert!(text.starts_with(&body));
        assert_eq!(bounded("short"), ("short".to_owned(), false));
    }

    #[test]
    fn message_text_reads_string_and_parts() {
        assert_eq!(message_text(r#"{"role":"user","content":"hi"}"#), "hi");
        assert_eq!(
            message_text(
                r#"{"role":"assistant","content":[{"type":"output_text","text":"a"},{"type":"output_text","text":"b"}]}"#
            ),
            "ab"
        );
        // An unparseable blob body is used as it is.
        assert_eq!(message_text("{broken"), "{broken");
    }

    #[test]
    fn reasoning_text_reads_summary_parts() {
        let json = r#"{"type":"reasoning","summary":[{"type":"summary_text","text":"think"}]}"#;
        assert_eq!(reasoning_text(json), "think");
    }

    fn step_read(status: Option<i64>, stop_reason: &str) -> StepRead {
        StepRead {
            id: "st1".into(),
            seq: 1,
            session_id: "s1".into(),
            thread_id: "t1".into(),
            prefix_len: 0,
            checkpoint: false,
            turn_no: 1,
            requested_at: 100,
            first_token_at: None,
            finished_at: Some(120),
            status,
            stop_reason: stop_reason.into(),
            error: None,
            inbound_protocol: "openai_responses".into(),
            upstream_protocol: "openai_responses".into(),
            provider_name: "p".into(),
            model: "m".into(),
            upstream_model: "m".into(),
            route_mode: None,
            input_tokens: 0,
            output_tokens: 0,
            cached_tokens: 0,
            reasoning_tokens: 0,
            system_blob: None,
            tools_blob: None,
            options_json: "{}".into(),
            capture: "full".into(),
            input_count: 0,
            output_count: 1,
        }
    }

    fn call_item(step_id: &str) -> ItemData {
        ItemData {
            read: ItemRead {
                step_id: step_id.into(),
                side: "output".into(),
                ord: 0,
                kind: "tool_call".into(),
                role: None,
                call_id: Some("c1".into()),
                tool_name: Some("shell".into()),
                blob: "b1".into(),
            },
            json: r#"{"type":"function_call","arguments":"{\"cmd\":\"ls\"}"}"#.into(),
            truncated: false,
        }
    }

    /// Projects the single tool call of a step, with an optional stored tool result.
    fn tool_of(step: StepRead, result_json: Option<&str>) -> TrajectoryRecord {
        let step_id = step.id.clone();
        let mut results = HashMap::new();
        if let Some(json) = result_json {
            results.insert(
                "c1".to_owned(),
                ToolResult {
                    step_id: "st2".into(),
                    requested_at: 130,
                    item: ItemData {
                        read: ItemRead {
                            step_id: "st2".into(),
                            side: "input".into(),
                            ord: 1,
                            kind: "tool_result".into(),
                            role: None,
                            call_id: Some("c1".into()),
                            tool_name: None,
                            blob: "b2".into(),
                        },
                        json: json.into(),
                        truncated: false,
                    },
                },
            );
        }
        let src = PageSource {
            thread_id: "t1".into(),
            reset: false,
            steps: Vec::new(),
            before: None,
            results,
            subthreads: HashMap::new(),
            system_json: HashMap::new(),
            live_phases: HashMap::new(),
            live: Vec::new(),
            session_steps: 1,
        };
        let s = StepSource {
            step,
            number: 1,
            input: Vec::new(),
            output: vec![call_item(&step_id)],
        };
        tool_record(&src, &s, &s.output[0])
    }

    #[test]
    fn tool_of_failed_step_is_error() {
        let rec = tool_of(step_read(Some(500), "stop"), None);
        assert_eq!(rec.kind, RecordKind::Tool);
        assert!(rec.is_error);
    }

    #[test]
    fn tool_of_error_stop_reason_is_error() {
        let rec = tool_of(step_read(Some(200), "error"), None);
        assert!(rec.is_error);
    }

    #[test]
    fn tool_result_marked_as_error_is_error() {
        let json = r#"{"type":"tool_result","output":"boom","is_error":true}"#;
        let rec = tool_of(step_read(Some(200), "stop"), Some(json));
        assert!(rec.is_error);
        assert_eq!(rec.result.as_deref(), Some("boom"));
    }

    #[test]
    fn healthy_tool_is_not_error() {
        let json = r#"{"type":"tool_result","output":"ok"}"#;
        let rec = tool_of(step_read(Some(200), "stop"), Some(json));
        assert!(!rec.is_error);
    }
}
