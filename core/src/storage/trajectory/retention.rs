//! Retention and cleanup of the trajectory database (spec 5.3).
//!
//! Sessions are deleted whole, each in its own transaction. Blobs are only removed by
//! [`blobs::delete_orphans`], which the writer thread calls after retention.

use rusqlite::Connection;

use crate::domain::Client;
use crate::error::CoreResult;
use crate::storage::trajectory::blobs;
use crate::storage::trajectory::repo;

/// Sessions deleted per capacity round before orphaned blobs are collected.
const SESSIONS_PER_ROUND: u32 = 20;
/// Blobs removed per `delete_orphans` call.
const ORPHAN_BATCH: u32 = 1000;
/// `last_at` is stored in milliseconds (see the proxy's `now_ms`).
const DAY_MS: i64 = 86_400_000;

/// What one [`enforce`] pass removed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RetentionReport {
    pub sessions_deleted: u32,
    pub blobs_deleted: u32,
    pub bytes_after: u64,
    /// Ids of the sessions this pass deleted. The writer evicts in-memory tails that
    /// point at them, so a later step cannot attach to a missing session.
    pub deleted_session_ids: Vec<String>,
}

/// Deletes sessions older than `retention_days`, then the oldest sessions while the
/// blob store is over `max_bytes`. `now` is in milliseconds.
pub fn enforce(
    conn: &Connection,
    retention_days: u32,
    max_bytes: u64,
    now: i64,
) -> CoreResult<RetentionReport> {
    let mut report = RetentionReport::default();

    // Expired sessions, then the blobs they orphaned.
    let cutoff = now.saturating_sub(i64::from(retention_days).saturating_mul(DAY_MS));
    let expired = session_ids(
        conn,
        "SELECT id FROM sessions WHERE last_at < ?1 ORDER BY last_at ASC",
        [cutoff],
    )?;
    for id in &expired {
        delete_in_tx(conn, id)?;
        record_deleted(&mut report, id);
    }
    report.blobs_deleted = report.blobs_deleted.saturating_add(drain_orphans(conn)?);

    // Then the oldest sessions, a round at a time, while the blob store is over capacity.
    while blobs::total_bytes(conn)? > max_bytes {
        let victims = session_ids(
            conn,
            "SELECT id FROM sessions ORDER BY last_at ASC LIMIT ?1",
            [SESSIONS_PER_ROUND],
        )?;
        if victims.is_empty() {
            break;
        }
        for id in &victims {
            delete_in_tx(conn, id)?;
            record_deleted(&mut report, id);
        }
        report.blobs_deleted = report.blobs_deleted.saturating_add(drain_orphans(conn)?);
    }

    report.bytes_after = blobs::total_bytes(conn)?;
    Ok(report)
}

/// Deletes one session and, through the foreign keys, its threads, steps and step items.
/// Runs in its own transaction. Blobs are left for `delete_orphans`.
pub fn delete_session(conn: &Connection, id: &str) -> CoreResult<()> {
    delete_in_tx(conn, id)
}

/// Deletes every session recorded for `client`, each in its own transaction, and returns
/// how many were deleted. Other clients are untouched. Orphaned blobs are left for the next
/// [`enforce`].
pub fn clear_client(conn: &Connection, client: Client) -> CoreResult<u32> {
    let ids = session_ids(
        conn,
        "SELECT id FROM sessions WHERE client = ?1",
        [repo::client_column(Some(client))],
    )?;
    for id in &ids {
        delete_in_tx(conn, id)?;
    }
    Ok(u32::try_from(ids.len()).unwrap_or(u32::MAX))
}

/// Ids of the sessions recorded for `client`. The writer reads them before [`clear_client`]
/// so it can evict their in-memory tails.
pub fn client_session_ids(conn: &Connection, client: Client) -> CoreResult<Vec<String>> {
    session_ids(
        conn,
        "SELECT id FROM sessions WHERE client = ?1",
        [repo::client_column(Some(client))],
    )
}

/// Removes unreferenced blobs, in batches. Only the writer thread may call this (spec 5.3).
pub fn collect_orphans(conn: &Connection) -> CoreResult<u32> {
    drain_orphans(conn)
}

/// Runs `PRAGMA incremental_vacuum` when `deleted_bytes` is more than 25% of `total_bytes`.
/// Returns whether it ran.
pub fn vacuum_if_needed(
    conn: &Connection,
    deleted_bytes: u64,
    total_bytes: u64,
) -> CoreResult<bool> {
    // deleted > total / 4, computed without overflow.
    if u128::from(deleted_bytes) * 4 <= u128::from(total_bytes) {
        return Ok(false);
    }
    conn.execute_batch("PRAGMA incremental_vacuum")?;
    Ok(true)
}

/// Deletes one session in its own transaction.
fn delete_in_tx(conn: &Connection, id: &str) -> CoreResult<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM sessions WHERE id = ?1", [id])?;
    tx.commit()?;
    Ok(())
}

/// Removes unreferenced blobs in batches until one batch comes back empty.
fn drain_orphans(conn: &Connection) -> CoreResult<u32> {
    let mut total: u32 = 0;
    loop {
        let removed = blobs::delete_orphans(conn, ORPHAN_BATCH)?;
        if removed == 0 {
            return Ok(total);
        }
        total = total.saturating_add(removed);
    }
}

fn session_ids(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> CoreResult<Vec<String>> {
    let mut stmt = conn.prepare(sql)?;
    let ids = stmt
        .query_map(params, |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ids)
}

fn record_deleted(report: &mut RetentionReport, id: &str) {
    report.sessions_deleted = report.sessions_deleted.saturating_add(1);
    report.deleted_session_ids.push(id.to_owned());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::KeySource;
    use crate::storage::trajectory::TrajectoryDb;
    use crate::storage::trajectory::blobs;
    use crate::storage::trajectory::repo::{self, StepRow, ThreadRow};
    use crate::trajectory::link::ThreadKind;
    use crate::trajectory::normalize::{EMPTY_FP, ItemKind, NormalizedItem};
    use sha2::{Digest, Sha256};

    /// 2024-01-01 in milliseconds. Sessions are seeded relative to it.
    const NOW: i64 = 1_704_067_200_000;

    fn db() -> TrajectoryDb {
        TrajectoryDb::open_in_memory().unwrap()
    }

    fn item(body: &str) -> NormalizedItem {
        NormalizedItem {
            kind: ItemKind::Message,
            role: Some("user".into()),
            call_id: None,
            tool_name: None,
            body: body.as_bytes().to_vec(),
            raw_bytes: body.len() as u64,
            truncated: false,
            hash: format!("{:x}", Sha256::digest(body.as_bytes())),
            fp: EMPTY_FP,
        }
    }

    /// Inserts a session with one main thread, one step and one input item per body.
    fn seed(conn: &Connection, id: &str, client: Client, last_at: i64, bodies: &[&str]) {
        repo::insert_session(conn, id, Some(client), None, KeySource::New, last_at).unwrap();
        let thread = format!("t-{id}");
        repo::insert_thread(
            conn,
            &ThreadRow {
                id: &thread,
                session_id: id,
                kind: ThreadKind::Main,
                parent_thread_id: None,
                spawn_call_id: None,
                reset: false,
                started_at: last_at,
            },
        )
        .unwrap();
        let step = format!("st-{id}");
        let tail_fp = format!("fp-{id}");
        repo::insert_step(
            conn,
            &StepRow {
                id: &step,
                session_id: id,
                thread_id: &thread,
                parent_step_id: None,
                prefix_len: 0,
                checkpoint: false,
                depth: 0,
                turn_no: 0,
                client_turn_id: None,
                requested_at: last_at,
                first_token_at: None,
                finished_at: None,
                status: None,
                stop_reason: "end",
                error: None,
                inbound_protocol: "openai",
                upstream_protocol: "openai",
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
                tail_fp: &tail_fp,
                capture: "full",
            },
        )
        .unwrap();
        for (ord, body) in bodies.iter().enumerate() {
            let it = item(body);
            blobs::put(conn, &it).unwrap();
            repo::insert_item(conn, &step, "input", ord as i64, &it).unwrap();
        }
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn session_exists(conn: &Connection, id: &str) -> bool {
        count(
            conn,
            &format!("SELECT COUNT(*) FROM sessions WHERE id = '{id}'"),
        ) == 1
    }

    fn blob_exists(conn: &Connection, body: &str) -> bool {
        let hash = format!("{:x}", Sha256::digest(body.as_bytes()));
        count(
            conn,
            &format!("SELECT COUNT(*) FROM blobs WHERE hash = '{hash}'"),
        ) == 1
    }

    const DAY: i64 = DAY_MS;

    #[test]
    fn expired_sessions_are_deleted() {
        let db = db();
        db.0.with_conn(|conn| {
            seed(conn, "old", Client::Codex, NOW - 31 * DAY, &["old-body"]);
            seed(conn, "fresh", Client::Codex, NOW - DAY, &["fresh-body"]);

            let report = enforce(conn, 30, u64::MAX, NOW)?;

            assert_eq!(report.sessions_deleted, 1);
            assert_eq!(report.deleted_session_ids, vec!["old".to_string()]);
            assert!(!session_exists(conn, "old"));
            assert!(session_exists(conn, "fresh"));
            // The expired session's exclusive blob is orphaned and collected.
            assert!(!blob_exists(conn, "old-body"));
            assert!(blob_exists(conn, "fresh-body"));
            assert_eq!(report.blobs_deleted, 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn over_capacity_deletes_oldest_first() {
        let db = db();
        db.0.with_conn(|conn| {
            // 25 sessions, s00 oldest. Each has its own blob.
            let ids: Vec<String> = (0..25).map(|i| format!("s{i:02}")).collect();
            for (i, id) in ids.iter().enumerate() {
                let last_at = NOW - (25 - i as i64) * DAY;
                seed(conn, id, Client::Codex, last_at, &[&format!("body-{i}")]);
            }
            let total = blobs::total_bytes(conn)?;

            // One round (20 sessions) is enough to get under the cap, so only the 20 oldest go.
            let report = enforce(conn, 30, total - 1, NOW)?;
            assert_eq!(report.sessions_deleted, 20);
            assert_eq!(report.deleted_session_ids, ids[..20].to_vec());
            assert!(session_exists(conn, "s20") && session_exists(conn, "s24"));
            assert!(report.bytes_after < total - 1);

            // Zero capacity drains the rest, still oldest first.
            let report = enforce(conn, 30, 0, NOW)?;
            assert_eq!(report.deleted_session_ids, ids[20..].to_vec());
            assert_eq!(report.bytes_after, 0);
            assert_eq!(count(conn, "SELECT COUNT(*) FROM blobs"), 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn shared_blob_survives_when_other_session_references_it() {
        let db = db();
        db.0.with_conn(|conn| {
            seed(
                conn,
                "old",
                Client::Codex,
                NOW - 31 * DAY,
                &["shared", "only-old"],
            );
            seed(conn, "fresh", Client::Codex, NOW - DAY, &["shared"]);

            enforce(conn, 30, u64::MAX, NOW)?;

            assert!(!session_exists(conn, "old"));
            assert!(blob_exists(conn, "shared"));
            assert!(!blob_exists(conn, "only-old"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn delete_session_cascades_steps_and_items() {
        let db = db();
        db.0.with_conn(|conn| {
            seed(conn, "doomed", Client::Codex, NOW, &["x", "y"]);
            seed(conn, "kept", Client::Codex, NOW, &["z"]);

            delete_session(conn, "doomed")?;

            assert!(!session_exists(conn, "doomed"));
            assert_eq!(
                count(
                    conn,
                    "SELECT COUNT(*) FROM threads WHERE session_id = 'doomed'"
                ),
                0
            );
            assert_eq!(
                count(
                    conn,
                    "SELECT COUNT(*) FROM steps WHERE session_id = 'doomed'"
                ),
                0
            );
            assert_eq!(
                count(
                    conn,
                    "SELECT COUNT(*) FROM step_items WHERE step_id = 'st-doomed'"
                ),
                0
            );
            // The other session keeps its rows.
            assert!(session_exists(conn, "kept"));
            assert_eq!(count(conn, "SELECT COUNT(*) FROM step_items"), 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn clear_client_only_touches_that_client() {
        let db = db();
        db.0.with_conn(|conn| {
            seed(conn, "codex-1", Client::Codex, NOW, &["c1"]);
            seed(conn, "codex-2", Client::Codex, NOW, &["c2"]);
            seed(conn, "claude-1", Client::ClaudeCode, NOW, &["k1"]);

            assert_eq!(clear_client(conn, Client::Codex)?, 2);

            assert!(!session_exists(conn, "codex-1"));
            assert!(!session_exists(conn, "codex-2"));
            assert!(session_exists(conn, "claude-1"));
            assert_eq!(
                count(
                    conn,
                    "SELECT COUNT(*) FROM steps WHERE session_id = 'claude-1'"
                ),
                1
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn vacuum_threshold_is_strictly_over_a_quarter() {
        let db = db();
        db.0.with_conn(|conn| {
            assert!(!vacuum_if_needed(conn, 25, 100)?);
            assert!(vacuum_if_needed(conn, 26, 100)?);
            assert!(!vacuum_if_needed(conn, 0, 0)?);
            Ok(())
        })
        .unwrap();
    }
}
