//! Batched, non-blocking usage writer: requests enqueue events, a dedicated
//! thread persists them in batches.

use super::{Db, usage};
use crate::domain::UsageEvent;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const QUEUE_CAPACITY: usize = 4096;
const BATCH_SIZE: usize = 256;
const FLUSH_INTERVAL: Duration = Duration::from_millis(500);

/// Producer side. Cheap to clone; `record` never blocks.
#[derive(Clone)]
pub struct UsageSink {
    tx: SyncSender<UsageEvent>,
}

impl UsageSink {
    /// Enqueues the event; when the queue is full (or the writer is gone) the event
    /// is dropped with a warning rather than blocking the request.
    pub fn record(&self, e: UsageEvent) {
        if let Err(err) = self.tx.try_send(e) {
            tracing::warn!("usage queue unavailable, dropping record: {err}");
        }
    }
}

/// Owns the writer thread.
pub struct UsageWriterHandle {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl UsageWriterHandle {
    /// Flushes everything queued so far and stops the writer thread.
    /// Events recorded after this call may be dropped.
    pub fn shutdown(self) {
        self.stop.store(true, Ordering::SeqCst);
        if self.thread.join().is_err() {
            tracing::error!("usage writer thread panicked");
        }
    }
}

pub fn spawn_usage_writer(db: Db) -> (UsageSink, UsageWriterHandle) {
    let (tx, rx) = sync_channel(QUEUE_CAPACITY);
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let thread = std::thread::Builder::new()
        .name("usage-writer".into())
        .spawn(move || run(&db, &rx, &flag))
        .expect("spawn usage writer thread");
    (UsageSink { tx }, UsageWriterHandle { stop, thread })
}

fn run(db: &Db, rx: &Receiver<UsageEvent>, stop: &AtomicBool) {
    let mut batch: Vec<UsageEvent> = Vec::with_capacity(BATCH_SIZE);
    let mut deadline = Instant::now() + FLUSH_INTERVAL;
    loop {
        let wait = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(wait) {
            Ok(e) => {
                if batch.is_empty() {
                    deadline = Instant::now() + FLUSH_INTERVAL;
                }
                batch.push(e);
                if batch.len() >= BATCH_SIZE || stop.load(Ordering::SeqCst) {
                    flush(db, &mut batch);
                    deadline = Instant::now() + FLUSH_INTERVAL;
                    if stop.load(Ordering::SeqCst) {
                        drain(db, rx, &mut batch);
                        return;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                flush(db, &mut batch);
                deadline = Instant::now() + FLUSH_INTERVAL;
                if stop.load(Ordering::SeqCst) {
                    drain(db, rx, &mut batch);
                    return;
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                drain(db, rx, &mut batch);
                return;
            }
        }
    }
}

/// Persists whatever is still queued, then the partial batch.
fn drain(db: &Db, rx: &Receiver<UsageEvent>, batch: &mut Vec<UsageEvent>) {
    while let Ok(e) = rx.try_recv() {
        batch.push(e);
        if batch.len() >= BATCH_SIZE {
            flush(db, batch);
        }
    }
    flush(db, batch);
}

fn flush(db: &Db, batch: &mut Vec<UsageEvent>) {
    if batch.is_empty() {
        return;
    }
    if let Err(e) = db.with_conn(|conn| usage::insert_batch(conn, batch)) {
        tracing::error!("failed to persist {} usage records: {e}", batch.len());
    }
    batch.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Protocol;
    use std::sync::mpsc::sync_channel;

    fn event(i: i64) -> UsageEvent {
        UsageEvent {
            requested_at: 1_700_000_000_000 + i,
            client: None,
            route_mode: None,
            provider_name: "p".into(),
            protocol: Protocol::Anthropic,
            model: "m".into(),
            input_tokens: 1,
            output_tokens: 2,
            cached_tokens: 0,
            reasoning_tokens: 0,
        }
    }

    fn count(db: &Db) -> i64 {
        db.with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM usage", [], |r| r.get(0))?))
            .unwrap()
    }

    #[test]
    fn records_are_flushed_in_batches() {
        let db = Db::open_in_memory().unwrap();
        let (sink, handle) = spawn_usage_writer(db.clone());
        for i in 0..600 {
            sink.record(event(i));
        }
        handle.shutdown();
        assert_eq!(count(&db), 600);
    }

    #[test]
    fn full_queue_drops_without_blocking() {
        // No consumer at all: the queue (capacity 1) fills and stays full.
        let (tx, rx) = sync_channel(1);
        let sink = UsageSink { tx };
        for i in 0..10 {
            sink.record(event(i));
        }
        assert_eq!(rx.try_iter().count(), 1);
    }
}
