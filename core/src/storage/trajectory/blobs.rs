//! Content-addressed blob store for trajectory bodies. Blobs are keyed by the
//! SHA-256 of their canonical body, so identical content is stored once, and
//! the stored bytes are zstd-compressed.

use crate::error::{CoreError, CoreResult};
use crate::trajectory::normalize::{ItemKind, NormalizedItem};
use rusqlite::{Connection, OptionalExtension, params};

const ZSTD_LEVEL: i32 = 3;

/// Deletes up to `?1` blobs that no step item, step system prompt or step tool list
/// refers to. Each `NOT EXISTS` is kept separate so it can probe its own index
/// (`idx_step_items_blob`, `idx_steps_system_blob`, `idx_steps_tools_blob`).
const DELETE_ORPHANS_SQL: &str = "DELETE FROM blobs WHERE hash IN (SELECT hash FROM blobs b WHERE NOT EXISTS (SELECT 1 FROM step_items i WHERE i.blob = b.hash) AND NOT EXISTS (SELECT 1 FROM steps s WHERE s.system_blob = b.hash) AND NOT EXISTS (SELECT 1 FROM steps s WHERE s.tools_blob = b.hash) LIMIT ?1)";

/// A blob read back from the store with its body decompressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBlob {
    pub kind: String,
    pub raw_bytes: u64,
    pub truncated: bool,
    pub json: String,
}

/// Stores the item's body if its hash is new. Returns `true` when a row was inserted.
pub fn put(conn: &Connection, item: &NormalizedItem) -> CoreResult<bool> {
    Ok(put_measured(conn, item)?.is_some())
}

/// As [`put`], but returns the compressed size of the row when one was inserted.
pub fn put_measured(conn: &Connection, item: &NormalizedItem) -> CoreResult<Option<u64>> {
    let data = zstd::stream::encode_all(&item.body[..], ZSTD_LEVEL)
        .map_err(|e| CoreError::Internal(format!("轨迹 blob 压缩失败：{e}")))?;
    let raw_bytes = i64::try_from(item.raw_bytes)
        .map_err(|_| CoreError::Internal("轨迹 blob 原始长度超出范围".into()))?;
    let stored = u64::try_from(data.len()).unwrap_or(u64::MAX);
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO blobs (hash, kind, raw_bytes, data, truncated) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            item.hash,
            kind_name(item.kind),
            raw_bytes,
            data,
            item.truncated
        ],
    )?;
    Ok((inserted == 1).then_some(stored))
}

/// Loads the blob with `hash` and decompresses it, or `None` when absent.
pub fn get(conn: &Connection, hash: &str) -> CoreResult<Option<StoredBlob>> {
    let row = conn
        .query_row(
            "SELECT kind, raw_bytes, truncated, data FROM blobs WHERE hash = ?1",
            [hash],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, bool>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((kind, raw_bytes, truncated, data)) = row else {
        return Ok(None);
    };
    let body = zstd::stream::decode_all(&data[..])
        .map_err(|e| CoreError::Internal(format!("轨迹 blob 解压失败：{e}")))?;
    let json = String::from_utf8(body)
        .map_err(|e| CoreError::Internal(format!("轨迹 blob 不是 UTF-8：{e}")))?;
    Ok(Some(StoredBlob {
        kind,
        raw_bytes: to_u64(raw_bytes)?,
        truncated,
        json,
    }))
}

/// Deletes up to `limit` unreferenced blobs and returns how many were removed.
pub fn delete_orphans(conn: &Connection, limit: u32) -> CoreResult<u32> {
    let deleted = conn.execute(DELETE_ORPHANS_SQL, [limit])?;
    u32::try_from(deleted).map_err(|_| CoreError::Internal("删除条数超出范围".into()))
}

/// Total compressed bytes held by all blobs.
pub fn total_bytes(conn: &Connection) -> CoreResult<u64> {
    let total: i64 = conn.query_row(
        "SELECT COALESCE(SUM(length(data)), 0) FROM blobs",
        [],
        |r| r.get(0),
    )?;
    to_u64(total)
}

fn to_u64(value: i64) -> CoreResult<u64> {
    u64::try_from(value).map_err(|_| CoreError::Database("轨迹 blob 长度为负".into()))
}

/// Stable string stored in the `blobs.kind` column.
pub fn kind_name(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Message => "message",
        ItemKind::ToolCall => "tool_call",
        ItemKind::ToolResult => "tool_result",
        ItemKind::Reasoning => "reasoning",
        ItemKind::System => "system",
        ItemKind::Tools => "tools",
        ItemKind::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::trajectory::TrajectoryDb;
    use crate::trajectory::normalize::EMPTY_FP;
    use rusqlite::params;
    use sha2::{Digest, Sha256};

    fn db() -> TrajectoryDb {
        TrajectoryDb::open_in_memory().unwrap()
    }

    fn item_with(kind: ItemKind, body: &str) -> NormalizedItem {
        let hash = format!("{:x}", Sha256::digest(body.as_bytes()));
        NormalizedItem {
            kind,
            role: None,
            call_id: None,
            tool_name: None,
            body: body.as_bytes().to_vec(),
            raw_bytes: body.len() as u64,
            truncated: false,
            hash,
            fp: EMPTY_FP,
        }
    }

    fn item(body: &str) -> NormalizedItem {
        item_with(ItemKind::Message, body)
    }

    fn insert_step(
        conn: &Connection,
        id: &str,
        system_blob: Option<&str>,
        tools_blob: Option<&str>,
    ) {
        conn.execute(
            "INSERT OR IGNORE INTO sessions (id, client, key_source, started_at, last_at) \
             VALUES ('s1', 'test', 'new', 0, 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO threads (id, session_id, kind, started_at) \
             VALUES ('t1', 's1', 'main', 0)",
            [],
        )
        .unwrap();
        let seq: i64 = conn
            .query_row("SELECT COUNT(*) FROM steps", [], |r| r.get(0))
            .unwrap();
        conn.execute(
            "INSERT INTO steps (id, seq, session_id, thread_id, prefix_len, turn_no, requested_at, \
             stop_reason, inbound_protocol, upstream_protocol, provider_name, model, upstream_model, \
             tail_fp, capture, system_blob, tools_blob) \
             VALUES (?1, ?2, 's1', 't1', 0, 0, 0, 'stop', 'openai', 'openai', 'p', 'm', 'm', \
             'fp', 'full', ?3, ?4)",
            params![id, seq + 1, system_blob, tools_blob],
        )
        .unwrap();
    }

    fn insert_step_item(conn: &Connection, step_id: &str, ord: i64, blob: &str) {
        conn.execute(
            "INSERT INTO step_items (step_id, side, ord, blob, kind) VALUES (?1, 'input', ?2, ?3, 'message')",
            params![step_id, ord, blob],
        )
        .unwrap();
    }

    fn blob_exists(conn: &Connection, hash: &str) -> bool {
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM blobs WHERE hash = ?1", [hash], |r| {
                r.get(0)
            })
            .unwrap();
        n == 1
    }

    #[test]
    fn put_is_idempotent() {
        let db = db();
        db.0.with_conn(|conn| {
            let it = item(r#"{"role":"user"}"#);
            assert!(put(conn, &it)?);
            assert!(!put(conn, &it)?);
            let rows: i64 = conn
                .query_row("SELECT COUNT(*) FROM blobs", [], |r| r.get(0))
                .unwrap();
            assert_eq!(rows, 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn round_trip_decompresses() {
        let db = db();
        db.0.with_conn(|conn| {
            let body = r#"{"name":"f","call_id":"c1"}"#;
            let mut it = item_with(ItemKind::ToolCall, body);
            it.raw_bytes = 4321;
            it.truncated = true;
            put(conn, &it)?;

            let got = get(conn, &it.hash)?.expect("stored blob");
            assert_eq!(got.kind, "tool_call");
            assert_eq!(got.raw_bytes, 4321);
            assert!(got.truncated);
            assert_eq!(got.json, body);

            assert!(get(conn, "missing-hash")?.is_none());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn orphans_are_removed_but_referenced_kept() {
        let db = db();
        db.0.with_conn(|conn| {
            let via_items = item(r#"{"via":"items"}"#);
            let via_system = item(r#"{"via":"system"}"#);
            let via_tools = item(r#"{"via":"tools"}"#);
            let orphan = item(r#"{"via":"nothing"}"#);
            for it in [&via_items, &via_system, &via_tools, &orphan] {
                put(conn, it)?;
            }
            insert_step(conn, "st1", Some(&via_system.hash), Some(&via_tools.hash));
            insert_step_item(conn, "st1", 0, &via_items.hash);

            assert_eq!(delete_orphans(conn, 100)?, 1);
            assert!(!blob_exists(conn, &orphan.hash));
            assert!(blob_exists(conn, &via_items.hash));
            assert!(blob_exists(conn, &via_system.hash));
            assert!(blob_exists(conn, &via_tools.hash));
            // Nothing left to clean up on a second pass.
            assert_eq!(delete_orphans(conn, 100)?, 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn orphan_cleanup_respects_limit() {
        let db = db();
        db.0.with_conn(|conn| {
            for n in 0..3 {
                put(conn, &item(&format!(r#"{{"orphan":{n}}}"#)))?;
            }
            assert_eq!(delete_orphans(conn, 2)?, 2);
            assert_eq!(delete_orphans(conn, 2)?, 1);
            assert_eq!(delete_orphans(conn, 2)?, 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn compression_shrinks_repetitive_json() {
        let db = db();
        db.0.with_conn(|conn| {
            let body = format!(r#"{{"text":"{}"}}"#, "lorem ipsum ".repeat(9000));
            assert!(body.len() > 100 * 1024);
            put(conn, &item(&body))?;
            let stored: u64 = conn
                .query_row("SELECT length(data) FROM blobs", [], |r| r.get::<_, i64>(0))
                .unwrap() as u64;
            assert!(stored < 10 * 1024, "stored {stored} bytes");
            assert_eq!(total_bytes(conn)?, stored);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn orphan_query_plan_uses_indexes() {
        let db = db();
        db.0.with_conn(|conn| {
            let sql = format!("EXPLAIN QUERY PLAN {DELETE_ORPHANS_SQL}");
            let mut stmt = conn.prepare(&sql).unwrap();
            let details: Vec<String> = stmt
                .query_map([10u32], |r| r.get::<_, String>(3))
                .unwrap()
                .map(|d| d.unwrap())
                .collect();
            let plan = details.join("\n");
            // The subqueries are aliased `s` and `i`, so a full scan of either shows
            // as "SCAN s" / "SCAN i". The only scan allowed is the outer walk of `blobs` (alias `b`).
            let scans: Vec<&str> = plan.lines().filter(|l| l.starts_with("SCAN ")).collect();
            assert!(
                scans.iter().all(|l| l.starts_with("SCAN b ")),
                "unexpected scan in plan:\n{plan}"
            );
            assert!(
                plan.contains("SEARCH i USING COVERING INDEX idx_step_items_blob"),
                "{plan}"
            );
            assert!(
                plan.contains("SEARCH s USING COVERING INDEX idx_steps_system_blob"),
                "{plan}"
            );
            assert!(
                plan.contains("SEARCH s USING COVERING INDEX idx_steps_tools_blob"),
                "{plan}"
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn total_bytes_is_zero_when_empty() {
        let db = db();
        db.0.with_conn(|conn| {
            assert_eq!(total_bytes(conn)?, 0);
            Ok(())
        })
        .unwrap();
    }
}
