//! Usage events: batched inserts and aggregated reports.

use super::{enum_str, parse_enum, query_as};
use crate::domain::{
    UsageClient, UsageDailyModel, UsageDay, UsageEvent, UsageModel, UsageRecord, UsageReport,
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
           input_tokens, output_tokens, cached_tokens, reasoning_tokens) \
         VALUES (?11, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
        ])?;
    }
    Ok(())
}

const FILTER: &str = "requested_at >= ?1 AND requested_at < ?2 AND (?3 IS NULL OR model = ?3)";
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
    let args = || (q.start_ms, q.end_ms, q.model.as_deref());

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
               input_tokens, output_tokens, cached_tokens, reasoning_tokens \
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
