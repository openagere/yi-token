//! SQLite side of the trajectory writer: the step linker's [`LinkStore`] over the
//! trajectory database, the in-memory tail index in front of `steps.tail_fp`,
//! and the row shapes the writer inserts. Everything here runs on the writer thread.

use std::collections::{HashMap, HashSet, VecDeque};
use std::str::FromStr;

use rusqlite::{Connection, OptionalExtension, ToSql, params, params_from_iter};
use serde_json::Value;

use crate::domain::{Client, KeySource, RouteMode, SessionSummary};
use crate::error::{CoreError, CoreResult};
use crate::storage::trajectory::blobs;
use crate::trajectory::link::{LinkStore, OpenCall, TailEntry, ThreadKind, prompt_fp};
use crate::trajectory::normalize::NormalizedItem;
use crate::trajectory::record::StopReason;

/// Tails kept in memory in front of `steps.tail_fp` (spec 4.3).
pub const TAIL_CAPACITY: usize = 2000;

/// Value of `sessions.client` for requests on the global route (no terminal client).
pub const GLOBAL_CLIENT: &str = "api";

/// Recently written step tails, keyed by their tail fingerprint. Evicts the
/// least recently inserted entry once full. Lookups do not refresh recency.
#[derive(Default)]
pub struct TailIndex {
    entries: HashMap<[u8; 32], TailEntry>,
    order: VecDeque<[u8; 32]>,
}

impl TailIndex {
    pub fn get(&self, fp: &[u8; 32]) -> Option<&TailEntry> {
        self.entries.get(fp)
    }

    /// Inserts or replaces the entry for `fp`; a replacement becomes the newest.
    pub fn insert(&mut self, fp: [u8; 32], entry: TailEntry) {
        if self.entries.insert(fp, entry).is_some()
            && let Some(pos) = self.order.iter().position(|k| *k == fp)
        {
            self.order.remove(pos);
        }
        self.order.push_back(fp);
        while self.order.len() > TAIL_CAPACITY {
            if let Some(old) = self.order.pop_front() {
                self.entries.remove(&old);
            }
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    /// Drops every entry whose session is one of `session_ids`.
    pub fn remove_sessions(&mut self, session_ids: &[String]) {
        if session_ids.is_empty() {
            return;
        }
        let gone: HashSet<&str> = session_ids.iter().map(String::as_str).collect();
        self.entries
            .retain(|_, entry| !gone.contains(entry.session_id.as_str()));
        self.order.retain(|fp| self.entries.contains_key(fp));
    }
}

/// Read-only linker view over the trajectory database and the tail index.
pub struct SqliteLinkStore<'a> {
    pub conn: &'a Connection,
    pub tails: &'a TailIndex,
}

impl LinkStore for SqliteLinkStore<'_> {
    fn session_by_key(&self, client: Option<Client>, key: &str) -> Option<String> {
        logged(
            "session_by_key",
            self.conn
                .query_row(
                    "SELECT id FROM sessions WHERE client = ?1 AND client_key = ?2",
                    params![client_column(client), key],
                    |r| r.get(0),
                )
                .optional()
                .map_err(Into::into),
        )
        .flatten()
    }

    fn tail_by_fp(&self, fp: &[u8; 32]) -> Option<TailEntry> {
        if let Some(entry) = self.tails.get(fp) {
            return Some(entry.clone());
        }
        logged("tail_by_fp", self.db_tail(fp)).flatten()
    }

    fn open_tool_calls(&self, session_id: &str) -> Vec<OpenCall> {
        logged("open_tool_calls", self.db_open_calls(session_id)).unwrap_or_default()
    }

    fn thread_kind(&self, thread_id: &str) -> Option<ThreadKind> {
        logged(
            "thread_kind",
            self.conn
                .query_row("SELECT kind FROM threads WHERE id = ?1", [thread_id], |r| {
                    r.get::<_, String>(0)
                })
                .optional()
                .map_err(Into::into),
        )
        .flatten()
        .and_then(|k| thread_kind_from_column(&k))
    }

    fn session_has_main(&self, session_id: &str) -> bool {
        logged(
            "session_has_main",
            self.conn
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM threads WHERE session_id = ?1 AND kind = 'main')",
                    [session_id],
                    |r| r.get::<_, bool>(0),
                )
                .map_err(Into::into),
        )
        .unwrap_or(false)
    }
}

impl SqliteLinkStore<'_> {
    /// Most recently requested step whose tail matches `fp` (spec 4.3).
    fn db_tail(&self, fp: &[u8; 32]) -> CoreResult<Option<TailEntry>> {
        let row = self
            .conn
            .query_row(
                "SELECT s.id, s.session_id, s.thread_id, s.requested_at, s.turn_no, s.depth, \
                 s.client_turn_id, se.client, \
                 s.prefix_len + (SELECT COUNT(*) FROM step_items i WHERE i.step_id = s.id) \
                 FROM steps s JOIN sessions se ON se.id = s.session_id \
                 WHERE s.tail_fp = ?1 ORDER BY s.requested_at DESC, s.seq DESC LIMIT 1",
                [fp_hex(fp)],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, i64>(5)?,
                        r.get::<_, Option<String>>(6)?,
                        r.get::<_, String>(7)?,
                        r.get::<_, i64>(8)?,
                    ))
                },
            )
            .optional()?;
        let Some((step_id, session_id, thread_id, last_at, turn_no, depth, turn, client, len)) =
            row
        else {
            return Ok(None);
        };
        Ok(Some(TailEntry {
            step_id,
            session_id,
            thread_id,
            client: client_from_column(&client),
            last_at,
            tail_len: usize::try_from(len).unwrap_or(0),
            turn_no: u32::try_from(turn_no).unwrap_or(0),
            depth: u32::try_from(depth).unwrap_or(0),
            client_turn_id: turn,
        }))
    }

    /// Tool calls with no result in the session yet. Only a `Task` call can
    /// spawn a subagent, so only those carry a prompt fingerprint.
    fn db_open_calls(&self, session_id: &str) -> CoreResult<Vec<OpenCall>> {
        let mut stmt = self.conn.prepare(
            "SELECT i.call_id, i.tool_name, i.blob, s.thread_id FROM step_items i \
             JOIN steps s ON s.id = i.step_id \
             WHERE s.session_id = ?1 AND i.side = 'output' AND i.kind = 'tool_call' \
             AND i.call_id IS NOT NULL \
             AND NOT EXISTS (SELECT 1 FROM step_items r JOIN steps rs ON rs.id = r.step_id \
                 WHERE rs.session_id = ?1 AND r.kind = 'tool_result' AND r.call_id = i.call_id) \
             ORDER BY s.seq, i.ord",
        )?;
        let rows = stmt
            .query_map([session_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut calls = Vec::with_capacity(rows.len());
        for (call_id, tool, hash, thread_id) in rows {
            let prompt_fp = if tool.as_deref() == Some("Task") {
                task_prompt_fp(self.conn, &hash)?
            } else {
                None
            };
            calls.push(OpenCall {
                thread_id,
                call_id,
                prompt_fp,
            });
        }
        Ok(calls)
    }
}

/// Fingerprint of the `prompt` argument of a stored `Task` call, read from its blob.
fn task_prompt_fp(conn: &Connection, hash: &str) -> CoreResult<Option<[u8; 32]>> {
    let Some(stored) = blobs::get(conn, hash)? else {
        return Ok(None);
    };
    let Ok(item) = serde_json::from_str::<Value>(&stored.json) else {
        return Ok(None);
    };
    let prompt = item
        .get("arguments")
        .and_then(Value::as_str)
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .and_then(|args| {
            args.get("prompt")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    Ok(prompt.map(|text| prompt_fp(&text)))
}

/// Logs a read failure and substitutes `None` (the linker then treats the
/// lookup as a miss), so a broken read never fails the request path.
fn logged<T>(what: &str, result: CoreResult<T>) -> Option<T> {
    match result {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!("trajectory lookup {what} failed: {e}");
            None
        }
    }
}

// ------------------------------------------------------------ column values

/// The `sessions.client` value for a route.
pub fn client_column(client: Option<Client>) -> &'static str {
    client.map_or(GLOBAL_CLIENT, |c| c.as_str())
}

fn client_from_column(s: &str) -> Option<Client> {
    if s == GLOBAL_CLIENT {
        None
    } else {
        Client::from_str(s).ok()
    }
}

pub fn key_source_column(source: KeySource) -> &'static str {
    match source {
        KeySource::Client => "client",
        KeySource::Chain => "chain",
        KeySource::New => "new",
    }
}

pub fn thread_kind_column(kind: ThreadKind) -> &'static str {
    match kind {
        ThreadKind::Main => "main",
        ThreadKind::Subagent => "subagent",
        ThreadKind::Side => "side",
        ThreadKind::Pending => "pending",
    }
}

fn thread_kind_from_column(s: &str) -> Option<ThreadKind> {
    match s {
        "main" => Some(ThreadKind::Main),
        "subagent" => Some(ThreadKind::Subagent),
        "side" => Some(ThreadKind::Side),
        "pending" => Some(ThreadKind::Pending),
        _ => None,
    }
}

pub fn route_mode_column(mode: Option<RouteMode>) -> Option<&'static str> {
    mode.map(|m| match m {
        RouteMode::Fixed => "fixed",
        RouteMode::Auto => "auto",
    })
}

pub fn stop_reason_column(reason: StopReason) -> &'static str {
    match reason {
        StopReason::End => "end",
        StopReason::ToolUse => "tool_use",
        StopReason::MaxTokens => "max_tokens",
        StopReason::Refusal => "refusal",
        StopReason::Error => "error",
        StopReason::Cancelled => "cancelled",
        StopReason::Unknown => "unknown",
    }
}

/// Lowercase hex, the stored form of a tail fingerprint.
pub fn fp_hex(fp: &[u8; 32]) -> String {
    fp.iter().map(|b| format!("{b:02x}")).collect()
}

// --------------------------------------------------------------- row writes

/// Inserts a new session with zeroed totals; the title and totals follow in separate updates.
pub fn insert_session(
    conn: &Connection,
    id: &str,
    client: Option<Client>,
    client_key: Option<&str>,
    source: KeySource,
    started_at: i64,
) -> CoreResult<()> {
    conn.execute(
        "INSERT INTO sessions (id, client, client_key, key_source, title, started_at, last_at) \
         VALUES (?1, ?2, ?3, ?4, '', ?5, ?5)",
        params![
            id,
            client_column(client),
            client_key,
            key_source_column(source),
            started_at
        ],
    )?;
    Ok(())
}

/// Sets the title if it is still empty.
pub fn fill_session_title(conn: &Connection, session_id: &str, title: &str) -> CoreResult<()> {
    conn.execute(
        "UPDATE sessions SET title = ?2 WHERE id = ?1 AND title = ''",
        params![session_id, title],
    )?;
    Ok(())
}

/// Totals added by one step.
pub struct SessionDelta {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub bytes: i64,
    pub at: i64,
}

pub fn add_session_totals(conn: &Connection, session_id: &str, d: &SessionDelta) -> CoreResult<()> {
    conn.execute(
        "UPDATE sessions SET step_count = step_count + 1, input_tokens = input_tokens + ?2, \
         output_tokens = output_tokens + ?3, bytes = bytes + ?4, last_at = MAX(last_at, ?5) \
         WHERE id = ?1",
        params![session_id, d.input_tokens, d.output_tokens, d.bytes, d.at],
    )?;
    Ok(())
}

pub struct ThreadRow<'a> {
    pub id: &'a str,
    pub session_id: &'a str,
    pub kind: ThreadKind,
    pub parent_thread_id: Option<&'a str>,
    pub spawn_call_id: Option<&'a str>,
    pub reset: bool,
    pub started_at: i64,
}

pub fn insert_thread(conn: &Connection, t: &ThreadRow) -> CoreResult<()> {
    conn.execute(
        "INSERT INTO threads (id, session_id, kind, parent_thread_id, spawn_call_id, reset, started_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            t.id,
            t.session_id,
            thread_kind_column(t.kind),
            t.parent_thread_id,
            t.spawn_call_id,
            t.reset,
            t.started_at
        ],
    )?;
    Ok(())
}

/// Makes a pending thread the session's main thread, when the session has none.
pub fn promote_to_main(conn: &Connection, thread_id: &str, session_id: &str) -> CoreResult<()> {
    conn.execute(
        "UPDATE threads SET kind = 'main' WHERE id = ?1 AND kind = 'pending' AND NOT EXISTS \
         (SELECT 1 FROM threads WHERE session_id = ?2 AND kind = 'main')",
        params![thread_id, session_id],
    )?;
    Ok(())
}

/// One `steps` row. `seq` is assigned by the insert.
pub struct StepRow<'a> {
    pub id: &'a str,
    pub session_id: &'a str,
    pub thread_id: &'a str,
    pub parent_step_id: Option<&'a str>,
    pub prefix_len: i64,
    pub checkpoint: bool,
    pub depth: i64,
    pub turn_no: i64,
    pub client_turn_id: Option<&'a str>,
    pub requested_at: i64,
    pub first_token_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub status: Option<i64>,
    pub stop_reason: &'a str,
    pub error: Option<&'a str>,
    pub inbound_protocol: &'a str,
    pub upstream_protocol: &'a str,
    pub provider_name: &'a str,
    pub model: &'a str,
    pub upstream_model: &'a str,
    pub route_mode: Option<&'a str>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub reasoning_tokens: i64,
    pub system_blob: Option<&'a str>,
    pub tools_blob: Option<&'a str>,
    pub tail_fp: &'a str,
    pub capture: &'a str,
}

pub fn insert_step(conn: &Connection, s: &StepRow) -> CoreResult<()> {
    // The counter only grows: deleting steps never gives a sequence number back (v2 migration).
    conn.execute(
        "UPDATE meta SET value = value + 1 WHERE key = 'last_seq'",
        [],
    )?;
    conn.execute(
        "INSERT INTO steps (id, seq, session_id, thread_id, parent_step_id, prefix_len, checkpoint, \
         depth, turn_no, client_turn_id, requested_at, first_token_at, finished_at, status, \
         stop_reason, error, inbound_protocol, upstream_protocol, provider_name, model, \
         upstream_model, route_mode, input_tokens, output_tokens, cached_tokens, reasoning_tokens, \
         system_blob, tools_blob, tail_fp, capture) \
         VALUES (?1, (SELECT value FROM meta WHERE key = 'last_seq'), ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, \
         ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29)",
        params![
            s.id,
            s.session_id,
            s.thread_id,
            s.parent_step_id,
            s.prefix_len,
            s.checkpoint,
            s.depth,
            s.turn_no,
            s.client_turn_id,
            s.requested_at,
            s.first_token_at,
            s.finished_at,
            s.status,
            s.stop_reason,
            s.error,
            s.inbound_protocol,
            s.upstream_protocol,
            s.provider_name,
            s.model,
            s.upstream_model,
            s.route_mode,
            s.input_tokens,
            s.output_tokens,
            s.cached_tokens,
            s.reasoning_tokens,
            s.system_blob,
            s.tools_blob,
            s.tail_fp,
            s.capture
        ],
    )?;
    Ok(())
}

/// One `step_items` row: the item at `ord` on `side` of `step_id`.
pub fn insert_item(
    conn: &Connection,
    step_id: &str,
    side: &str,
    ord: i64,
    item: &NormalizedItem,
) -> CoreResult<()> {
    conn.execute(
        "INSERT INTO step_items (step_id, side, ord, blob, kind, role, call_id, tool_name) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            step_id,
            side,
            ord,
            item.hash,
            blobs::kind_name(item.kind),
            item.role,
            item.call_id,
            item.tool_name
        ],
    )?;
    Ok(())
}

// ------------------------------------------------------------- read side

/// One `steps` row as the readers use it.
#[derive(Debug, Clone)]
pub struct StepRead {
    pub id: String,
    pub seq: i64,
    pub session_id: String,
    pub thread_id: String,
    pub prefix_len: i64,
    pub checkpoint: bool,
    pub turn_no: i64,
    pub requested_at: i64,
    pub first_token_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub status: Option<i64>,
    pub stop_reason: String,
    pub error: Option<String>,
    pub inbound_protocol: String,
    pub upstream_protocol: String,
    pub provider_name: String,
    pub model: String,
    pub upstream_model: String,
    pub route_mode: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub reasoning_tokens: i64,
    pub system_blob: Option<String>,
    pub tools_blob: Option<String>,
    pub options_json: String,
    pub capture: String,
    /// Stored input items: the request input from `prefix_len` on (all of it for a checkpoint).
    pub input_count: i64,
    pub output_count: i64,
}

const STEP_SELECT: &str = "SELECT s.id, s.seq, s.session_id, s.thread_id, s.prefix_len, \
    s.checkpoint, s.turn_no, s.requested_at, s.first_token_at, s.finished_at, s.status, \
    s.stop_reason, s.error, s.inbound_protocol, s.upstream_protocol, s.provider_name, s.model, \
    s.upstream_model, s.route_mode, s.input_tokens, s.output_tokens, s.cached_tokens, \
    s.reasoning_tokens, s.system_blob, s.tools_blob, s.options_json, s.capture, \
    (SELECT COUNT(*) FROM step_items i WHERE i.step_id = s.id AND i.side = 'input'), \
    (SELECT COUNT(*) FROM step_items i WHERE i.step_id = s.id AND i.side = 'output') \
    FROM steps s";

fn step_from_row(r: &rusqlite::Row) -> rusqlite::Result<StepRead> {
    Ok(StepRead {
        id: r.get(0)?,
        seq: r.get(1)?,
        session_id: r.get(2)?,
        thread_id: r.get(3)?,
        prefix_len: r.get(4)?,
        checkpoint: r.get(5)?,
        turn_no: r.get(6)?,
        requested_at: r.get(7)?,
        first_token_at: r.get(8)?,
        finished_at: r.get(9)?,
        status: r.get(10)?,
        stop_reason: r.get(11)?,
        error: r.get(12)?,
        inbound_protocol: r.get(13)?,
        upstream_protocol: r.get(14)?,
        provider_name: r.get(15)?,
        model: r.get(16)?,
        upstream_model: r.get(17)?,
        route_mode: r.get(18)?,
        input_tokens: r.get(19)?,
        output_tokens: r.get(20)?,
        cached_tokens: r.get(21)?,
        reasoning_tokens: r.get(22)?,
        system_blob: r.get(23)?,
        tools_blob: r.get(24)?,
        options_json: r.get(25)?,
        capture: r.get(26)?,
        input_count: r.get(27)?,
        output_count: r.get(28)?,
    })
}

/// One stored item (`step_items` row) before its blob is loaded.
#[derive(Debug, Clone)]
pub struct ItemRead {
    pub step_id: String,
    /// `input` or `output`.
    pub side: String,
    pub ord: i64,
    pub kind: String,
    pub role: Option<String>,
    pub call_id: Option<String>,
    pub tool_name: Option<String>,
    pub blob: String,
}

/// A stored item with its blob decompressed.
#[derive(Debug, Clone)]
pub struct ItemData {
    pub read: ItemRead,
    /// Canonical JSON of the item (`null` when the blob is missing).
    pub json: String,
    pub truncated: bool,
}

fn item_from_row(r: &rusqlite::Row) -> rusqlite::Result<ItemRead> {
    Ok(ItemRead {
        step_id: r.get(0)?,
        side: r.get(1)?,
        ord: r.get(2)?,
        kind: r.get(3)?,
        role: r.get(4)?,
        call_id: r.get(5)?,
        tool_name: r.get(6)?,
        blob: r.get(7)?,
    })
}

const ITEM_COLUMNS: &str =
    "i.step_id, i.side, i.ord, i.kind, i.role, i.call_id, i.tool_name, i.blob";

/// Filters of the session list. `after` is the keyset cursor: the `(last_at, id)` of the
/// last row of the previous page.
#[derive(Debug, Clone, Default)]
pub struct SessionQuery {
    pub client: Option<Client>,
    /// Lowercase substring matched against the title.
    pub title: Option<String>,
    pub model: Option<String>,
    pub start_ms: Option<i64>,
    /// Exclusive upper bound on `last_at`.
    pub end_ms: Option<i64>,
    pub after: Option<(i64, String)>,
}

struct SessionRow {
    id: String,
    client: String,
    title: String,
    key_source: String,
    started_at: i64,
    last_at: i64,
    step_count: i64,
    input_tokens: i64,
    output_tokens: i64,
    bytes: i64,
}

const SESSION_SELECT: &str = "SELECT se.id, se.client, se.title, se.key_source, se.started_at, \
    se.last_at, se.step_count, se.input_tokens, se.output_tokens, se.bytes FROM sessions se";

fn session_from_row(r: &rusqlite::Row) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: r.get(0)?,
        client: r.get(1)?,
        title: r.get(2)?,
        key_source: r.get(3)?,
        started_at: r.get(4)?,
        last_at: r.get(5)?,
        step_count: r.get(6)?,
        input_tokens: r.get(7)?,
        output_tokens: r.get(8)?,
        bytes: r.get(9)?,
    })
}

/// Sessions matching `q`, newest first, at most `limit` rows. `live` is left false.
pub fn list_sessions(
    conn: &Connection,
    q: &SessionQuery,
    limit: u32,
) -> CoreResult<Vec<SessionSummary>> {
    let title = q.title.as_deref().map(str::to_lowercase);
    let (after_at, after_id) = match &q.after {
        Some((at, id)) => (Some(*at), Some(id.as_str())),
        None => (None, None),
    };
    let sql = format!(
        "{SESSION_SELECT} WHERE (?1 IS NULL OR se.client = ?1) \
         AND (?2 IS NULL OR instr(lower(se.title), ?2) > 0) \
         AND (?3 IS NULL OR EXISTS (SELECT 1 FROM steps st WHERE st.session_id = se.id AND st.model = ?3)) \
         AND (?4 IS NULL OR se.last_at >= ?4) AND (?5 IS NULL OR se.last_at < ?5) \
         AND (?6 IS NULL OR se.last_at < ?6 OR (se.last_at = ?6 AND se.id < ?7)) \
         ORDER BY se.last_at DESC, se.id DESC LIMIT ?8"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(
            params![
                q.client.map(|c| c.as_str()),
                title,
                q.model.as_deref(),
                q.start_ms,
                q.end_ms,
                after_at,
                after_id,
                i64::from(limit)
            ],
            session_from_row,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter().map(|row| summary(conn, row)).collect()
}

/// One session's summary, or `None` when it does not exist. `live` is left false.
pub fn session_summary(conn: &Connection, id: &str) -> CoreResult<Option<SessionSummary>> {
    let sql = format!("{SESSION_SELECT} WHERE se.id = ?1");
    let row = conn.query_row(&sql, [id], session_from_row).optional()?;
    row.map(|r| summary(conn, r)).transpose()
}

fn summary(conn: &Connection, row: SessionRow) -> CoreResult<SessionSummary> {
    let models = session_models(conn, &row.id)?;
    Ok(SessionSummary {
        client: client_from_column(&row.client),
        key_source: key_source_from_column(&row.key_source)?,
        started_at: row.started_at,
        last_at: row.last_at,
        step_count: u32::try_from(row.step_count).unwrap_or(u32::MAX),
        input_tokens: non_negative(row.input_tokens),
        output_tokens: non_negative(row.output_tokens),
        bytes: non_negative(row.bytes),
        models,
        live: false,
        id: row.id,
        title: row.title,
    })
}

/// Distinct exposed model names of a session, in first-use order.
pub fn session_models(conn: &Connection, session_id: &str) -> CoreResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT model FROM steps WHERE session_id = ?1 GROUP BY model ORDER BY MIN(seq)",
    )?;
    let models = stmt
        .query_map([session_id], |r| r.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(models)
}

/// Whether a session with this id exists.
pub fn session_exists(conn: &Connection, id: &str) -> CoreResult<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sessions WHERE id = ?1)",
        [id],
        |r| r.get(0),
    )?)
}

/// Number of sessions.
pub fn session_count(conn: &Connection) -> CoreResult<u32> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))?;
    Ok(u32::try_from(n).unwrap_or(u32::MAX))
}

/// A thread with its aggregates.
#[derive(Debug, Clone)]
pub struct ThreadRead {
    pub id: String,
    /// `main`, `subagent`, `side` or `pending`.
    pub kind: String,
    pub parent_thread_id: Option<String>,
    pub spawn_call_id: Option<String>,
    pub reset: bool,
    pub started_at: i64,
    pub step_count: i64,
    /// Some step of the thread carried tool definitions or tool calls.
    pub has_tools: bool,
}

/// The threads of a session, oldest first.
pub fn session_threads(conn: &Connection, session_id: &str) -> CoreResult<Vec<ThreadRead>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.kind, t.parent_thread_id, t.spawn_call_id, t.reset, t.started_at, \
         (SELECT COUNT(*) FROM steps s WHERE s.thread_id = t.id), \
         EXISTS (SELECT 1 FROM steps s WHERE s.thread_id = t.id AND (s.tools_blob IS NOT NULL \
             OR EXISTS (SELECT 1 FROM step_items i WHERE i.step_id = s.id AND i.side = 'output' \
                 AND i.kind = 'tool_call'))) \
         FROM threads t WHERE t.session_id = ?1 ORDER BY t.started_at, t.rowid",
    )?;
    let rows = stmt
        .query_map([session_id], |r| {
            Ok(ThreadRead {
                id: r.get(0)?,
                kind: r.get(1)?,
                parent_thread_id: r.get(2)?,
                spawn_call_id: r.get(3)?,
                reset: r.get(4)?,
                started_at: r.get(5)?,
                step_count: r.get(6)?,
                has_tools: r.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The thread to show: `requested` when it belongs to the session; otherwise the main
/// thread, or the thread with the most steps when there is no main thread. `None` when no
/// such thread exists.
pub fn resolve_thread(
    conn: &Connection,
    session_id: &str,
    requested: Option<&str>,
) -> CoreResult<Option<String>> {
    if let Some(id) = requested {
        return Ok(conn
            .query_row(
                "SELECT id FROM threads WHERE id = ?1 AND session_id = ?2",
                params![id, session_id],
                |r| r.get(0),
            )
            .optional()?);
    }
    Ok(conn
        .query_row(
            "SELECT t.id FROM threads t WHERE t.session_id = ?1 \
             ORDER BY (t.kind = 'main') DESC, \
             (SELECT COUNT(*) FROM steps s WHERE s.thread_id = t.id) DESC, \
             t.started_at ASC, t.rowid ASC LIMIT 1",
            [session_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// Up to `limit` turn numbers of a thread below `before` (exclusive), newest first.
pub fn turn_numbers(
    conn: &Connection,
    thread_id: &str,
    before: Option<i64>,
    limit: i64,
) -> CoreResult<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT turn_no FROM steps WHERE thread_id = ?1 \
         AND (?2 IS NULL OR turn_no < ?2) ORDER BY turn_no DESC LIMIT ?3",
    )?;
    let turns = stmt
        .query_map(params![thread_id, before, limit], |r| r.get(0))?
        .collect::<Result<Vec<i64>, _>>()?;
    Ok(turns)
}

/// Steps of a thread whose turn lies in `lo..=hi`, in `seq` order.
pub fn thread_steps(
    conn: &Connection,
    thread_id: &str,
    lo: i64,
    hi: i64,
) -> CoreResult<Vec<StepRead>> {
    let sql = format!(
        "{STEP_SELECT} WHERE s.thread_id = ?1 AND s.turn_no BETWEEN ?2 AND ?3 ORDER BY s.seq"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![thread_id, lo, hi], step_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Items of the steps of a thread in `lo..=hi`, in step order, input before output.
pub fn thread_items(
    conn: &Connection,
    thread_id: &str,
    lo: i64,
    hi: i64,
) -> CoreResult<Vec<ItemRead>> {
    let sql = format!(
        "SELECT {ITEM_COLUMNS} FROM step_items i JOIN steps s ON s.id = i.step_id \
         WHERE s.thread_id = ?1 AND s.turn_no BETWEEN ?2 AND ?3 ORDER BY s.seq, i.side, i.ord"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![thread_id, lo, hi], item_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// All stored items of one step, input before output.
pub fn step_items(conn: &Connection, step_id: &str) -> CoreResult<Vec<ItemRead>> {
    let sql = format!(
        "SELECT {ITEM_COLUMNS} FROM step_items i WHERE i.step_id = ?1 ORDER BY i.side, i.ord"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([step_id], item_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Session-wide request number of every step: 1-based, ordered by `seq`.
pub fn step_numbers(conn: &Connection, session_id: &str) -> CoreResult<HashMap<String, u32>> {
    let mut stmt = conn
        .prepare("SELECT id, ROW_NUMBER() OVER (ORDER BY seq) FROM steps WHERE session_id = ?1")?;
    let pairs = stmt
        .query_map([session_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(pairs
        .into_iter()
        .map(|(id, n)| (id, u32::try_from(n).unwrap_or(u32::MAX)))
        .collect())
}

/// The step before `seq` in the same thread, if any.
pub fn previous_step(conn: &Connection, thread_id: &str, seq: i64) -> CoreResult<Option<StepRead>> {
    let sql =
        format!("{STEP_SELECT} WHERE s.thread_id = ?1 AND s.seq < ?2 ORDER BY s.seq DESC LIMIT 1");
    Ok(conn
        .query_row(&sql, params![thread_id, seq], step_from_row)
        .optional()?)
}

/// A step by id, or `None`.
pub fn step_by_id(conn: &Connection, id: &str) -> CoreResult<Option<StepRead>> {
    let sql = format!("{STEP_SELECT} WHERE s.id = ?1");
    Ok(conn.query_row(&sql, [id], step_from_row).optional()?)
}

/// A tool-result input item, with the requested time of the step that carries it.
#[derive(Debug, Clone)]
pub struct ResultRead {
    pub requested_at: i64,
    pub item: ItemRead,
}

/// Tool-result input items for `call_ids` in the session, in step order. A result can
/// appear in several later requests, so callers keep the first one per call.
pub fn tool_results(
    conn: &Connection,
    session_id: &str,
    call_ids: &[String],
) -> CoreResult<Vec<ResultRead>> {
    if call_ids.is_empty() {
        return Ok(Vec::new());
    }
    let marks = (0..call_ids.len())
        .map(|i| format!("?{}", i + 2))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT s.requested_at, {ITEM_COLUMNS} FROM step_items i JOIN steps s ON s.id = i.step_id \
         WHERE s.session_id = ?1 AND i.side = 'input' AND i.kind = 'tool_result' \
         AND i.call_id IN ({marks}) ORDER BY s.seq, i.ord"
    );
    let mut values: Vec<&dyn ToSql> = Vec::with_capacity(call_ids.len() + 1);
    values.push(&session_id);
    for id in call_ids {
        values.push(id);
    }
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_from_iter(values), |r| {
            Ok(ResultRead {
                requested_at: r.get(0)?,
                item: ItemRead {
                    step_id: r.get(1)?,
                    side: r.get(2)?,
                    ord: r.get(3)?,
                    kind: r.get(4)?,
                    role: r.get(5)?,
                    call_id: r.get(6)?,
                    tool_name: r.get(7)?,
                    blob: r.get(8)?,
                },
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// `(spawn_call_id, thread_id)` of every subagent thread in the session.
pub fn spawn_links(conn: &Connection, session_id: &str) -> CoreResult<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT spawn_call_id, id FROM threads WHERE session_id = ?1 AND spawn_call_id IS NOT NULL",
    )?;
    let links = stmt
        .query_map([session_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(links)
}

/// Step ids of the request context of `step_id`, from the checkpoint down to the step.
/// Walks `parent_step_id` and stops at the first checkpoint (spec 4.4, 5.1). One query.
pub fn context_chain(conn: &Connection, step_id: &str) -> CoreResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE chain(id, parent_step_id, checkpoint, depth) AS (
             SELECT id, parent_step_id, checkpoint, 0 FROM steps WHERE id = ?1
             UNION ALL
             SELECT s.id, s.parent_step_id, s.checkpoint, c.depth + 1
             FROM steps s JOIN chain c ON s.id = c.parent_step_id
             WHERE c.checkpoint = 0
         )
         SELECT id FROM chain ORDER BY depth DESC",
    )?;
    let ids = stmt
        .query_map([step_id], |r| r.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(ids)
}

/// Sessions with a step whose `seq` lies in `(since, upto]`. `upto` is a cursor read before
/// this call, so steps committed afterwards are left for the next call.
pub fn changed_sessions(conn: &Connection, since: i64, upto: i64) -> CoreResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT session_id FROM steps WHERE seq > ?1 AND seq <= ?2 ORDER BY session_id",
    )?;
    let ids = stmt
        .query_map(params![since, upto], |r| r.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(ids)
}

/// The last sequence number handed out. Monotonic: deletes do not lower it, so a change
/// cursor taken earlier stays valid (v2 migration).
pub fn last_seq(conn: &Connection) -> CoreResult<i64> {
    Ok(conn.query_row("SELECT value FROM meta WHERE key = 'last_seq'", [], |r| r.get(0))?)
}

/// Decompresses each item's blob. A missing blob yields `null` rather than failing the read.
pub fn load_items(conn: &Connection, rows: Vec<ItemRead>) -> CoreResult<Vec<ItemData>> {
    let mut cache: HashMap<String, (String, bool)> = HashMap::new();
    let mut out = Vec::with_capacity(rows.len());
    for read in rows {
        let loaded = match cache.get(&read.blob) {
            Some(hit) => hit.clone(),
            None => {
                let loaded = match blobs::get(conn, &read.blob)? {
                    Some(stored) => (stored.json, stored.truncated),
                    None => {
                        tracing::warn!(blob = %read.blob, "trajectory blob missing");
                        ("null".to_owned(), false)
                    }
                };
                cache.insert(read.blob.clone(), loaded.clone());
                loaded
            }
        };
        out.push(ItemData {
            json: loaded.0,
            truncated: loaded.1,
            read,
        });
    }
    Ok(out)
}

/// The decompressed body of one blob and its truncation flag, or `None` when absent.
pub fn blob_json(conn: &Connection, hash: &str) -> CoreResult<Option<(String, bool)>> {
    Ok(blobs::get(conn, hash)?.map(|b| (b.json, b.truncated)))
}

/// Parses a stored `key_source` column.
fn key_source_from_column(s: &str) -> CoreResult<KeySource> {
    match s {
        "client" => Ok(KeySource::Client),
        "chain" => Ok(KeySource::Chain),
        "new" => Ok(KeySource::New),
        other => Err(CoreError::Database(format!("未知的会话来源：{other}"))),
    }
}

fn non_negative(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(step: &str) -> TailEntry {
        TailEntry {
            step_id: step.into(),
            session_id: "s".into(),
            thread_id: "t".into(),
            client: None,
            last_at: 0,
            tail_len: 1,
            turn_no: 1,
            depth: 0,
            client_turn_id: None,
        }
    }

    #[test]
    fn tail_index_evicts_least_recently_inserted() {
        let mut idx = TailIndex::default();
        let key = |n: usize| {
            let mut k = [0u8; 32];
            k[0] = (n % 256) as u8;
            k[1] = (n / 256) as u8;
            k
        };
        for n in 0..TAIL_CAPACITY {
            idx.insert(key(n), entry(&format!("step{n}")));
        }
        assert!(idx.get(&key(0)).is_some());
        // Re-inserting key 0 makes it the newest; the next insert evicts key 1.
        idx.insert(key(0), entry("again"));
        idx.insert(key(TAIL_CAPACITY), entry("new"));
        assert!(idx.get(&key(1)).is_none());
        assert_eq!(idx.get(&key(0)).unwrap().step_id, "again");
        assert_eq!(idx.get(&key(TAIL_CAPACITY)).unwrap().step_id, "new");
    }

    fn seed_step(conn: &Connection, id: &str) -> CoreResult<()> {
        insert_session(conn, id, Some(Client::Codex), None, KeySource::New, 0)?;
        let thread = format!("t-{id}");
        insert_thread(
            conn,
            &ThreadRow {
                id: &thread,
                session_id: id,
                kind: ThreadKind::Main,
                parent_thread_id: None,
                spawn_call_id: None,
                reset: false,
                started_at: 0,
            },
        )?;
        let tail = format!("fp-{id}");
        insert_step(
            conn,
            &StepRow {
                id: &format!("st-{id}"),
                session_id: id,
                thread_id: &thread,
                parent_step_id: None,
                prefix_len: 0,
                checkpoint: true,
                depth: 0,
                turn_no: 1,
                client_turn_id: None,
                requested_at: 0,
                first_token_at: None,
                finished_at: None,
                status: None,
                stop_reason: "end",
                error: None,
                inbound_protocol: "anthropic",
                upstream_protocol: "anthropic",
                provider_name: "p",
                model: "m",
                upstream_model: "m",
                route_mode: None,
                input_tokens: 0,
                output_tokens: 0,
                cached_tokens: 0,
                reasoning_tokens: 0,
                system_blob: None,
                tools_blob: None,
                tail_fp: &tail,
                capture: "full",
            },
        )?;
        Ok(())
    }

    #[test]
    fn change_feed_is_bounded_by_the_cursor_read_first() {
        let tdb = crate::storage::trajectory::TrajectoryDb::open_in_memory().unwrap();
        tdb.0
            .with_conn(|conn| {
                seed_step(conn, "a")?;
                // The service reads the cursor first, then the changes up to it.
                let cursor = last_seq(conn)?;
                // A step committed after the cursor read gets a seq above the cursor.
                seed_step(conn, "b")?;

                assert_eq!(changed_sessions(conn, 0, cursor)?, vec!["a".to_string()]);
                // The next call, with the newer cursor, reports it.
                let next = last_seq(conn)?;
                assert!(next > cursor);
                assert_eq!(changed_sessions(conn, cursor, next)?, vec!["b".to_string()]);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn fingerprint_hex_is_lowercase_64_chars() {
        let mut fp = [0u8; 32];
        fp[0] = 0xab;
        fp[31] = 0x0f;
        let hex = fp_hex(&fp);
        assert_eq!(hex.len(), 64);
        assert!(hex.starts_with("ab") && hex.ends_with("0f"));
    }
}
