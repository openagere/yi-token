//! Trajectory writer (spec 4.2, 5.1, 5.2, 5.4). `TrajectorySink` is the capture
//! sink the proxy submits to; it never blocks. A single thread links each record
//! and writes it, one SAVEPOINT per record, in batches.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{
    Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError, channel, sync_channel,
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use bytes::Bytes;
use rusqlite::{Connection, params};
use serde_json::Value;
use uuid::Uuid;

use crate::domain::{Client, TrajectorySettings};
use crate::error::{CoreError, CoreResult};
use crate::protocol::{to_ir, translate_response_parsed};
use crate::proxy::live::LiveRegistry;
use crate::storage::Db;
use crate::storage::trajectory::TrajectoryDb;
use crate::storage::trajectory::blobs;
use crate::storage::trajectory::repo::{
    self, SessionDelta, SqliteLinkStore, StepRow, TailIndex, ThreadRow,
};
use crate::storage::trajectory::retention;
use crate::trajectory::link::{SessionDecision, StepInput, TailEntry, ThreadDecision, link};
use crate::trajectory::normalize::{
    ItemKind, NormalizedItem, first_user_text, normalize_item, normalize_system, normalize_tools,
};
use crate::trajectory::record::{CaptureRecord, CaptureSink};

/// In-flight capture budget: request plus response bytes held by queued records (spec 4.2).
pub const INFLIGHT_LIMIT_BYTES: usize = 64 * 1024 * 1024;
const QUEUE_CAPACITY: usize = 4096;
/// A batch commits at this many records or this many charged bytes, whichever comes first.
const BATCH_RECORDS: usize = 64;
const BATCH_BYTES: usize = 32 * 1024 * 1024;
/// A non-empty batch commits this long after its first record; an idle writer wakes this often.
const FLUSH_INTERVAL: Duration = Duration::from_millis(500);
/// A usage backfill is retried while its usage row has not been written yet.
const BACKFILL_TTL: Duration = Duration::from_secs(60);
const BACKFILL_CAPACITY: usize = 10_000;
const SAVEPOINT: &str = "trajectory_record";
/// Retention and capacity are enforced after this many committed steps (spec 5.3).
const RETENTION_EVERY_STEPS: usize = 256;

enum Msg {
    Record(Box<Job>),
    /// Commits what is pending, then acknowledges.
    Flush(Sender<()>),
    /// Deletes one session and evicts its tails (spec 5.3, manual delete).
    DeleteSession {
        id: String,
        ack: Sender<CoreResult<()>>,
    },
    /// Deletes every session of one client and evicts their tails.
    ClearClient {
        client: Client,
        ack: Sender<CoreResult<u32>>,
    },
    Stop,
}

/// One record on the queue.
struct Job {
    record: CaptureRecord,
    /// Bytes charged to the in-flight budget, returned once the job leaves the writer.
    charged: usize,
    /// Set when the bodies were dropped at submit time for the budget.
    metadata_only: bool,
}

/// Producer side of the trajectory writer. `submit` never blocks and never fails
/// the request path.
pub struct TrajectorySink {
    tx: SyncSender<Msg>,
    inflight: Arc<AtomicUsize>,
    limit: usize,
    settings: Arc<ArcSwap<TrajectorySettings>>,
}

impl TrajectorySink {
    /// Atomically replaces `enabled` and `disabled_clients`.
    pub fn set_settings(&self, s: &TrajectorySettings) {
        self.settings.store(Arc::new(s.clone()));
    }
}

impl CaptureSink for TrajectorySink {
    fn enabled_for(&self, client: Option<Client>) -> bool {
        let s = self.settings.load();
        // The global route (`None`) has its own switch; a terminal client is gated by its list.
        let allowed = match client {
            Some(c) => !s.disabled_clients.contains(&c),
            None => !s.disabled_global,
        };
        s.enabled && allowed
    }

    fn submit(&self, mut record: CaptureRecord) {
        let size = len_of(&record.request_body) + len_of(&record.response_body);
        let before = self.inflight.fetch_add(size, Ordering::SeqCst);
        let mut charged = size;
        let mut metadata_only = false;
        if before.saturating_add(size) > self.limit {
            self.inflight.fetch_sub(size, Ordering::SeqCst);
            charged = 0;
            metadata_only = true;
            record.request_body = None;
            record.response_body = None;
            record.response = None;
        }
        let step_id = record.step_id.clone();
        let msg = Msg::Record(Box::new(Job {
            record,
            charged,
            metadata_only,
        }));
        if let Err(err) = self.tx.try_send(msg) {
            let (reason, msg) = match err {
                TrySendError::Full(m) => ("queue is full", m),
                TrySendError::Disconnected(m) => ("writer has stopped", m),
            };
            if let Msg::Record(job) = msg {
                self.inflight.fetch_sub(job.charged, Ordering::SeqCst);
            }
            tracing::warn!(step_id = %step_id, "trajectory record dropped: {reason}");
        }
    }
}

/// Owns the writer thread.
pub struct TrajectoryWriterHandle {
    tx: SyncSender<Msg>,
    thread: JoinHandle<()>,
}

impl TrajectoryWriterHandle {
    /// Commits everything queued so far and stops the writer thread.
    /// Records submitted after this call may be dropped.
    pub fn shutdown(self) {
        // A failed send means the thread already exited; the join below still applies.
        let _ = self.tx.send(Msg::Stop);
        if self.thread.join().is_err() {
            tracing::error!("trajectory writer thread panicked");
        }
    }

    /// Blocks until everything queued so far has been committed.
    pub fn flush_for_test(&self) {
        let (ack_tx, ack_rx) = channel();
        if self.tx.send(Msg::Flush(ack_tx)).is_ok() {
            let _ = ack_rx.recv();
        }
    }

    /// Deletes a session on the writer thread, so the writer's tails stay consistent with
    /// the rows. Records queued before this call are committed first.
    pub fn delete_session(&self, id: &str) -> CoreResult<()> {
        self.request(|ack| Msg::DeleteSession {
            id: id.to_owned(),
            ack,
        })
    }

    /// Deletes every session of `client` on the writer thread and returns how many went.
    pub fn clear_client(&self, client: Client) -> CoreResult<u32> {
        self.request(|ack| Msg::ClearClient { client, ack })
    }

    /// Sends a request to the writer thread and waits for its answer.
    fn request<T>(&self, make: impl FnOnce(Sender<CoreResult<T>>) -> Msg) -> CoreResult<T> {
        let (ack_tx, ack_rx) = channel();
        let stopped = || CoreError::Internal("轨迹写入线程已停止".into());
        self.tx.send(make(ack_tx)).map_err(|_| stopped())?;
        ack_rx.recv().map_err(|_| stopped())?
    }
}

/// Starts the writer thread with the default in-flight budget.
pub fn spawn_trajectory_writer(
    tdb: TrajectoryDb,
    main: Db,
    live: LiveRegistry,
    settings: TrajectorySettings,
) -> (Arc<TrajectorySink>, TrajectoryWriterHandle) {
    spawn_trajectory_writer_with_limit(tdb, main, live, settings, INFLIGHT_LIMIT_BYTES)
}

/// As [`spawn_trajectory_writer`], with the in-flight budget set to `inflight_limit`
/// bytes instead of [`INFLIGHT_LIMIT_BYTES`].
pub fn spawn_trajectory_writer_with_limit(
    tdb: TrajectoryDb,
    main: Db,
    live: LiveRegistry,
    settings: TrajectorySettings,
    inflight_limit: usize,
) -> (Arc<TrajectorySink>, TrajectoryWriterHandle) {
    let (tx, rx) = sync_channel(QUEUE_CAPACITY);
    let inflight = Arc::new(AtomicUsize::new(0));
    let settings = Arc::new(ArcSwap::from_pointee(settings));
    let sink = Arc::new(TrajectorySink {
        tx: tx.clone(),
        inflight: Arc::clone(&inflight),
        limit: inflight_limit,
        settings: Arc::clone(&settings),
    });
    let worker = Worker {
        db: tdb.0,
        main,
        live,
        tails: TailIndex::default(),
        backfill: VecDeque::new(),
        inflight,
        settings,
        steps_since_retention: 0,
        freed_since_vacuum: 0,
    };
    let thread = std::thread::Builder::new()
        .name("trajectory-writer".into())
        .spawn(move || worker.run(&rx))
        .expect("spawn trajectory writer thread");
    (sink, TrajectoryWriterHandle { tx, thread })
}

/// A usage row whose `session_id` is still to be set.
struct Backfill {
    step_id: String,
    session_id: String,
    since: Instant,
}

/// The writer thread's state. Only this thread touches the trajectory database.
struct Worker {
    db: Db,
    main: Db,
    live: LiveRegistry,
    tails: TailIndex,
    backfill: VecDeque<Backfill>,
    inflight: Arc<AtomicUsize>,
    settings: Arc<ArcSwap<TrajectorySettings>>,
    /// Steps committed since the last retention pass.
    steps_since_retention: usize,
    /// Blob bytes freed by retention since the last vacuum, in this writer run (spec 5.3).
    freed_since_vacuum: u64,
}

impl Worker {
    fn run(mut self, rx: &Receiver<Msg>) {
        self.apply_retention(now_millis());
        let mut batch: Vec<Job> = Vec::new();
        let mut deadline = Instant::now() + FLUSH_INTERVAL;
        loop {
            if !batch.is_empty() && Instant::now() >= deadline {
                self.commit(&mut batch);
                self.retry_backfill();
            }
            let wait = if batch.is_empty() {
                FLUSH_INTERVAL
            } else {
                deadline.saturating_duration_since(Instant::now())
            };
            match rx.recv_timeout(wait) {
                Ok(Msg::Record(job)) => {
                    if batch.is_empty() {
                        deadline = Instant::now() + FLUSH_INTERVAL;
                    }
                    batch.push(*job);
                    let bytes: usize = batch.iter().map(|j| j.charged).sum();
                    if batch.len() >= BATCH_RECORDS || bytes >= BATCH_BYTES {
                        self.commit(&mut batch);
                    }
                }
                Ok(Msg::Flush(ack)) => {
                    self.commit(&mut batch);
                    self.retry_backfill();
                    let _ = ack.send(());
                }
                Ok(Msg::DeleteSession { id, ack }) => {
                    self.commit(&mut batch);
                    let _ = ack.send(self.delete_session(&id));
                }
                Ok(Msg::ClearClient { client, ack }) => {
                    self.commit(&mut batch);
                    let _ = ack.send(self.clear_client(client));
                }
                Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => {
                    self.commit(&mut batch);
                    return;
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.retry_backfill();
                }
            }
        }
    }

    /// Writes the batch in one transaction, then runs the after-commit steps.
    fn commit(&mut self, batch: &mut Vec<Job>) {
        if batch.is_empty() {
            return;
        }
        let jobs = std::mem::take(batch);
        let charged: usize = jobs.iter().map(|j| j.charged).sum();
        let tails = &mut self.tails;
        let outcome = self.db.with_tx(|tx| {
            let mut done = Vec::with_capacity(jobs.len());
            for job in &jobs {
                done.extend(write_job(tx, &mut *tails, job));
            }
            Ok(done)
        });
        match outcome {
            Ok(done) => {
                self.steps_since_retention += done.len();
                for d in done {
                    self.after_commit(d);
                }
                if self.steps_since_retention >= RETENTION_EVERY_STEPS {
                    self.apply_retention(now_millis());
                }
            }
            Err(e) => {
                // Nothing in this batch is stored, so the in-memory tails may point at rows that do not exist.
                tracing::error!("trajectory batch of {} records not stored: {e}", jobs.len());
                self.tails.clear();
            }
        }
        self.inflight.fetch_sub(charged, Ordering::SeqCst);
    }

    /// Deletes one session, then evicts its in-memory tails. Any failure also clears the
    /// tail index's entries for it, so a later step cannot attach to it.
    fn delete_session(&mut self, id: &str) -> CoreResult<()> {
        let outcome = self.db.with_conn(|conn| {
            if !repo::session_exists(conn, id)? {
                return Err(CoreError::NotFound(format!("会话不存在：{id}")));
            }
            retention::delete_session(conn, id)?;
            collect_orphans_logged(conn);
            Ok(())
        });
        self.tails.remove_sessions(&[id.to_owned()]);
        outcome
    }

    /// Deletes every session of `client` and evicts their tails. A failure clears the whole
    /// tail index, which only costs missed chain matches.
    fn clear_client(&mut self, client: Client) -> CoreResult<u32> {
        let outcome = self.db.with_conn(|conn| {
            let ids = retention::client_session_ids(conn, client)?;
            let removed = retention::clear_client(conn, client)?;
            collect_orphans_logged(conn);
            Ok((ids, removed))
        });
        match outcome {
            Ok((ids, removed)) => {
                self.tails.remove_sessions(&ids);
                Ok(removed)
            }
            Err(e) => {
                self.tails.clear();
                Err(e)
            }
        }
    }

    /// Enforces retention and capacity, then evicts in-memory tails of the sessions it
    /// deleted, so a later step cannot link to a session that no longer exists. Any
    /// failure clears the whole tail index, which only costs missed chain matches.
    fn apply_retention(&mut self, now: i64) {
        self.steps_since_retention = 0;
        let (days, max_bytes) = {
            let s = self.settings.load();
            (s.retention_days, s.max_bytes)
        };
        let freed_since_vacuum = &mut self.freed_since_vacuum;
        let outcome = self.db.with_conn(|conn| {
            let before = blobs::total_bytes(conn)?;
            let report = retention::enforce(conn, days, max_bytes, now)?;
            let freed = before.saturating_sub(report.bytes_after);
            vacuum_after_pass(conn, freed_since_vacuum, freed, before)?;
            Ok(report)
        });
        match outcome {
            Ok(report) => {
                self.tails.remove_sessions(&report.deleted_session_ids);
                if report.sessions_deleted > 0 {
                    tracing::info!(
                        sessions = report.sessions_deleted,
                        blobs = report.blobs_deleted,
                        bytes = report.bytes_after,
                        "trajectory retention deleted sessions"
                    );
                }
            }
            Err(e) => {
                tracing::error!("trajectory retention failed: {e}");
                self.tails.clear();
            }
        }
    }

    /// Sets `usage.session_id` and tells the live view the session. A row that is
    /// not written yet is retried later.
    fn after_commit(&mut self, done: Done) {
        match backfill_usage(&self.main, &done.step_id, &done.session_id) {
            Ok(0) => {
                if self.backfill.len() >= BACKFILL_CAPACITY
                    && let Some(evicted) = self.backfill.pop_front()
                {
                    tracing::warn!(
                        step_id = %evicted.step_id,
                        session_id = %evicted.session_id,
                        capacity = BACKFILL_CAPACITY,
                        "usage session backfill queue full, dropping pending backfill for step"
                    );
                }
                self.backfill.push_back(Backfill {
                    step_id: done.step_id.clone(),
                    session_id: done.session_id.clone(),
                    since: Instant::now(),
                });
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("usage session backfill failed: {e}"),
        }
        self.live.set_session(&done.step_id, &done.session_id);
    }

    fn retry_backfill(&mut self) {
        if self.backfill.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut keep = VecDeque::with_capacity(self.backfill.len());
        while let Some(item) = self.backfill.pop_front() {
            if now.duration_since(item.since) > BACKFILL_TTL {
                tracing::warn!(
                    step_id = %item.step_id,
                    session_id = %item.session_id,
                    "usage session_id backfill abandoned: no usage row after {}s",
                    BACKFILL_TTL.as_secs()
                );
                continue;
            }
            match backfill_usage(&self.main, &item.step_id, &item.session_id) {
                Ok(0) => keep.push_back(item),
                Ok(_) => {}
                Err(e) => tracing::warn!("usage session backfill failed: {e}"),
            }
        }
        self.backfill = keep;
    }
}

/// What a committed record leaves behind for the steps after it.
struct Done {
    step_id: String,
    session_id: String,
    /// Tail to index in memory; `None` for metadata-only steps, which must never match.
    tail: Option<([u8; 32], TailEntry)>,
}

/// Writes one record inside its own SAVEPOINT. A failure rolls back this record only.
fn write_job(conn: &Connection, tails: &mut TailIndex, job: &Job) -> Option<Done> {
    if let Err(e) = conn.execute_batch(&format!("SAVEPOINT {SAVEPOINT}")) {
        tracing::warn!(step_id = %job.record.step_id, "trajectory savepoint failed: {e}");
        return None;
    }
    let outcome = write_record(conn, tails, job).and_then(|done| {
        conn.execute_batch(&format!("RELEASE {SAVEPOINT}"))?;
        Ok(done)
    });
    match outcome {
        Ok(Some(mut done)) => {
            if let Some((fp, entry)) = done.tail.take() {
                tails.insert(fp, entry);
            }
            Some(done)
        }
        Ok(None) => None,
        Err(e) => {
            let _ = conn.execute_batch(&format!("ROLLBACK TO {SAVEPOINT}; RELEASE {SAVEPOINT}"));
            tracing::warn!(step_id = %job.record.step_id, "trajectory record not stored: {e}");
            None
        }
    }
}

/// A side request, such as a client's title generator: no client session key, no
/// tools, and one user message with no history. Such a record is not stored at all.
fn is_side_only(rec: &CaptureRecord, prep: &Prepared) -> bool {
    rec.keys.session_key.is_none()
        && prep.tools.is_none()
        && matches!(
            prep.input.as_slice(),
            [only] if only.kind == ItemKind::Message && only.role.as_deref() == Some("user")
        )
}

/// Links one record and writes its session, thread, step, blobs and items.
/// Returns `None` when the record is skipped (see `is_side_only`).
fn write_record(conn: &Connection, tails: &TailIndex, job: &Job) -> CoreResult<Option<Done>> {
    let rec = &job.record;
    let prep = prepare(rec, job.metadata_only);
    if is_side_only(rec, &prep) {
        return Ok(None);
    }
    let res = link(
        &SqliteLinkStore { conn, tails },
        &StepInput {
            client: rec.client,
            keys: &rec.keys,
            requested_at: rec.requested_at,
            input: &prep.input,
            output: &prep.output,
            has_tools: prep.tools.is_some(),
            model: &rec.model,
        },
        rec.requested_at,
    );

    let session_id = match &res.session {
        SessionDecision::Existing(id) => id.clone(),
        SessionDecision::New {
            key_source,
            client_key,
        } => {
            let id = new_id();
            repo::insert_session(
                conn,
                &id,
                rec.client,
                client_key.as_deref(),
                *key_source,
                rec.requested_at,
            )?;
            id
        }
    };
    if let Some(title) = first_user_text(&prep.input) {
        repo::fill_session_title(conn, &session_id, &title)?;
    }

    let (thread_id, parent_step, link_prefix) = match &res.thread {
        ThreadDecision::Continue {
            thread_id,
            parent_step_id,
            prefix_len,
        } => (
            thread_id.clone(),
            Some(parent_step_id.as_str()),
            *prefix_len,
        ),
        ThreadDecision::New {
            kind,
            parent_thread_id,
            spawn_call_id,
            reset,
        } => {
            let id = new_id();
            repo::insert_thread(
                conn,
                &ThreadRow {
                    id: &id,
                    session_id: &session_id,
                    kind: *kind,
                    parent_thread_id: parent_thread_id.as_deref(),
                    spawn_call_id: spawn_call_id.as_deref(),
                    reset: *reset,
                    started_at: rec.requested_at,
                },
            )?;
            (id, None, 0)
        }
    };

    // A checkpoint stores the whole input and no parent (spec 4.4); other steps store
    // only the items after the matched prefix.
    let (prefix_stored, parent) = if res.checkpoint {
        (0, None)
    } else {
        (link_prefix, parent_step)
    };
    let stored_input = &prep.input[prefix_stored.min(prep.input.len())..];

    let mut written: u64 = 0;
    if let Some(item) = &prep.system {
        written += put_blob(conn, item)?;
    }
    if let Some(item) = &prep.tools {
        written += put_blob(conn, item)?;
    }
    for item in stored_input.iter().chain(&prep.output) {
        written += put_blob(conn, item)?;
    }

    let step_id = rec.step_id.as_str();
    let tail_fp = if prep.full {
        repo::fp_hex(&res.tail_fp)
    } else {
        format!("meta:{step_id}")
    };
    let usage = &rec.usage;
    repo::insert_step(
        conn,
        &StepRow {
            id: step_id,
            session_id: &session_id,
            thread_id: &thread_id,
            parent_step_id: parent,
            prefix_len: count(prefix_stored),
            checkpoint: res.checkpoint,
            depth: i64::from(res.depth),
            turn_no: i64::from(res.turn_no),
            client_turn_id: rec.keys.turn_id.as_deref(),
            requested_at: rec.requested_at,
            first_token_at: rec.first_token_at,
            finished_at: rec.finished_at,
            status: rec.status.map(i64::from),
            stop_reason: repo::stop_reason_column(rec.stop_reason),
            error: rec.error.as_deref(),
            inbound_protocol: rec.inbound.as_str(),
            upstream_protocol: rec.upstream.as_str(),
            provider_name: &rec.provider_name,
            model: &rec.model,
            upstream_model: &rec.upstream_model,
            route_mode: repo::route_mode_column(rec.route_mode),
            input_tokens: sat(usage.input_tokens),
            output_tokens: sat(usage.output_tokens),
            cached_tokens: sat(usage.cached_tokens),
            reasoning_tokens: sat(usage.reasoning_tokens),
            system_blob: prep.system.as_ref().map(|i| i.hash.as_str()),
            tools_blob: prep.tools.as_ref().map(|i| i.hash.as_str()),
            tail_fp: &tail_fp,
            capture: if prep.full { "full" } else { "metadata_only" },
        },
    )?;
    for (ord, item) in stored_input.iter().enumerate() {
        repo::insert_item(conn, step_id, "input", count(ord), item)?;
    }
    for (ord, item) in prep.output.iter().enumerate() {
        repo::insert_item(conn, step_id, "output", count(ord), item)?;
    }

    repo::add_session_totals(
        conn,
        &session_id,
        &SessionDelta {
            input_tokens: sat(usage.input_tokens),
            output_tokens: sat(usage.output_tokens),
            bytes: sat(written),
            at: rec.requested_at,
        },
    )?;
    // The first tool call of a session without a main thread makes its thread the main one.
    if prep
        .output
        .iter()
        .any(|item| item.kind == ItemKind::ToolCall)
    {
        repo::promote_to_main(conn, &thread_id, &session_id)?;
    }

    let tail = prep.full.then(|| {
        (
            res.tail_fp,
            TailEntry {
                step_id: step_id.to_owned(),
                session_id: session_id.clone(),
                thread_id: thread_id.clone(),
                client: rec.client,
                last_at: rec.requested_at,
                tail_len: prep.input.len() + prep.output.len(),
                turn_no: res.turn_no,
                depth: res.depth,
                client_turn_id: rec.keys.turn_id.clone(),
            },
        )
    });
    Ok(Some(Done {
        step_id: step_id.to_owned(),
        session_id,
        tail,
    }))
}

/// A record reduced to what the writer stores.
struct Prepared {
    /// False when only metadata is stored (no bodies, no items).
    full: bool,
    input: Vec<NormalizedItem>,
    output: Vec<NormalizedItem>,
    system: Option<NormalizedItem>,
    tools: Option<NormalizedItem>,
}

impl Prepared {
    fn metadata() -> Self {
        Self {
            full: false,
            input: Vec::new(),
            output: Vec::new(),
            system: None,
            tools: None,
        }
    }
}

fn prepare(rec: &CaptureRecord, metadata_only: bool) -> Prepared {
    if metadata_only {
        return Prepared::metadata();
    }
    let Some(body) = &rec.request_body else {
        return Prepared::metadata();
    };
    let request = match serde_json::from_slice::<Value>(body)
        .map_err(CoreError::from)
        .and_then(|v| to_ir(rec.inbound, v))
    {
        Ok(request) => request,
        Err(e) => {
            tracing::warn!(step_id = %rec.step_id, "trajectory request not parsed, storing metadata only: {e}");
            return Prepared::metadata();
        }
    };
    Prepared {
        full: true,
        input: request.input.iter().map(normalize_item).collect(),
        output: response_output(rec),
        system: request
            .instructions
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(normalize_system),
        tools: normalize_tools(&request.tools),
    }
}

/// Output items of the step: the collected IR when there is one, otherwise the raw
/// upstream body parsed with the same translation as the client response.
fn response_output(rec: &CaptureRecord) -> Vec<NormalizedItem> {
    let collected = rec
        .response
        .as_ref()
        .map(|r| &r.output)
        .filter(|o| !o.is_empty());
    if let Some(output) = collected {
        return output.iter().map(normalize_item).collect();
    }
    rec.response_body
        .as_deref()
        .and_then(|raw| serde_json::from_slice::<Value>(raw).ok())
        .and_then(|v| translate_response_parsed(rec.inbound, rec.upstream, v).ok())
        .and_then(|(_, _, parsed)| parsed)
        .map(|r| r.output.iter().map(normalize_item).collect())
        .unwrap_or_default()
}

/// Sets `usage.session_id` for the request's usage row. Returns the number of rows changed.
fn backfill_usage(main: &Db, step_id: &str, session_id: &str) -> CoreResult<usize> {
    main.with_conn(|c| {
        Ok(c.execute(
            "UPDATE usage SET session_id = ?1 WHERE step_id = ?2",
            params![session_id, step_id],
        )?)
    })
}

fn put_blob(conn: &Connection, item: &NormalizedItem) -> CoreResult<u64> {
    Ok(blobs::put_measured(conn, item)?.unwrap_or(0))
}

/// Removes orphaned blobs after a manual delete. A failure is logged: the delete itself
/// has already committed, and the next retention pass collects the blobs.
fn collect_orphans_logged(conn: &Connection) {
    if let Err(e) = retention::collect_orphans(conn) {
        tracing::warn!("trajectory orphan blob cleanup failed: {e}");
    }
}

/// Wall-clock time in milliseconds, the unit of `requested_at` and `last_at`.
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

fn len_of(body: &Option<Bytes>) -> usize {
    body.as_ref().map_or(0, Bytes::len)
}

fn new_id() -> String {
    Uuid::new_v4().to_string()
}

fn sat(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn count(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// Adds one pass's freed bytes to the run's running total and vacuums once that total is
/// more than 25% of the library (spec 5.3). A vacuum resets the total. Returns whether it ran.
fn vacuum_after_pass(
    conn: &Connection,
    freed_since_vacuum: &mut u64,
    freed: u64,
    library_bytes: u64,
) -> CoreResult<bool> {
    *freed_since_vacuum = freed_since_vacuum.saturating_add(freed);
    let vacuumed = retention::vacuum_if_needed(conn, *freed_since_vacuum, library_bytes)?;
    if vacuumed {
        *freed_since_vacuum = 0;
    }
    Ok(vacuumed)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::Mutex;

    use super::*;
    use crate::domain::KeySource;
    use crate::trajectory::link::{LinkStore, ThreadKind};

    /// Collects tracing output in memory so a test can assert on a warning.
    #[derive(Clone, Default)]
    struct Logs(Arc<Mutex<Vec<u8>>>);

    impl Logs {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    impl Write for Logs {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Logs {
        type Writer = Logs;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn worker() -> Worker {
        Worker {
            db: TrajectoryDb::open_in_memory().unwrap().0,
            main: Db::open_in_memory().unwrap(),
            live: LiveRegistry::default(),
            tails: TailIndex::default(),
            backfill: VecDeque::new(),
            inflight: Arc::new(AtomicUsize::new(0)),
            settings: Arc::new(ArcSwap::from_pointee(TrajectorySettings::default())),
            steps_since_retention: 0,
            freed_since_vacuum: 0,
        }
    }

    const DAY_MS: i64 = 86_400_000;

    #[test]
    fn retention_evicts_tails_of_deleted_sessions() {
        let mut w = worker();
        let now = 100 * DAY_MS;
        w.db.with_conn(|conn| {
            // Default retention is 30 days, so the session last seen at 0 is expired.
            repo::insert_session(conn, "old", Some(Client::Codex), None, KeySource::New, 0)?;
            repo::insert_session(
                conn,
                "fresh",
                Some(Client::Codex),
                None,
                KeySource::New,
                now,
            )?;
            Ok(())
        })
        .unwrap();
        let tail = |step: &str, session: &str| TailEntry {
            step_id: step.into(),
            session_id: session.into(),
            thread_id: "t".into(),
            client: Some(Client::Codex),
            last_at: 0,
            tail_len: 1,
            turn_no: 1,
            depth: 0,
            client_turn_id: None,
        };
        w.tails.insert([1; 32], tail("step-old", "old"));
        w.tails.insert([2; 32], tail("step-fresh", "fresh"));

        w.apply_retention(now);

        assert!(
            w.tails.get(&[1; 32]).is_none(),
            "tail of a deleted session is still indexed"
        );
        assert_eq!(w.tails.get(&[2; 32]).unwrap().session_id, "fresh");
    }

    fn capturing<F: FnOnce()>(f: F) -> String {
        let logs = Logs::default();
        let sub = tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_ansi(false)
            .finish();
        tracing::subscriber::with_default(sub, f);
        logs.text()
    }

    #[test]
    fn abandoned_backfill_logs_step_and_is_dropped() {
        let mut w = worker();
        let since = Instant::now()
            .checked_sub(BACKFILL_TTL + Duration::from_secs(1))
            .expect("instant in range");
        w.backfill.push_back(Backfill {
            step_id: "step-expired".into(),
            session_id: "sess-1".into(),
            since,
        });

        let out = capturing(|| w.retry_backfill());

        assert!(w.backfill.is_empty());
        assert!(out.contains("step-expired"), "{out}");
        assert!(out.contains("usage session_id backfill abandoned"), "{out}");
    }

    #[test]
    fn full_backfill_queue_logs_evicted_step() {
        let mut w = worker();
        for i in 0..BACKFILL_CAPACITY {
            w.backfill.push_back(Backfill {
                step_id: format!("step-{i}"),
                session_id: "sess-1".into(),
                since: Instant::now(),
            });
        }
        let done = Done {
            step_id: "step-new".into(),
            session_id: "sess-1".into(),
            tail: None,
        };

        let out = capturing(|| w.after_commit(done));

        assert_eq!(w.backfill.len(), BACKFILL_CAPACITY);
        assert_eq!(w.backfill.front().unwrap().step_id, "step-1");
        assert!(out.contains("step-0"), "{out}");
        assert!(out.contains("queue full"), "{out}");
    }

    /// Inserts one thread and one step of `session` whose stored tail is `tail_fp`.
    fn seed_step(conn: &Connection, session: &str, step: &str, tail_fp: &[u8; 32]) {
        let thread = format!("t-{step}");
        repo::insert_thread(
            conn,
            &ThreadRow {
                id: &thread,
                session_id: session,
                kind: ThreadKind::Main,
                parent_thread_id: None,
                spawn_call_id: None,
                reset: false,
                started_at: 0,
            },
        )
        .unwrap();
        let tail = repo::fp_hex(tail_fp);
        repo::insert_step(
            conn,
            &StepRow {
                id: step,
                session_id: session,
                thread_id: &thread,
                parent_step_id: None,
                prefix_len: 0,
                checkpoint: false,
                depth: 0,
                turn_no: 1,
                client_turn_id: None,
                requested_at: 0,
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
                tail_fp: &tail,
                capture: "full",
            },
        )
        .unwrap();
    }

    #[test]
    fn stored_tail_of_deleted_session_is_not_found() {
        let mut w = worker();
        let now = 100 * DAY_MS;
        let stored_fp = [3; 32];
        w.db.with_conn(|conn| {
            repo::insert_session(conn, "old", Some(Client::Codex), None, KeySource::New, 0)?;
            seed_step(conn, "old", "step-old-db", &stored_fp);
            // Only the database knows this tail, so the lookup has to reach the steps table.
            let store = SqliteLinkStore {
                conn,
                tails: &w.tails,
            };
            assert!(store.tail_by_fp(&stored_fp).is_some());
            Ok(())
        })
        .unwrap();

        w.apply_retention(now);

        w.db.with_conn(|conn| {
            assert_eq!(repo::session_count(conn)?, 0);
            let store = SqliteLinkStore {
                conn,
                tails: &w.tails,
            };
            assert!(store.tail_by_fp(&stored_fp).is_none());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn freed_bytes_accumulate_until_vacuum() {
        let db = TrajectoryDb::open_in_memory().unwrap();
        db.0.with_conn(|conn| {
            let mut run = 0;
            // 20 of 100 bytes is under 25%: no vacuum, and the total is kept for later passes.
            assert!(!vacuum_after_pass(conn, &mut run, 20, 100)?);
            assert_eq!(run, 20);
            // The second pass brings the run to 40 of 100 bytes: vacuum runs and the total restarts.
            assert!(vacuum_after_pass(conn, &mut run, 20, 100)?);
            assert_eq!(run, 0);
            Ok(())
        })
        .unwrap();
    }

    fn capture_record(step_id: &str) -> CaptureRecord {
        CaptureRecord {
            step_id: step_id.into(),
            client: Some(Client::Codex),
            keys: Default::default(),
            inbound: crate::domain::Protocol::Responses,
            upstream: crate::domain::Protocol::Responses,
            provider_name: "p".into(),
            model: "m".into(),
            upstream_model: "m".into(),
            route_mode: None,
            // Recent, so the retention pass this commit triggers keeps the session.
            requested_at: now_millis(),
            first_token_at: None,
            finished_at: None,
            status: Some(200),
            stop_reason: crate::trajectory::record::StopReason::End,
            error: None,
            usage: Default::default(),
            request_body: None,
            response: None,
            response_body: None,
        }
    }

    #[test]
    fn retention_pass_resets_step_counter_after_256_steps() {
        let mut w = worker();
        w.steps_since_retention = RETENTION_EVERY_STEPS - 1;
        let job = Job {
            record: capture_record("step-trigger"),
            charged: 0,
            metadata_only: true,
        };

        w.commit(&mut vec![job]);

        // The 256th committed step ran a retention pass, which resets the counter.
        assert_eq!(w.steps_since_retention, 0);
        w.db.with_conn(|conn| {
            assert_eq!(repo::session_count(conn)?, 1, "the job was not committed");
            Ok(())
        })
        .unwrap();
    }
}
