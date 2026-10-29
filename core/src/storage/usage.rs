//! Usage events: batched inserts and aggregated reports.

use super::{enum_str, parse_enum, query_as};
use crate::domain::{
    Client, LivePhase, LiveRequest, UsageClient, UsageDailyModel, UsageDay, UsageEvent,
    UsageGroupBy, UsageGroupRow, UsageModel, UsageRecord, UsageReport, UsageTotals,
};
use crate::error::CoreResult;
use rusqlite::{Connection, params};
use serde::Deserialize;

const RECENT_LIMIT: u32 = 100;

/// Report filter on UTC-millisecond timestamps: the window is `[start_ms, end_ms)`.
/// Day buckets in the report are computed in the local time zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageQuery {
    pub start_ms: i64,
    pub end_ms: i64,
    pub model: Option<String>,
    pub client: Option<Client>,
}

/// Inserts all events atomically (a savepoint, so it also nests in a transaction).
pub fn insert_batch(conn: &Connection, events: &[UsageEvent]) -> CoreResult<()> {
    if events.is_empty() {
        return Ok(());
    }
    conn.execute_batch("SAVEPOINT usage_batch")?;
    match insert_all(conn, events) {
        Ok(()) => {
            conn.execute_batch("RELEASE usage_batch")?;
            Ok(())
        }
        Err(e) => {
            conn.execute_batch("ROLLBACK TO usage_batch; RELEASE usage_batch")?;
            Err(e)
        }
    }
}

fn insert_all(conn: &Connection, events: &[UsageEvent]) -> CoreResult<()> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO usage (id, requested_at, client, route_mode, provider_name, protocol, model, \
           input_tokens, output_tokens, cached_tokens, reasoning_tokens, \
           step_id, session_id, ttft_ms, duration_ms, status) \
         VALUES (?11, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?12, ?13, ?14, ?15, ?16)",
    )?;
    for e in events {
        stmt.execute(params![
            e.requested_at,
            e.client.as_ref().map(enum_str).transpose()?,
            e.route_mode.as_ref().map(enum_str).transpose()?,
            e.provider_name,
            enum_str(&e.protocol)?,
            e.model,
            e.input_tokens,
            e.output_tokens,
            e.cached_tokens,
            e.reasoning_tokens,
            uuid::Uuid::new_v4().to_string(),
            e.step_id,
            e.session_id,
            e.ttft_ms,
            e.duration_ms,
            e.status,
        ])?;
    }
    Ok(())
}

const FILTER: &str = "requested_at >= ?1 AND requested_at < ?2 AND (?3 IS NULL OR model = ?3) \
     AND (?4 IS NULL OR client = ?4)";
const DAY: &str = "date(requested_at / 1000, 'unixepoch', 'localtime')";

#[derive(Deserialize)]
struct RecordRow {
    id: String,
    requested_at: i64,
    client: Option<String>,
    route_mode: Option<String>,
    provider_name: String,
    protocol: String,
    model: String,
    input_tokens: u32,
    output_tokens: u32,
    cached_tokens: u32,
    reasoning_tokens: u32,
    step_id: Option<String>,
    session_id: Option<String>,
    ttft_ms: Option<u32>,
    duration_ms: Option<u32>,
    status: Option<u16>,
}

impl RecordRow {
    fn into_record(self) -> CoreResult<UsageRecord> {
        Ok(UsageRecord {
            id: self.id,
            requested_at: self.requested_at,
            client: self.client.as_deref().map(parse_enum).transpose()?,
            route_mode: self.route_mode.as_deref().map(parse_enum).transpose()?,
            provider_name: self.provider_name,
            protocol: parse_enum(&self.protocol)?,
            model: self.model,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cached_tokens: self.cached_tokens,
            reasoning_tokens: self.reasoning_tokens,
            step_id: self.step_id,
            session_id: self.session_id,
            ttft_ms: self.ttft_ms,
            duration_ms: self.duration_ms,
            status: self.status,
        })
    }
}

/// SQLite integers are signed; aggregates here are never negative.
fn unsigned(r: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<u64> {
    Ok(u64::try_from(r.get::<_, i64>(idx)?).unwrap_or(0))
}

/// Aggregates events in `[start_ms, end_ms)` by day, model, day x model and client.
/// Day buckets (`days`, `daily_models`) use the local time zone.
pub fn query(conn: &Connection, q: UsageQuery) -> CoreResult<UsageReport> {
    let client = q.client.as_ref().map(enum_str).transpose()?;
    let args = || (q.start_ms, q.end_ms, q.model.as_deref(), client.as_deref());

    let (total_requests, input_tokens, output_tokens, cached_tokens, reasoning_tokens) = conn
        .query_row(
            &format!(
                "SELECT COUNT(*), COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0), \
                   COALESCE(SUM(cached_tokens), 0), COALESCE(SUM(reasoning_tokens), 0) \
                 FROM usage WHERE {FILTER}"
            ),
            args(),
            |r| {
                Ok((
                    unsigned(r, 0)?,
                    unsigned(r, 1)?,
                    unsigned(r, 2)?,
                    unsigned(r, 3)?,
                    unsigned(r, 4)?,
                ))
            },
        )?;

    let days = conn
        .prepare(&format!(
            "SELECT {DAY}, SUM(input_tokens), SUM(output_tokens), SUM(cached_tokens), \
               SUM(reasoning_tokens) FROM usage WHERE {FILTER} GROUP BY 1 ORDER BY 1"
        ))?
        .query_map(args(), |r| {
            Ok(UsageDay {
                day: r.get(0)?,
                input_tokens: unsigned(r, 1)?,
                output_tokens: unsigned(r, 2)?,
                cached_tokens: unsigned(r, 3)?,
                reasoning_tokens: unsigned(r, 4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let daily_models = conn
        .prepare(&format!(
            "SELECT {DAY}, model, SUM(input_tokens + output_tokens) FROM usage \
             WHERE {FILTER} GROUP BY 1, 2 ORDER BY 1, 3 DESC, 2"
        ))?
        .query_map(args(), |r| {
            Ok(UsageDailyModel {
                day: r.get(0)?,
                model: r.get(1)?,
                tokens: unsigned(r, 2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let models = conn
        .prepare(&format!(
            "SELECT model, COUNT(*), SUM(input_tokens), SUM(output_tokens), SUM(cached_tokens), \
               SUM(reasoning_tokens) FROM usage WHERE {FILTER} \
             GROUP BY model ORDER BY SUM(input_tokens + output_tokens) DESC, model"
        ))?
        .query_map(args(), |r| {
            Ok(UsageModel {
                model: r.get(0)?,
                requests: unsigned(r, 1)?,
                input_tokens: unsigned(r, 2)?,
                output_tokens: unsigned(r, 3)?,
                cached_tokens: unsigned(r, 4)?,
                reasoning_tokens: unsigned(r, 5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let by_client = conn
        .prepare(&format!(
            "SELECT client, COUNT(*), SUM(input_tokens), SUM(output_tokens), SUM(cached_tokens), \
               SUM(reasoning_tokens) FROM usage WHERE {FILTER} \
             GROUP BY client ORDER BY SUM(input_tokens + output_tokens) DESC, client"
        ))?
        .query_map(args(), |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                unsigned(r, 1)?,
                unsigned(r, 2)?,
                unsigned(r, 3)?,
                unsigned(r, 4)?,
                unsigned(r, 5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|(client, requests, input, output, cached, reasoning)| {
            Ok(UsageClient {
                client: client.as_deref().map(parse_enum).transpose()?,
                requests,
                input_tokens: input,
                output_tokens: output,
                cached_tokens: cached,
                reasoning_tokens: reasoning,
            })
        })
        .collect::<CoreResult<Vec<_>>>()?;

    let recent = query_as::<RecordRow>(
        conn,
        &format!(
            "SELECT id, requested_at, client, route_mode, provider_name, protocol, model, \
               input_tokens, output_tokens, cached_tokens, reasoning_tokens, \
               step_id, session_id, ttft_ms, duration_ms, status \
             FROM usage WHERE {FILTER} ORDER BY requested_at DESC, rowid DESC LIMIT {RECENT_LIMIT}"
        ),
        args(),
    )?
    .into_iter()
    .map(RecordRow::into_record)
    .collect::<CoreResult<Vec<_>>>()?;

    Ok(UsageReport {
        total_requests,
        input_tokens,
        output_tokens,
        cached_tokens,
        reasoning_tokens,
        days,
        daily_models,
        models,
        by_client,
        recent,
    })
}

/// `cached / input`, or `None` when no input tokens were recorded.
fn hit_rate(cached: u64, input: u64) -> Option<f64> {
    (input > 0).then(|| cached as f64 / input as f64)
}

/// Totals and timing averages over the same window and filters as [`query`].
/// Averages skip NULL timings (requests that never reached a first token or completion).
pub fn totals(conn: &Connection, q: &UsageQuery) -> CoreResult<UsageTotals> {
    let client = q.client.as_ref().map(enum_str).transpose()?;
    let args = (q.start_ms, q.end_ms, q.model.as_deref(), client.as_deref());
    Ok(conn.query_row(
        &format!(
            "SELECT COUNT(*), COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0), \
               COALESCE(SUM(cached_tokens), 0), COALESCE(SUM(reasoning_tokens), 0), \
               COUNT(CASE WHEN ttft_ms IS NOT NULL OR duration_ms IS NOT NULL THEN 1 END), \
               AVG(ttft_ms), AVG(duration_ms) \
             FROM usage WHERE {FILTER}"
        ),
        args,
        |r| {
            let input_tokens = unsigned(r, 1)?;
            let cached_tokens = unsigned(r, 3)?;
            Ok(UsageTotals {
                requests: unsigned(r, 0)?,
                input_tokens,
                output_tokens: unsigned(r, 2)?,
                cached_tokens,
                reasoning_tokens: unsigned(r, 4)?,
                cache_hit_rate: hit_rate(cached_tokens, input_tokens),
                avg_ttft_ms: r.get(6)?,
                avg_duration_ms: r.get(7)?,
                timed_requests: unsigned(r, 5)?,
            })
        },
    )?)
}

/// Aggregates by `by`, ordered by total tokens (descending), then key.
pub fn groups(
    conn: &Connection,
    q: &UsageQuery,
    by: UsageGroupBy,
) -> CoreResult<Vec<UsageGroupRow>> {
    // The column comes from this fixed match, never from user input.
    let column = match by {
        UsageGroupBy::Model => "model",
        UsageGroupBy::Client => "COALESCE(client, '')",
        UsageGroupBy::Provider => "provider_name",
    };
    let client = q.client.as_ref().map(enum_str).transpose()?;
    let args = (q.start_ms, q.end_ms, q.model.as_deref(), client.as_deref());
    conn.prepare(&format!(
        "SELECT {column}, COUNT(*), SUM(input_tokens), SUM(output_tokens), SUM(cached_tokens), \
           AVG(duration_ms) \
         FROM usage WHERE {FILTER} GROUP BY 1 ORDER BY SUM(input_tokens + output_tokens) DESC, 1"
    ))?
    .query_map(args, |r| {
        let key: String = r.get(0)?;
        let input_tokens = unsigned(r, 2)?;
        let cached_tokens = unsigned(r, 4)?;
        Ok(UsageGroupRow {
            label: key.clone(),
            key,
            requests: unsigned(r, 1)?,
            input_tokens,
            output_tokens: unsigned(r, 3)?,
            cached_tokens,
            cache_hit_rate: hit_rate(cached_tokens, input_tokens),
            avg_duration_ms: r.get(5)?,
        })
    })?
    .collect::<rusqlite::Result<Vec<_>>>()
    .map_err(Into::into)
}

/// The most recent `limit` requests, oldest first, shaped like live-registry entries so the
/// live view can show them again after a restart.
pub fn recent_live_requests(conn: &Connection, limit: usize) -> CoreResult<Vec<LiveRequest>> {
    let mut stmt = conn.prepare(
        "SELECT requested_at, client, provider_name, model, input_tokens, output_tokens, \
           step_id, session_id, ttft_ms, duration_ms, status \
         FROM usage ORDER BY requested_at DESC, rowid DESC LIMIT ?1",
    )?;
    let rows = stmt
        .query_map([limit as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<i64>>(8)?,
                r.get::<_, Option<i64>>(9)?,
                r.get::<_, Option<i64>>(10)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(
        rows.into_iter()
            .rev()
            .map(|row| {
                let (
                    at,
                    client,
                    provider,
                    model,
                    input,
                    output,
                    step,
                    session,
                    ttft,
                    duration,
                    status,
                ) = row;
                let status = status.map(|s| s as u16);
                LiveRequest {
                    seq: 0,
                    step_id: step.unwrap_or_default(),
                    client: client.and_then(|c| parse_enum::<Client>(&c).ok()),
                    model,
                    provider_name: provider,
                    started_at: at,
                    first_token_at: ttft.map(|t| at + t),
                    finished_at: duration.map(|d| at + d),
                    phase: if status.is_some_and(|s| s >= 400) {
                        LivePhase::Error
                    } else {
                        LivePhase::Done
                    },
                    status,
                    input_tokens: input as u64,
                    output_tokens: output as u64,
                    session_id: session,
                }
            })
            .collect(),
    )
}
