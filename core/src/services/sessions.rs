//! Session and trajectory services (spec 7): listing, detail, projected trajectory,
//! step context, blobs, change feed, deletion, and the trajectory database lifecycle
//! (availability, rebuild, clear).
//!
//! Reads run on the blocking pool against the trajectory database. Deletes go through the
//! writer thread, which owns the in-memory tails (spec 4.3, 5.3). Clear-all and rebuild
//! replace the database file, so they stop the writer first.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::trajectory_view::{
    Heads, PageSource, StepSource, ToolResult, build_context, project_page, step_header,
};
use super::{Services, lock};
use crate::domain::{
    BlobView, Client, LivePhase, SessionChanges, SessionDetail, SessionListInput, SessionPage,
    StepContext, ThreadKindView, ThreadView, TrajectoryInput, TrajectoryPage, TrajectorySettings,
    TrajectoryStatus,
};
use crate::error::{CoreError, CoreResult};
use crate::proxy::live::LiveRegistry;
use crate::storage::Db;
use crate::storage::settings as settings_repo;
use crate::storage::trajectory::TrajectoryDb;
use crate::storage::trajectory::blobs;
use crate::storage::trajectory::repo::{self, ItemData, ItemRead, SessionQuery, ThreadRead};
use crate::storage::trajectory::writer::{
    TrajectorySink, TrajectoryWriterHandle, spawn_trajectory_writer,
};
use crate::trajectory::record::{CaptureSink, CaptureSlot};

const DEFAULT_PAGE: u32 = 50;
const MAX_PAGE: u32 = 200;
const DEFAULT_TURNS: u32 = 20;

/// The trajectory database as the services see it.
pub(crate) enum TrajectoryState {
    Ready {
        /// Reader handle; the writer thread owns its own clone.
        db: TrajectoryDb,
        sink: Arc<TrajectorySink>,
        writer: TrajectoryWriterHandle,
    },
    Unavailable {
        error: String,
    },
}

/// Opens the trajectory database at `path` and starts its writer. A failure does not
/// stop the service: the state records the reason and capture stays off (spec 5.4).
pub(crate) fn open_trajectory(
    path: &Path,
    main: &Db,
    live: &LiveRegistry,
    capture: &CaptureSlot,
) -> TrajectoryState {
    match TrajectoryDb::open(path) {
        Ok(tdb) => start_writer(tdb, main, live, capture),
        Err(e) => {
            tracing::error!("trajectory database unavailable, capture disabled: {e}");
            TrajectoryState::Unavailable { error: e.message() }
        }
    }
}

/// Starts the writer over an open database and points the capture slot at it.
fn start_writer(
    tdb: TrajectoryDb,
    main: &Db,
    live: &LiveRegistry,
    capture: &CaptureSlot,
) -> TrajectoryState {
    let settings = main
        .with_conn(settings_repo::get_trajectory)
        .unwrap_or_else(|e| {
            tracing::warn!("trajectory settings unreadable, using defaults: {e}");
            TrajectorySettings::default()
        });
    let reader = TrajectoryDb(tdb.0.clone());
    let (sink, writer) = spawn_trajectory_writer(tdb, main.clone(), live.clone(), settings);
    capture.set(Some(Arc::clone(&sink) as Arc<dyn CaptureSink>));
    TrajectoryState::Ready {
        db: reader,
        sink,
        writer,
    }
}

impl TrajectoryState {
    /// Stops the writer, if one runs. Dropping the state releases the reader.
    pub(crate) fn shutdown(self) {
        if let TrajectoryState::Ready { writer, .. } = self {
            writer.shutdown();
        }
    }
}

/// The conflict every session method returns while the trajectory database is down.
pub(crate) fn unavailable(reason: &str) -> CoreError {
    CoreError::Conflict(format!("轨迹库不可用：{reason}"))
}

fn not_found(what: &str, id: &str) -> CoreError {
    CoreError::NotFound(format!("{what}不存在：{id}"))
}

impl Services {
    /// Reader handle for the trajectory database, or the conflict that explains why it is down.
    pub(crate) fn trajectory_reader(&self) -> CoreResult<Db> {
        match &*self.lock_trajectory() {
            TrajectoryState::Ready { db, .. } => Ok(db.0.clone()),
            TrajectoryState::Unavailable { error } => Err(unavailable(error)),
        }
    }

    fn lock_trajectory(&self) -> MutexGuard<'_, TrajectoryState> {
        lock(&self.trajectory)
    }

    /// Producer slot the proxy captures into; shared with the proxy state.
    pub fn capture_slot(&self) -> CaptureSlot {
        self.capture.clone()
    }

    /// Live request registry shared with the proxy state.
    pub fn live_registry(&self) -> LiveRegistry {
        self.live.clone()
    }

    /// Sessions with a request in flight.
    fn live_session_ids(&self) -> HashSet<String> {
        self.live
            .snapshot(0)
            .in_flight
            .iter()
            .filter_map(|r| r.session_id.clone())
            .collect()
    }

    pub async fn list_sessions(
        self: &Arc<Self>,
        input: SessionListInput,
    ) -> CoreResult<SessionPage> {
        self.run(move |s| {
            let db = s.trajectory_reader()?;
            let limit = page_size(input.limit)?;
            let after = input.cursor.as_deref().map(parse_cursor).transpose()?;
            let query = SessionQuery {
                client: input.client,
                title: input
                    .query
                    .map(|q| q.trim().to_owned())
                    .filter(|q| !q.is_empty()),
                model: input.model.filter(|m| !m.trim().is_empty()),
                start_ms: input.start_ms,
                end_ms: input.end_ms,
                after,
            };
            let live = s.live_session_ids();
            db.with_conn(|c| {
                let mut items = repo::list_sessions(c, &query, limit + 1)?;
                let more = items.len() > limit as usize;
                items.truncate(limit as usize);
                for item in &mut items {
                    item.live = live.contains(&item.id);
                }
                let next_cursor = more
                    .then(|| {
                        items
                            .last()
                            .map(|last| format!("{}:{}", last.last_at, last.id))
                    })
                    .flatten();
                Ok(SessionPage { items, next_cursor })
            })
        })
        .await
    }

    pub async fn get_session(self: &Arc<Self>, id: String) -> CoreResult<SessionDetail> {
        self.run(move |s| {
            let db = s.trajectory_reader()?;
            let live = s.live_session_ids();
            db.with_conn(|c| {
                let mut summary =
                    repo::session_summary(c, &id)?.ok_or_else(|| not_found("会话", &id))?;
                summary.live = live.contains(&id);
                let threads = repo::session_threads(c, &id)?
                    .iter()
                    .map(thread_view)
                    .collect();
                Ok(SessionDetail { summary, threads })
            })
        })
        .await
    }

    pub async fn get_trajectory(
        self: &Arc<Self>,
        input: TrajectoryInput,
    ) -> CoreResult<TrajectoryPage> {
        self.run(move |s| {
            let db = s.trajectory_reader()?;
            let limit = match input.limit_turns {
                None => DEFAULT_TURNS,
                Some(n) if (1..=MAX_PAGE).contains(&n) => n,
                Some(_) => {
                    return Err(CoreError::Validation(
                        "每页回合数必须在 1 到 200 之间".into(),
                    ));
                }
            };
            let snapshot = s.live.snapshot(0);
            let live_phases: HashMap<String, LivePhase> = snapshot
                .in_flight
                .iter()
                .chain(snapshot.completed.iter())
                .map(|r| (r.step_id.clone(), r.phase))
                .collect();
            let session_live: Vec<_> = snapshot
                .in_flight
                .iter()
                .filter(|r| r.session_id.as_deref() == Some(input.session_id.as_str()))
                .cloned()
                .collect();
            db.with_conn(|c| {
                if !repo::session_exists(c, &input.session_id)? {
                    return Err(not_found("会话", &input.session_id));
                }
                let Some(thread_id) =
                    repo::resolve_thread(c, &input.session_id, input.thread_id.as_deref())?
                else {
                    if let Some(requested) = &input.thread_id {
                        return Err(not_found("线程", requested));
                    }
                    return Ok(TrajectoryPage {
                        turns: Vec::new(),
                        has_older: false,
                        seq: 0,
                    });
                };
                let thread = repo::session_threads(c, &input.session_id)?
                    .into_iter()
                    .find(|t| t.id == thread_id)
                    .ok_or_else(|| not_found("线程", &thread_id))?;

                // Turn window: the newest `limit` turns below `before_turn`.
                let before = input.before_turn.map(i64::from);
                let mut turns = repo::turn_numbers(c, &thread_id, before, i64::from(limit) + 1)?;
                let has_older = turns.len() > limit as usize;
                turns.truncate(limit as usize);
                let hi = turns.first().copied().unwrap_or(0);
                let lo = turns.last().copied().unwrap_or(0);

                let steps_read = repo::thread_steps(c, &thread_id, lo, hi)?;
                let numbers = repo::step_numbers(c, &input.session_id)?;
                let source =
                    page_source(c, &input.session_id, &thread, &steps_read, &numbers, lo, hi)?;
                // In-flight placeholders belong to the Main thread on the newest page only.
                let newest = before.is_none();
                let on_main = thread.kind == "main";
                let src = PageSource {
                    live: if newest && on_main { session_live } else { Vec::new() },
                    live_phases,
                    session_steps: u32::try_from(numbers.len()).unwrap_or(u32::MAX),
                    ..source
                };
                let seq = steps_read.iter().map(|st| st.seq).max().unwrap_or(0);
                Ok(TrajectoryPage {
                    turns: project_page(&src),
                    has_older,
                    seq: u64::try_from(seq).unwrap_or(0),
                })
            })
        })
        .await
    }

    pub async fn get_step_context(self: &Arc<Self>, step_id: String) -> CoreResult<StepContext> {
        self.run(move |s| {
            let db = s.trajectory_reader()?;
            let phases: HashMap<String, LivePhase> = {
                let snapshot = s.live.snapshot(0);
                snapshot
                    .in_flight
                    .iter()
                    .chain(snapshot.completed.iter())
                    .map(|r| (r.step_id.clone(), r.phase))
                    .collect()
            };
            db.with_conn(|c| {
                let step =
                    repo::step_by_id(c, &step_id)?.ok_or_else(|| not_found("步骤", &step_id))?;
                let numbers = repo::step_numbers(c, &step.session_id)?;
                let mut chain = Vec::new();
                for id in repo::context_chain(c, &step_id)? {
                    let (input, output) = split_sides(c, &id)?;
                    chain.push((input, output));
                }
                let system_json = match &step.system_blob {
                    Some(hash) => repo::blob_json(c, hash)?.map(|(json, _)| json),
                    None => None,
                };
                let tools_json = match &step.tools_blob {
                    Some(hash) => repo::blob_json(c, hash)?.map(|(json, _)| json),
                    None => None,
                };
                let options =
                    serde_json::from_str::<Value>(&step.options_json).unwrap_or_else(|_| json!({}));
                let number = numbers.get(&step.id).copied().unwrap_or(0);
                let header = step_header(&step, number, phases.get(&step.id).copied());
                Ok(build_context(
                    header,
                    &chain,
                    system_json.as_deref(),
                    tools_json.as_deref(),
                    options,
                ))
            })
        })
        .await
    }

    pub async fn get_blob(self: &Arc<Self>, hash: String) -> CoreResult<BlobView> {
        self.run(move |s| {
            let db = s.trajectory_reader()?;
            db.with_conn(|c| {
                let stored = blobs::get(c, &hash)?.ok_or_else(|| not_found("正文", &hash))?;
                Ok(BlobView {
                    kind: stored.kind,
                    raw_bytes: stored.raw_bytes,
                    truncated: stored.truncated,
                    json: stored.json,
                })
            })
        })
        .await
    }

    pub async fn session_changes(self: &Arc<Self>, since_seq: u64) -> CoreResult<SessionChanges> {
        self.run(move |s| {
            let db = s.trajectory_reader()?;
            let since = i64::try_from(since_seq).unwrap_or(i64::MAX);
            db.with_conn(|c| {
                // Cursor first: steps committed after this read have seq above it and come
                // back on the next call, so none is lost between the two reads.
                let cursor = repo::last_seq(c)?;
                let session_ids = repo::changed_sessions(c, since, cursor)?;
                Ok(SessionChanges {
                    session_ids,
                    seq: u64::try_from(cursor).unwrap_or(0),
                })
            })
        })
        .await
    }

    pub async fn delete_session(self: &Arc<Self>, id: String) -> CoreResult<()> {
        self.run(move |s| {
            let state = s.lock_trajectory();
            match &*state {
                TrajectoryState::Ready { writer, .. } => writer.delete_session(&id),
                TrajectoryState::Unavailable { error } => Err(unavailable(error)),
            }
        })
        .await
    }

    /// Deletes every session of `client`, or every session when `client` is `None`.
    /// Returns the number of sessions removed.
    pub async fn clear_sessions(self: &Arc<Self>, client: Option<Client>) -> CoreResult<u32> {
        self.run(move |s| match client {
            Some(c) => {
                let state = s.lock_trajectory();
                match &*state {
                    TrajectoryState::Ready { writer, .. } => writer.clear_client(c),
                    TrajectoryState::Unavailable { error } => Err(unavailable(error)),
                }
            }
            None => s.clear_all(),
        })
        .await
    }

    /// Clears everything by replacing the database file: stop the writer, delete the file,
    /// create a fresh database and start a new writer. Requests that arrive meanwhile are
    /// not captured, but they still succeed.
    fn clear_all(&self) -> CoreResult<u32> {
        let mut state = self.lock_trajectory();
        if let TrajectoryState::Unavailable { error } = &*state {
            return Err(unavailable(error));
        }
        self.capture.set(None);
        let previous = std::mem::replace(
            &mut *state,
            TrajectoryState::Unavailable {
                error: "正在清空轨迹库".into(),
            },
        );
        // Stopping the writer commits the records still queued, so the count is read after it.
        let removed = match previous {
            TrajectoryState::Ready { db, writer, .. } => {
                writer.shutdown();
                db.0.with_conn(repo::session_count).unwrap_or_else(|e| {
                    tracing::warn!("trajectory session count before clear failed: {e}");
                    0
                })
            }
            TrajectoryState::Unavailable { .. } => 0,
        };
        let path = self.paths.trajectory_db.clone();
        if let Err(e) = remove_db_files(&path) {
            *state = open_trajectory(&path, &self.db, &self.live, &self.capture);
            return Err(e);
        }
        *state = open_trajectory(&path, &self.db, &self.live, &self.capture);
        Ok(removed)
    }

    pub async fn trajectory_status(self: &Arc<Self>) -> CoreResult<TrajectoryStatus> {
        self.run(move |s| {
            let settings = s.db.with_conn(settings_repo::get_trajectory)?;
            let state = s.lock_trajectory();
            match &*state {
                TrajectoryState::Ready { db, .. } => db.0.with_conn(|c| {
                    Ok(TrajectoryStatus {
                        available: true,
                        error: None,
                        bytes: blobs::total_bytes(c)?,
                        sessions: repo::session_count(c)?,
                        settings: settings.clone(),
                    })
                }),
                TrajectoryState::Unavailable { error } => Ok(TrajectoryStatus {
                    available: false,
                    error: Some(error.clone()),
                    bytes: 0,
                    sessions: 0,
                    settings,
                }),
            }
        })
        .await
    }

    /// Saves the settings (validated by the storage layer) and applies them to the writer.
    /// Works while the trajectory database is unavailable.
    pub async fn save_trajectory_settings(
        self: &Arc<Self>,
        s: TrajectorySettings,
    ) -> CoreResult<()> {
        self.run(move |svc| {
            svc.db
                .with_conn(|c| settings_repo::save_trajectory(c, &s))?;
            if let TrajectoryState::Ready { sink, .. } = &*svc.lock_trajectory() {
                sink.set_settings(&s);
            }
            Ok(())
        })
        .await
    }

    /// Replaces the trajectory database with an empty one. An existing file is renamed to
    /// `trajectory.db.broken-<timestamp>` rather than deleted, so it can be inspected.
    pub async fn rebuild_trajectory_db(self: &Arc<Self>) -> CoreResult<()> {
        self.run(move |s| {
            let mut state = s.lock_trajectory();
            s.capture.set(None);
            let previous = std::mem::replace(
                &mut *state,
                TrajectoryState::Unavailable {
                    error: "正在重建轨迹库".into(),
                },
            );
            previous.shutdown();
            let path = s.paths.trajectory_db.clone();
            if let Err(e) = move_aside(&path, now_millis()) {
                *state = open_trajectory(&path, &s.db, &s.live, &s.capture);
                return Err(e);
            }
            *state = open_trajectory(&path, &s.db, &s.live, &s.capture);
            match &*state {
                TrajectoryState::Ready { .. } => Ok(()),
                TrajectoryState::Unavailable { error } => Err(unavailable(error)),
            }
        })
        .await
    }
}

/// Page size for session lists: default 50, at most 200.
fn page_size(limit: Option<u32>) -> CoreResult<u32> {
    match limit {
        None => Ok(DEFAULT_PAGE),
        Some(n) if (1..=MAX_PAGE).contains(&n) => Ok(n),
        Some(_) => Err(CoreError::Validation("每页数量必须在 1 到 200 之间".into())),
    }
}

/// Parses the keyset cursor `"{last_at}:{id}"`.
fn parse_cursor(cursor: &str) -> CoreResult<(i64, String)> {
    let invalid = || CoreError::Validation("会话列表游标无效".into());
    let (at, id) = cursor.split_once(':').ok_or_else(invalid)?;
    let at: i64 = at.parse().map_err(|_| invalid())?;
    if id.is_empty() {
        return Err(invalid());
    }
    Ok((at, id.to_owned()))
}

/// Thread kind as the detail shows it. A pending thread is a side thread when it is a
/// single step without tools, otherwise it is shown as other (spec 4.4).
fn thread_view(t: &ThreadRead) -> ThreadView {
    let kind = match t.kind.as_str() {
        "main" => ThreadKindView::Main,
        "subagent" => ThreadKindView::Subagent,
        "side" => ThreadKindView::Side,
        _ if t.step_count == 1 && !t.has_tools => ThreadKindView::Side,
        _ => ThreadKindView::Other,
    };
    ThreadView {
        id: t.id.clone(),
        kind,
        parent_thread_id: t.parent_thread_id.clone(),
        spawn_call_id: t.spawn_call_id.clone(),
        reset: t.reset,
        step_count: u32::try_from(t.step_count).unwrap_or(u32::MAX),
        started_at: t.started_at,
    }
}

/// Gathers everything a thread page needs: the steps' items (only the ones each step added),
/// tool results, subagent links, system prompts and the heads of the step before the page.
fn page_source(
    c: &rusqlite::Connection,
    session_id: &str,
    thread: &ThreadRead,
    steps_read: &[repo::StepRead],
    numbers: &HashMap<String, u32>,
    lo: i64,
    hi: i64,
) -> CoreResult<PageSource> {
    // A checkpoint stores its whole input, so its new items start after the previous
    // step's full context: prefix + stored input + stored output of that step.
    let mut new_start: HashMap<String, i64> = HashMap::new();
    for st in steps_read.iter().filter(|st| st.checkpoint) {
        let start = match repo::previous_step(c, &thread.id, st.seq)? {
            Some(prev) => prev.prefix_len + prev.input_count + prev.output_count,
            None => 0,
        };
        new_start.insert(st.id.clone(), start);
    }

    let item_rows = repo::thread_items(c, &thread.id, lo, hi)?;
    let mut kept: Vec<ItemRead> = Vec::with_capacity(item_rows.len());
    for row in item_rows {
        if row.side == "input"
            && let Some(start) = new_start.get(&row.step_id)
            && row.ord < *start
        {
            continue;
        }
        kept.push(row);
    }
    let call_ids: Vec<String> = kept
        .iter()
        .filter(|r| r.side == "output" && r.kind == "tool_call")
        .filter_map(|r| r.call_id.clone())
        .collect();
    let loaded = repo::load_items(c, kept)?;
    let mut inputs: HashMap<String, Vec<ItemData>> = HashMap::new();
    let mut outputs: HashMap<String, Vec<ItemData>> = HashMap::new();
    for item in loaded {
        let side = if item.read.side == "input" {
            &mut inputs
        } else {
            &mut outputs
        };
        side.entry(item.read.step_id.clone())
            .or_default()
            .push(item);
    }

    let results = first_results(c, session_id, &call_ids)?;
    let subthreads = repo::spawn_links(c, session_id)?.into_iter().collect();

    let mut system_json = HashMap::new();
    for st in steps_read {
        if let Some(hash) = &st.system_blob
            && let Some((json, _)) = repo::blob_json(c, hash)?
        {
            system_json.insert(hash.clone(), json);
        }
    }

    let before = match steps_read.first() {
        Some(first) => repo::previous_step(c, &thread.id, first.seq)?.map(|p| Heads::of(&p)),
        None => None,
    };
    let steps = steps_read
        .iter()
        .map(|st| StepSource {
            step: st.clone(),
            number: numbers.get(&st.id).copied().unwrap_or(0),
            input: inputs.remove(&st.id).unwrap_or_default(),
            output: outputs.remove(&st.id).unwrap_or_default(),
        })
        .collect();
    Ok(PageSource {
        thread_id: thread.id.clone(),
        reset: thread.reset,
        steps,
        before,
        results,
        subthreads,
        system_json,
        live_phases: HashMap::new(),
        live: Vec::new(),
        session_steps: 0,
    })
}

/// The first result of each call, decoded.
fn first_results(
    c: &rusqlite::Connection,
    session_id: &str,
    call_ids: &[String],
) -> CoreResult<HashMap<String, ToolResult>> {
    let mut seen = HashSet::new();
    let mut firsts = Vec::new();
    for read in repo::tool_results(c, session_id, call_ids)? {
        let call = read.item.call_id.clone().unwrap_or_default();
        if seen.insert(call.clone()) {
            firsts.push((call, read));
        }
    }
    let decoded = repo::load_items(c, firsts.iter().map(|(_, r)| r.item.clone()).collect())?;
    Ok(firsts
        .into_iter()
        .zip(decoded)
        .map(|((call, read), item)| {
            (
                call,
                ToolResult {
                    step_id: read.item.step_id.clone(),
                    requested_at: read.requested_at,
                    item,
                },
            )
        })
        .collect())
}

/// Input and output items of one step, decoded, with the full stored input.
fn split_sides(
    c: &rusqlite::Connection,
    step_id: &str,
) -> CoreResult<(Vec<ItemData>, Vec<ItemData>)> {
    let rows = repo::step_items(c, step_id)?;
    let loaded = repo::load_items(c, rows)?;
    let (input, output): (Vec<ItemData>, Vec<ItemData>) =
        loaded.into_iter().partition(|i| i.read.side == "input");
    Ok((input, output))
}

/// Removes the database file and its WAL and shared-memory files. Missing files are fine.
fn remove_db_files(path: &Path) -> CoreResult<()> {
    for suffix in ["", "-wal", "-shm"] {
        let file = PathBuf::from(format!("{}{suffix}", path.display()));
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(CoreError::Io(format!(
                    "无法删除轨迹库文件 {}：{e}",
                    file.display()
                )));
            }
        }
    }
    Ok(())
}

/// Renames the database file and its sidecars to `<name>.broken-<stamp>`, so the old data
/// is kept for inspection.
fn move_aside(path: &Path, stamp: i64) -> CoreResult<()> {
    for suffix in ["", "-wal", "-shm"] {
        let from = PathBuf::from(format!("{}{suffix}", path.display()));
        if !from.exists() {
            continue;
        }
        let to = PathBuf::from(format!("{}.broken-{stamp}{suffix}", path.display()));
        std::fs::rename(&from, &to)
            .map_err(|e| CoreError::Io(format!("无法重命名轨迹库文件 {}：{e}", from.display())))?;
    }
    Ok(())
}

/// Wall-clock milliseconds, used to name moved-aside files.
fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}
