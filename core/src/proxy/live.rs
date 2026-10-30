//! In-memory registry of requests in flight and recently completed (live view).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::domain::{Client, LivePhase, LiveRequest, LiveSnapshot};
use crate::protocol::Usage;

/// Completed requests kept for the live view.
pub const RING_CAPACITY: usize = 500;

/// Shared by the pipeline (writer) and the live view (reader). Cheap to clone.
#[derive(Clone, Default)]
pub struct LiveRegistry {
    inner: Arc<Mutex<LiveState>>,
}

#[derive(Default)]
struct LiveState {
    in_flight: Vec<LiveRequest>,
    completed: VecDeque<LiveRequest>,
    /// Last assigned sequence number; monotonic for the life of the process.
    seq: u64,
}

impl LiveRegistry {
    fn lock(&self) -> MutexGuard<'_, LiveState> {
        // A poisoned lock only means a panic happened elsewhere; the data is still usable.
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn begin(
        &self,
        step_id: &str,
        client: Option<Client>,
        model: &str,
        provider: &str,
        started_at: i64,
    ) {
        self.lock().in_flight.push(LiveRequest {
            seq: 0,
            step_id: step_id.to_owned(),
            client,
            model: model.to_owned(),
            provider_name: provider.to_owned(),
            started_at,
            first_token_at: None,
            finished_at: None,
            phase: LivePhase::Waiting,
            status: None,
            input_tokens: 0,
            output_tokens: 0,
            session_id: None,
        });
    }

    pub fn first_token(&self, step_id: &str, at: i64) {
        let mut st = self.lock();
        if let Some(r) = st.in_flight.iter_mut().find(|r| r.step_id == step_id) {
            if r.first_token_at.is_none() {
                r.first_token_at = Some(at);
            }
            if r.phase == LivePhase::Waiting {
                r.phase = LivePhase::Streaming;
            }
        }
    }

    /// Moves a request from in flight to completed and assigns its sequence number.
    pub fn finish(
        &self,
        step_id: &str,
        phase: LivePhase,
        status: Option<u16>,
        usage: Usage,
        at: Option<i64>,
    ) {
        let mut st = self.lock();
        let Some(index) = st.in_flight.iter().position(|r| r.step_id == step_id) else {
            return;
        };
        let mut req = st.in_flight.remove(index);
        st.seq += 1;
        req.seq = st.seq;
        req.phase = phase;
        req.status = status;
        req.input_tokens = usage.input_tokens;
        req.output_tokens = usage.output_tokens;
        req.finished_at = at;
        st.completed.push_back(req);
        while st.completed.len() > RING_CAPACITY {
            st.completed.pop_front();
        }
    }

    /// Seeds the completed ring with requests from before a restart, oldest first.
    pub fn restore(&self, rows: Vec<LiveRequest>) {
        let mut st = self.lock();
        for mut req in rows {
            st.seq += 1;
            req.seq = st.seq;
            st.completed.push_back(req);
        }
        while st.completed.len() > RING_CAPACITY {
            st.completed.pop_front();
        }
    }

    /// Attaches the session identifier once the writer has resolved it.
    pub fn set_session(&self, step_id: &str, session_id: &str) {
        let mut guard = self.lock();
        let st = &mut *guard;
        let found = st
            .in_flight
            .iter_mut()
            .chain(st.completed.iter_mut().rev())
            .find(|r| r.step_id == step_id);
        if let Some(r) = found {
            r.session_id = Some(session_id.to_owned());
        }
    }

    /// In-flight requests, and completed ones with `seq > since_seq`.
    pub fn snapshot(&self, since_seq: u64) -> LiveSnapshot {
        let st = self.lock();
        LiveSnapshot {
            in_flight: st.in_flight.clone(),
            completed: st
                .completed
                .iter()
                .filter(|r| r.seq > since_seq)
                .cloned()
                .collect(),
            seq: st.seq,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finish_ok(reg: &LiveRegistry, id: &str) {
        reg.finish(id, LivePhase::Done, Some(200), Usage::default(), Some(2));
    }

    #[test]
    fn restored_requests_continue_the_sequence() {
        let reg = LiveRegistry::default();
        let row = |id: &str| LiveRequest {
            seq: 0,
            step_id: id.into(),
            client: Some(Client::Codex),
            model: "m".into(),
            provider_name: "p".into(),
            started_at: 1,
            first_token_at: None,
            finished_at: Some(2),
            phase: LivePhase::Done,
            status: Some(200),
            input_tokens: 0,
            output_tokens: 0,
            session_id: None,
        };
        reg.restore(vec![row("old-a"), row("old-b")]);
        reg.begin("new", Some(Client::Codex), "m", "p", 1);
        finish_ok(&reg, "new");
        let snapshot = reg.snapshot(0);
        let seqs: Vec<_> = snapshot
            .completed
            .iter()
            .map(|r| (r.step_id.as_str(), r.seq))
            .collect();
        assert_eq!(seqs, [("old-a", 1), ("old-b", 2), ("new", 3)]);
    }

    #[test]
    fn snapshot_since_returns_only_new_completed() {
        let reg = LiveRegistry::default();
        for id in ["a", "b", "c"] {
            reg.begin(id, Some(Client::Codex), "m", "p", 1);
            finish_ok(&reg, id);
        }
        let ids = |since| -> Vec<String> {
            reg.snapshot(since)
                .completed
                .into_iter()
                .map(|r| r.step_id)
                .collect()
        };
        assert_eq!(ids(0), ["a", "b", "c"]);
        assert_eq!(ids(1), ["b", "c"]);
        assert!(ids(3).is_empty());
        assert_eq!(reg.snapshot(0).seq, 3);
    }

    #[test]
    fn ring_keeps_last_500() {
        let reg = LiveRegistry::default();
        for i in 0..600 {
            let id = format!("s{i}");
            reg.begin(&id, None, "m", "p", i);
            finish_ok(&reg, &id);
        }
        let snap = reg.snapshot(0);
        assert_eq!(snap.completed.len(), 500);
        assert_eq!(snap.completed.first().unwrap().seq, 101);
        assert_eq!(snap.completed.last().unwrap().seq, 600);
        assert_eq!(snap.seq, 600);
    }

    #[test]
    fn in_flight_moves_to_completed_on_finish() {
        let reg = LiveRegistry::default();
        reg.begin("x", Some(Client::Codex), "m", "p", 1000);
        let snap = reg.snapshot(0);
        assert_eq!(snap.in_flight.len(), 1);
        assert!(snap.completed.is_empty());
        assert_eq!(snap.in_flight[0].phase, LivePhase::Waiting);

        reg.first_token("x", 1200);
        assert_eq!(reg.snapshot(0).in_flight[0].first_token_at, Some(1200));

        let usage = Usage {
            input_tokens: 3,
            output_tokens: 2,
            ..Usage::default()
        };
        reg.finish("x", LivePhase::Done, Some(200), usage, Some(1500));
        let snap = reg.snapshot(0);
        assert!(snap.in_flight.is_empty());
        assert_eq!(snap.completed.len(), 1);
        let done = &snap.completed[0];
        assert_eq!(done.seq, 1);
        assert_eq!(done.phase, LivePhase::Done);
        assert_eq!(done.status, Some(200));
        assert_eq!((done.input_tokens, done.output_tokens), (3, 2));
        assert_eq!(done.finished_at, Some(1500));

        reg.set_session("x", "sess-1");
        assert_eq!(
            reg.snapshot(0).completed[0].session_id.as_deref(),
            Some("sess-1")
        );
    }
}
