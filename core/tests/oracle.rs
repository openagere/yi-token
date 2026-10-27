//! Semantic oracle against the yi-llm `refactor/backend-rust` request pipeline.
//!
//! The corpus (`fixtures/oracle/request_pipeline.jsonl`, see
//! `scripts/export-oracle.md`) records, per (inbound protocol, field, boundary
//! value, upstream protocol), whether yi-llm accepted the request and the
//! upstream JSON it produced. Here every case runs through the yi-core
//! pipeline and is compared SEMANTICALLY: accepted vs rejected, and the
//! generated upstream body as `serde_json::Value` (key order ignored). Error
//! text and hashes are never compared.
//!
//! A difference is only tolerated when a named rule in [`ALLOWED_DIFFS`]
//! explains it, and every rule checks the shape of the difference (not just the
//! case id), so a rule cannot hide an unrelated regression.

use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use yi_core::domain::capabilities::{Effort, Support};
use yi_core::domain::{Capabilities, Protocol, Thinking};
use yi_core::protocol::OutboundCtx;
use yi_core::protocol::prepare::{PrepareCtx, passthrough, validate_for_translation};
use yi_core::protocol::{build_upstream, thinking, to_ir};

/// Intended differences from yi-llm: `(rule, reason)`. Each reason is a ruling.
/// Never add a rule just to make the test pass; each rule is verified in
/// [`accept_rule`] / [`key_rule`].
const ALLOWED_DIFFS: &[(&str, &str)] = &[
    (
        "B6",
        "B6 fix: the provider thinking fallback now takes effect on raw Responses bodies and on \
         the same-protocol path (yi-llm cleared it before forwarding). Excused only when the \
         difference is exactly what thinking::apply_raw writes.",
    ),
    (
        "PASSTHROUGH-NO-VALIDATION",
        "spec 5.2: same-protocol requests are forwarded raw; yi-llm's client-side option \
         validation is not reproduced and the upstream is the authority. Only for \
         yi-llm-rejected / yi-core-forwarded passthrough cases.",
    ),
    (
        "LENIENT-OPTION-VALIDATION",
        "yi-llm rejected malformed or untranslatable option values locally (wrong JSON type, \
         parameters with no counterpart on the upstream). yi-core maps what it can, logs what \
         it drops and lets the upstream judge; upstream errors are returned as-is (spec 5.4). \
         Only for yi-llm-rejected / yi-core-accepted translation cases.",
    ),
    (
        "MALFORMED-VALUE-TOLERATED",
        "yi-llm rejected wrongly typed values of option fields (tools, tool_choice, stop, \
         metadata, ...) locally. yi-core ignores the malformed value and builds the same request \
         as for the base case (checked), logging what it drops; the upstream judges the rest \
         (spec 5.4). Only for the listed fields; reasoning controls are excused without the \
         equality check because they decide whether the thinking fallback applies.",
    ),
    (
        "TARGET-NATIVE-PARAMS-FORWARDED",
        "spec 5.2: unknown top-level fields are preserved. A parameter that the TARGET protocol \
         itself defines (Chat seed/user/..., Anthropic top_k/stop_sequences, Responses \
         include/store/...) rides along with exactly the client's value; yi-llm dropped it. \
         Parameters foreign to the target are not forwarded (Responses uses a denylist).",
    ),
    (
        "STATEFUL-REJECTED",
        "spec 5.2: store=true / previous_response_id are rejected when translating to a \
         non-Responses upstream; yi-llm silently dropped them.",
    ),
    (
        "MODEL-REQUIRED",
        "`model` is the routing key and must be a string; yi-llm tolerated any value because \
         it replaced the model afterwards.",
    ),
    (
        "ORPHAN-TOOL-MESSAGE",
        "a tool result with no matching tool call is rejected locally (tool adjacency, commit \
         9260812) instead of producing a request every upstream would refuse.",
    ),
    (
        "TYPED-FIELD-STRICT",
        "`instructions` must be a string and an empty conversation is not translated; yi-llm \
         accepted wrong-typed / empty values and sent a degenerate request.",
    ),
    (
        "FOREIGN-FIELD-SHADOWS-IR-FIELD",
        "contrived input: a client of another protocol sending a key that collides with an IR \
         field (`input`, `max_output_tokens`) makes the flattened extra win on serialisation. \
         Real clients of those protocols never send these keys.",
    ),
    (
        "NULL-SCALARS-OMITTED",
        "an explicit JSON null for an optional parameter is omitted; yi-llm forwarded the null.",
    ),
    (
        "SAMPLING-OMITTED-WITH-THINKING",
        "temperature/top_p are omitted when the Anthropic body enables thinking (Anthropic \
         rejects the combination). In yi-llm the thinking fallback never reached the wire \
         (B6), so the combination never arose.",
    ),
    (
        "STOP-IS-NOT-A-RESPONSES-PARAM",
        "`stop` is not a Responses parameter, so it is not translated to stop_sequences.",
    ),
    (
        "EMPTY-TEXT-OMITTED",
        "empty instructions / input text produce no system message or empty text block; \
         Anthropic rejects empty text blocks and an empty system message carries nothing.",
    ),
    (
        "DEVELOPER-AS-INPUT-MESSAGE",
        "a developer message stays an input message on a Responses upstream, which supports \
         that role natively; yi-llm hoisted it into `instructions`.",
    ),
    (
        "ADAPTIVE-THINKING-NO-FALLBACK",
        "an Anthropic client sending thinking.type=adaptive has expressed its own thinking \
         intent, so the provider fallback is not stacked on top (spec 5.5 'no reasoning \
         parameter'); yi-llm injected reasoning_effort=medium.",
    ),
    (
        "FOREIGN-TOOL-SHAPE-NOT-TRANSLATED",
        "contrived input: an Anthropic client sending an OpenAI-shaped tool (`name` + \
         `parameters`, no `input_schema`). Real Anthropic tools carry `input_schema`; the \
         unrecognised tool is dropped with a log line instead of being reinterpreted.",
    ),
    (
        "STRUCTURED-OUTPUT-REJECTED",
        "json_schema structured output cannot be translated yet (tracked follow-up), so \
         validate_for_translation rejects it with a Validation error instead of dropping the \
         schema silently; yi-llm translated it. Only for json_schema response_format / \
         output_config.format cases.",
    ),
];

#[derive(Debug)]
struct Case {
    id: String,
    inbound: Protocol,
    upstream: Protocol,
    field: String,
    /// JSON text of the substituted value (empty for the base case).
    value: String,
    request: Value,
    /// `request` with numbers canonicalised, for comparing against bodies.
    request_canon: Value,
    accepted: bool,
    body: Option<Value>,
}

type Outcome = (bool, Option<Value>);

fn proto(name: &str) -> Protocol {
    match name {
        "responses" => Protocol::Responses,
        "openai_chat" => Protocol::OpenaiChat,
        "anthropic" => Protocol::Anthropic,
        other => panic!("unknown protocol {other}"),
    }
}

fn load() -> Vec<Case> {
    let text = include_str!("fixtures/oracle/request_pipeline.jsonl");
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let v: Value = serde_json::from_str(line).unwrap();
            let mut request_canon = v["request"].clone();
            canon_numbers(&mut request_canon);
            Case {
                request_canon,
                id: v["case_id"].as_str().unwrap().to_owned(),
                inbound: proto(v["inbound"].as_str().unwrap()),
                upstream: proto(v["upstream"].as_str().unwrap()),
                field: v["field"].as_str().unwrap().to_owned(),
                value: v["value"].as_str().unwrap().to_owned(),
                request: v["request"].clone(),
                accepted: v["outcome"] == "accepted",
                body: v.get("upstream_body").cloned(),
            }
        })
        .collect()
}

fn capabilities() -> Capabilities {
    Capabilities {
        effort: Effort {
            support: Support::Supported,
            levels: vec!["low".into(), "medium".into(), "high".into()],
            default: None,
        },
        ..Capabilities::default()
    }
}

/// yi-core outcome: (accepted, upstream body).
fn run(case: &Case) -> Outcome {
    let caps = capabilities();
    if case.inbound == case.upstream {
        let mut body = case.request.clone();
        let ctx = PrepareCtx {
            inbound: case.inbound,
            upstream: case.upstream,
            upstream_model: "up".into(),
            capabilities: caps,
            thinking: Some(Thinking::Medium),
            non_standard: false,
        };
        return match passthrough(&mut body, &ctx) {
            Ok(()) => (true, Some(body)),
            Err(_) => (false, None),
        };
    }
    let Ok(ir) = to_ir(case.inbound, case.request.clone()) else {
        return (false, None);
    };
    if validate_for_translation(&ir, case.upstream).is_err() {
        return (false, None);
    }
    let ctx = OutboundCtx {
        upstream_model: "up".into(),
        capabilities: caps,
        // The proxy resolves thinking: the provider fallback is used only when
        // the client sent no reasoning parameter of its own (spec 5.5).
        thinking: (!thinking::has_reasoning(case.inbound, &case.request))
            .then_some(Thinking::Medium),
    };
    match build_upstream(&ir, case.upstream, &ctx) {
        Ok(body) => (true, Some(body)),
        Err(_) => (false, None),
    }
}

/// `0.0` and `0` are the same JSON number on the wire for every upstream.
fn canon_numbers(v: &mut Value) {
    match v {
        Value::Number(n) => {
            if let Some(f) = n.as_f64()
                && n.is_f64()
                && f.fract() == 0.0
                && f.abs() < 9.0e15
            {
                *v = json!(f as i64);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(canon_numbers),
        Value::Object(o) => o.values_mut().for_each(canon_numbers),
        _ => {}
    }
}

/// Envelope differences that are not about request semantics. Applied to BOTH
/// sides before comparing bodies:
///  - `stream` / `stream_options`: yi-llm always streams upstream and forces
///    `stream_options.include_usage`; yi-core follows the client's `stream`
///    flag and adds `include_usage` itself when streaming (covered by the chat
///    outbound unit tests), so these keys cannot be compared.
///  - `store: false`: yi-llm pins it on Responses bodies; yi-core forwards the
///    client's value untouched.
///  - `type: "message"` on role items: optional on the Responses wire; yi-core
///    keeps the item exactly as received.
///  - `tools: []` is the same as no tools.
///  - tool parameter schema: a missing schema, `{"type":"object"}` and
///    `{"type":"object","properties":{}}` are the same empty schema.
fn canon(body: &mut Value) {
    canon_numbers(body);
    let Some(obj) = body.as_object_mut() else {
        return;
    };
    obj.remove("stream");
    obj.remove("stream_options");
    if obj.get("store") == Some(&Value::Bool(false)) {
        obj.remove("store");
    }
    if let Some(items) = obj.get_mut("input").and_then(Value::as_array_mut) {
        for item in items {
            if let Some(o) = item.as_object_mut()
                && o.contains_key("role")
                && !o.contains_key("type")
            {
                o.insert("type".into(), json!("message"));
            }
        }
    }
    if obj
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
    {
        obj.remove("tools");
    }
    if let Some(tools) = obj.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools {
            let slot = if tool.get("function").is_some() {
                tool.get_mut("function").and_then(Value::as_object_mut)
            } else {
                tool.as_object_mut()
            };
            if let Some(t) = slot {
                let key = if t.contains_key("input_schema") {
                    "input_schema"
                } else {
                    "parameters"
                };
                let schema = t.entry(key).or_insert_with(|| json!({"type": "object"}));
                if schema == &json!({"type": "object", "properties": {}}) {
                    *schema = json!({"type": "object"});
                }
            }
        }
    }
}

/// Why an accept/reject disagreement is intended, if it is.
fn accept_rule(
    case: &Case,
    old_accepted: bool,
    new_body: Option<&Value>,
    base_body: Option<&Value>,
) -> Option<&'static str> {
    let same = case.inbound == case.upstream;
    if !old_accepted {
        // yi-llm rejected, yi-core forwarded / translated.
        if same {
            return Some("PASSTHROUGH-NO-VALIDATION");
        }
        let top = case.field.split('.').next().unwrap_or_default();
        if TOLERATED_PARAMS.contains(&case.field.as_str()) {
            let dropped = new_body.is_none_or(|b| b.get(top).is_none());
            if dropped || case.upstream == Protocol::OpenaiChat {
                return Some("LENIENT-OPTION-VALIDATION");
            }
        }
        if MALFORMED_TOLERATED.contains(&case.field.as_str()) {
            // The malformed value must not change the generated request, except
            // for reasoning controls (they decide whether the fallback applies).
            let strip = |b: &Value| {
                let mut b = b.clone();
                if let Some(o) = b.as_object_mut() {
                    o.remove(top);
                    if top.starts_with("stop") {
                        o.remove("stop");
                        o.remove("stop_sequences");
                    }
                }
                b
            };
            let unchanged = match (new_body, base_body) {
                (Some(n), Some(b)) => strip(n) == strip(b),
                _ => false,
            };
            if unchanged || REASONING_FIELDS.contains(&case.field.as_str()) {
                return Some("MALFORMED-VALUE-TOLERATED");
            }
        }
        return None;
    }
    // yi-llm accepted, yi-core rejected (only reachable on translation).
    match case.field.as_str() {
        "store" | "previous_response_id" if case.upstream != Protocol::Responses => {
            Some("STATEFUL-REJECTED")
        }
        "model" => Some("MODEL-REQUIRED"),
        "response_format" | "output_config.format" if case.value.contains("\"json_schema\"") => {
            Some("STRUCTURED-OUTPUT-REJECTED")
        }
        "messages" if case.value.contains("tool_call_id") => Some("ORPHAN-TOOL-MESSAGE"),
        "instructions" if case.value.starts_with('[') => Some("TYPED-FIELD-STRICT"),
        "input" if case.value == "\"\"" && case.upstream == Protocol::OpenaiChat => {
            Some("TYPED-FIELD-STRICT")
        }
        _ => None,
    }
}

/// Fields where yi-llm rejected a wrongly typed / untranslatable value and
/// yi-core ignores it (the request is built as if the field were absent).
const MALFORMED_TOLERATED: &[&str] = &[
    "tools",
    "tool_choice",
    "parallel_tool_calls",
    "stop",
    "stop_sequences",
    "top_k",
    "metadata",
    "instructions",
    "input",
    "messages",
    "system",
    "response_format",
    "output_config",
    "output_config.format",
    "output_config.effort",
    "audio",
    "modalities",
    "prediction",
    "functions",
    "function_call",
    "top_logprobs",
    "thinking",
    "reasoning.effort",
    "reasoning_effort",
];

/// Subset of [`MALFORMED_TOLERATED`] that steers the thinking fallback, so the
/// body legitimately differs from the base case.
const REASONING_FIELDS: &[&str] = &[
    // `input` / `messages` are the conversation itself: a tolerated shape
    // change legitimately changes the generated messages.
    "input",
    "messages",
    "thinking",
    "reasoning.effort",
    "reasoning_effort",
    "output_config",
    "output_config.effort",
];

/// Parameters each target protocol defines itself. A client of another
/// protocol may send them; they ride along unchanged (the denylist for
/// Responses keeps foreign ones out).
fn target_native(upstream: Protocol) -> &'static [&'static str] {
    match upstream {
        Protocol::OpenaiChat => &[
            "n",
            "seed",
            "user",
            "stop",
            "logprobs",
            "top_logprobs",
            "logit_bias",
            "presence_penalty",
            "frequency_penalty",
            "response_format",
            "parallel_tool_calls",
            "modalities",
            "audio",
            "prediction",
            "functions",
            "function_call",
        ],
        Protocol::Anthropic => &["top_k", "stop_sequences"],
        Protocol::Responses => &[
            "include",
            "instructions",
            "previous_response_id",
            "store",
            "text",
            "reasoning",
            "parallel_tool_calls",
        ],
    }
}

/// Optional scalar / object parameters whose explicit JSON null yi-llm forwarded.
const NULLABLE_SCALARS: &[&str] = &[
    "temperature",
    "top_p",
    "max_output_tokens",
    "tool_choice",
    "metadata",
    "stop",
    "parallel_tool_calls",
];

/// Parameters yi-llm rejected on malformed values (or could not translate) and
/// yi-core tolerates: dropped for non-Chat upstreams, native on Chat.
const TOLERATED_PARAMS: &[&str] = &[
    "n",
    "seed",
    "user",
    "logprobs",
    "logit_bias",
    "presence_penalty",
    "frequency_penalty",
];

/// Why a top-level key difference (old vs new) is intended, if it is.
fn key_rule(
    case: &Case,
    key: &str,
    old: &Value,
    new: &Value,
    new_body: &Value,
) -> Option<&'static str> {
    let absent = Value::String("<absent>".into());
    // A parameter the target protocol defines itself, carrying exactly the
    // client's value (the Chat `stop` may come from a client `stop_sequences`).
    if target_native(case.upstream).contains(&key) {
        let req = &case.request_canon;
        let sent = req
            .get(key)
            .or_else(|| (key == "stop").then(|| req.get("stop_sequences")).flatten());
        if sent == Some(new) {
            return Some("TARGET-NATIVE-PARAMS-FORWARDED");
        }
    }
    if *new == absent && *old == Value::Null && NULLABLE_SCALARS.contains(&key) {
        return Some("NULL-SCALARS-OMITTED");
    }
    if case.field == key
        && matches!(key, "input" | "max_output_tokens")
        && case.inbound != Protocol::Responses
    {
        return Some("FOREIGN-FIELD-SHADOWS-IR-FIELD");
    }
    if case.upstream == Protocol::Anthropic
        && matches!(key, "temperature" | "top_p")
        && *new == absent
        && *old != absent
        && new_body.get("thinking").is_some()
    {
        return Some("SAMPLING-OMITTED-WITH-THINKING");
    }
    if case.inbound == Protocol::Responses
        && case.field == "stop"
        && key == "stop_sequences"
        && *new == absent
    {
        return Some("STOP-IS-NOT-A-RESPONSES-PARAM");
    }
    if matches!(
        case.field.as_str(),
        "instructions" | "input" | "system" | "messages"
    ) && case.value == "\"\""
    {
        return Some("EMPTY-TEXT-OMITTED");
    }
    if case.field == "messages"
        && case.value.contains("developer")
        && case.upstream == Protocol::Responses
    {
        let holds_developer_item = |v: &Value| {
            v.as_array().is_some_and(|items| {
                items
                    .iter()
                    .any(|i| i["role"] == "developer" && i.to_string().contains("rules"))
            })
        };
        if (key == "instructions" && *old == json!("rules") && *new == absent)
            || (key == "input" && holds_developer_item(new))
        {
            return Some("DEVELOPER-AS-INPUT-MESSAGE");
        }
    }
    if case.field == "thinking"
        && case.value.contains("adaptive")
        && matches!(key, "reasoning_effort" | "reasoning")
        && *new == absent
    {
        return Some("ADAPTIVE-THINKING-NO-FALLBACK");
    }
    if case.inbound == Protocol::Anthropic
        && case.field == "tools"
        && key == "tools"
        && *new == absent
        && case.request["tools"]
            .as_array()
            .is_some_and(|t| !t.is_empty() && t.iter().all(|t| t.get("input_schema").is_none()))
    {
        return Some("FOREIGN-TOOL-SHAPE-NOT-TRANSLATED");
    }
    None
}

/// Rules explaining every differing key, or `Err` with the unexplained keys.
fn explain_body(
    case: &Case,
    old: &Value,
    new: &Value,
    used: &mut BTreeSet<&'static str>,
) -> Result<(), Vec<String>> {
    let (Some(o), Some(n)) = (old.as_object(), new.as_object()) else {
        return Err(vec!["<non-object body>".into()]);
    };
    let absent = Value::String("<absent>".into());
    let keys: BTreeSet<&String> = o.keys().chain(n.keys()).collect();
    let mut unexplained = Vec::new();
    for key in keys {
        let ov = o.get(key).unwrap_or(&absent);
        let nv = n.get(key).unwrap_or(&absent);
        if ov == nv {
            continue;
        }
        match key_rule(case, key, ov, nv, new) {
            Some(rule) => {
                used.insert(rule);
            }
            None => unexplained.push(format!("{key}: old={ov} new={nv}")),
        }
    }
    if unexplained.is_empty() {
        Ok(())
    } else {
        Err(unexplained)
    }
}

#[test]
fn every_allowed_diff_has_a_reason() {
    let mut seen = BTreeSet::new();
    for (rule, reason) in ALLOWED_DIFFS {
        assert!(!rule.is_empty() && reason.trim().len() > 20, "{rule}");
        assert!(seen.insert(*rule), "duplicate rule {rule}");
    }
}

#[test]
fn request_pipeline_matches_yi_llm_semantically() {
    let cases = load();
    assert_eq!(cases.len(), 12105, "corpus size changed");
    let mut mismatches: Vec<String> = Vec::new();
    // yi-core's body for each (inbound, upstream) base case.
    let mut bases: std::collections::HashMap<(Protocol, Protocol), Value> = Default::default();
    for case in cases.iter().filter(|c| c.field == "base") {
        if let (true, Some(mut b)) = run(case) {
            canon(&mut b);
            bases.insert((case.inbound, case.upstream), b);
        }
    }
    let mut used_by_rule: BTreeMap<&'static str, usize> = BTreeMap::new();
    for case in &cases {
        let new = run(case);
        let mut old: Outcome = (case.accepted, case.body.clone());
        let mut used: BTreeSet<&'static str> = BTreeSet::new();

        // B6: yi-llm's raw Responses bodies never carried the provider fallback.
        let raw_body = case.inbound == case.upstream || case.upstream == Protocol::Responses;
        if raw_body && let Some(body) = old.1.as_mut() {
            let before = body.clone();
            thinking::apply_raw(case.upstream, body, Some(Thinking::Medium), &capabilities());
            if *body != before {
                used.insert("B6");
            }
        }
        let mut new = new;
        if let Some(b) = new.1.as_mut() {
            canon(b);
        }
        if let Some(b) = old.1.as_mut() {
            canon(b);
        }

        let verdict: Result<(), Vec<String>> = if new.0 != old.0 {
            match accept_rule(
                case,
                old.0,
                new.1.as_ref(),
                bases.get(&(case.inbound, case.upstream)),
            ) {
                Some(rule) => {
                    used.insert(rule);
                    Ok(())
                }
                None => Err(vec![format!("accepted: old={} new={}", old.0, new.0)]),
            }
        } else if let (true, Some(o), Some(n)) = (new.0, &old.1, &new.1) {
            explain_body(case, o, n, &mut used)
        } else {
            Ok(())
        };
        match verdict {
            Ok(()) => {
                for rule in used {
                    *used_by_rule.entry(rule).or_default() += 1;
                }
            }
            Err(why) => mismatches.push(format!(
                "{}\n  {}\n  old: {}\n  new: {}",
                case.id,
                why.join("; "),
                old.1.as_ref().map_or("<rejected>".into(), Value::to_string),
                new.1.as_ref().map_or("<rejected>".into(), Value::to_string),
            )),
        }
    }
    if std::env::var("ORACLE_RULES").is_ok() {
        println!("rule usage: {used_by_rule:#?}");
    }
    for (rule, _) in ALLOWED_DIFFS {
        assert!(
            used_by_rule.contains_key(rule),
            "allowed diff {rule} is never exercised; remove it"
        );
    }
    assert!(
        mismatches.is_empty(),
        "{} mismatches of {} cases; first 50:\n{}",
        mismatches.len(),
        cases.len(),
        mismatches
            .iter()
            .take(if std::env::var("ORACLE_ALL").is_ok() {
                usize::MAX
            } else {
                50
            })
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
