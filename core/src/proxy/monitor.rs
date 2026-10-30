//! Proxy traffic monitor: five-second buckets kept for one hour.

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};

use crate::domain::Client;

/// Milliseconds since the Unix epoch, as a `u64` for the monitor's buckets.
pub fn now() -> u64 {
    u64::try_from(crate::clock::now_ms()).unwrap_or(0)
}

pub const INTERVAL_MS: u64 = 5_000;
pub const CAPACITY: usize = 721;
const MAX_MINUTES: u32 = 60;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counters {
    pub connections: u64,
    pub accepted_connections: u64,
    pub peak_connections: u64,
    pub active_requests: u64,
    pub peak_requests: u64,
    pub requests: u64,
    pub completed: u64,
    pub errors: u64,
    pub received_bytes: u64,
    pub sent_bytes: u64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sample {
    pub timestamp: u64,
    pub connections: u64,
    pub active_requests: u64,
    pub requests: u64,
    pub errors: u64,
    pub received_bytes: u64,
    pub sent_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteStats {
    pub client: String,
    pub protocol: String,
    pub requests: u64,
    pub active_requests: u64,
    pub completed: u64,
    pub errors: u64,
    pub received_bytes: u64,
    pub sent_bytes: u64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub since: u64,
    pub sampled_at: u64,
    pub totals: Counters,
    pub rate_seconds: f64,
    pub received_per_second: f64,
    pub sent_per_second: f64,
    pub requests_per_second: f64,
    pub samples: Vec<Sample>,
    pub routes: Vec<RouteStats>,
}

/// The shape returned by the monitor command.
pub type MonitorView = Snapshot;

/// What survives a restart: lifetime totals, route stats and the retained buckets.
/// Gauges (open connections, requests in flight) are not kept; nothing is in flight
/// when the app starts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorState {
    pub since: u64,
    pub totals: Counters,
    pub samples: Vec<Sample>,
    pub routes: Vec<RouteStats>,
}

type Clock = dyn Fn() -> u64 + Send + Sync;

struct Inner {
    since: u64,
    totals: Counters,
    samples: VecDeque<Sample>,
    routes: BTreeMap<String, RouteStats>,
}

#[derive(Clone)]
pub struct Monitor {
    inner: Arc<Mutex<Inner>>,
    clock: Arc<Clock>,
}

impl Default for Monitor {
    fn default() -> Self {
        Self::with_clock(now)
    }
}

impl Monitor {
    /// Monitor with an injectable clock, for deterministic bucket tests.
    pub fn with_clock(clock: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        let clock: Arc<Clock> = Arc::new(clock);
        let since = clock();
        Self {
            inner: Arc::new(Mutex::new(Inner {
                since,
                totals: Counters::default(),
                samples: VecDeque::new(),
                routes: BTreeMap::new(),
            })),
            clock,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        crate::lock::lock_unpoisoned(&self.inner)
    }

    fn update(&self, action: impl FnOnce(&mut Inner)) {
        let time = (self.clock)();
        let mut inner = self.lock();
        inner.advance(time);
        action(&mut inner);
        let connections = inner.totals.connections;
        let active = inner.totals.active_requests;
        if let Some(last) = inner.samples.back_mut() {
            last.connections = connections;
            last.active_requests = active;
        }
    }

    pub fn totals(&self) -> Counters {
        self.lock().totals.clone()
    }

    /// Copies the parts that survive a restart.
    pub fn export(&self) -> MonitorState {
        let inner = self.lock();
        MonitorState {
            since: inner.since,
            totals: inner.totals.clone(),
            samples: inner.samples.iter().cloned().collect(),
            routes: inner.routes.values().cloned().collect(),
        }
    }

    /// Replaces the retained history with `state`. Gauges are reset, and buckets older
    /// than the retention window are dropped.
    pub fn restore(&self, state: MonitorState) {
        let time = (self.clock)() / INTERVAL_MS * INTERVAL_MS;
        let floor = time.saturating_sub((CAPACITY as u64 - 1) * INTERVAL_MS);
        let mut inner = self.lock();
        inner.since = state.since;
        inner.totals = Counters {
            connections: 0,
            active_requests: 0,
            ..state.totals
        };
        inner.samples = state
            .samples
            .into_iter()
            .filter(|s| s.timestamp >= floor)
            .map(|s| Sample {
                connections: 0,
                active_requests: 0,
                ..s
            })
            .collect();
        inner.routes = state
            .routes
            .into_iter()
            .map(|route| {
                let key = format!("{}:{}", route.client, route.protocol);
                (
                    key,
                    RouteStats {
                        active_requests: 0,
                        ..route
                    },
                )
            })
            .collect();
    }

    /// Number of retained five-second buckets (bounded by [`CAPACITY`]).
    pub fn bucket_count(&self) -> usize {
        self.lock().samples.len()
    }

    pub fn connection(&self, opened: bool) {
        self.update(|i| {
            if opened {
                i.totals.connections += 1;
                i.totals.accepted_connections += 1;
                i.totals.peak_connections = i.totals.peak_connections.max(i.totals.connections);
            } else {
                i.totals.connections = i.totals.connections.saturating_sub(1);
            }
        });
    }

    pub fn traffic(&self, received: usize, sent: usize) {
        if received + sent == 0 {
            return;
        }
        self.update(|i| {
            i.totals.received_bytes += received as u64;
            i.totals.sent_bytes += sent as u64;
            if let Some(sample) = i.samples.back_mut() {
                sample.received_bytes += received as u64;
                sample.sent_bytes += sent as u64;
            }
        });
    }

    /// Starts one request and returns the key used by `payload` and `finish`.
    pub fn begin(&self, path: &str) -> String {
        let (client, protocol) = route_parts(path);
        let key = format!("{client}:{protocol}");
        self.update(|i| {
            i.totals.requests += 1;
            i.totals.active_requests += 1;
            i.totals.peak_requests = i.totals.peak_requests.max(i.totals.active_requests);
            if let Some(sample) = i.samples.back_mut() {
                sample.requests += 1;
            }
            let route = i.routes.entry(key.clone()).or_insert_with(|| RouteStats {
                client: client.to_owned(),
                protocol: protocol.to_owned(),
                ..RouteStats::default()
            });
            route.requests += 1;
            route.active_requests += 1;
        });
        key
    }

    pub fn payload(&self, key: &str, received: usize, sent: usize) {
        self.update(|i| {
            if let Some(route) = i.routes.get_mut(key) {
                route.received_bytes += received as u64;
                route.sent_bytes += sent as u64;
            }
        });
    }

    pub fn finish(&self, key: &str, error: bool, duration: u64) {
        self.update(|i| {
            i.totals.active_requests = i.totals.active_requests.saturating_sub(1);
            i.totals.completed += 1;
            i.totals.errors += u64::from(error);
            i.totals.duration_ms += duration;
            if let Some(sample) = i.samples.back_mut() {
                sample.errors += u64::from(error);
            }
            if let Some(route) = i.routes.get_mut(key) {
                route.active_requests = route.active_requests.saturating_sub(1);
                route.completed += 1;
                route.errors += u64::from(error);
                route.duration_ms += duration;
            }
        });
    }

    pub fn snapshot(&self, minutes: Option<u32>) -> Snapshot {
        let time = (self.clock)();
        let mut inner = self.lock();
        inner.advance(time);
        // Rates use complete five-second buckets, excluding the current partial bucket.
        let boundary = time / INTERVAL_MS * INTERVAL_MS;
        let start = boundary.saturating_sub(10_000).max(inner.since);
        let seconds = boundary.saturating_sub(start) as f64 / 1000.0;
        let rate: Vec<_> = inner
            .samples
            .iter()
            .filter(|s| s.timestamp >= start / INTERVAL_MS * INTERVAL_MS && s.timestamp < boundary)
            .collect();
        let divisor = seconds.max(1.0);
        let minutes = minutes.unwrap_or(15).clamp(1, MAX_MINUTES);
        Snapshot {
            since: inner.since,
            sampled_at: time,
            totals: inner.totals.clone(),
            rate_seconds: seconds,
            received_per_second: rate.iter().map(|s| s.received_bytes).sum::<u64>() as f64
                / divisor,
            sent_per_second: rate.iter().map(|s| s.sent_bytes).sum::<u64>() as f64 / divisor,
            requests_per_second: rate.iter().map(|s| s.requests).sum::<u64>() as f64 / divisor,
            samples: inner
                .samples
                .iter()
                .filter(|s| s.timestamp >= time.saturating_sub(minutes as u64 * 60_000))
                .cloned()
                .collect(),
            routes: inner.routes.values().cloned().collect(),
        }
    }
}

impl Inner {
    fn advance(&mut self, time: u64) {
        let time = time / INTERVAL_MS * INTERVAL_MS;
        if self.samples.is_empty() {
            self.samples.push_back(Sample {
                timestamp: time,
                ..Sample::default()
            });
            return;
        }
        let previous = self.samples.back().map_or(time, |s| s.timestamp);
        if time <= previous {
            return;
        }
        let start = previous
            .saturating_add(INTERVAL_MS)
            .max(time.saturating_sub((CAPACITY as u64 - 1) * INTERVAL_MS));
        for timestamp in (start..=time).step_by(INTERVAL_MS as usize) {
            if self.samples.len() == CAPACITY {
                self.samples.pop_front();
            }
            self.samples.push_back(Sample {
                timestamp,
                connections: self.totals.connections,
                active_requests: self.totals.active_requests,
                ..Sample::default()
            });
        }
    }
}

fn route_parts(path: &str) -> (String, String) {
    let parts: Vec<_> = path.split('/').filter(|p| !p.is_empty()).collect();
    let client = if parts.first() == Some(&"clients") {
        parts
            .get(1)
            .and_then(|name| Client::from_str(name).ok())
            .map_or_else(|| "unknown".to_owned(), |c| c.as_str().to_owned())
    } else {
        "api".to_owned()
    };
    let protocol = match parts.last().copied().unwrap_or_default() {
        "responses" => "responses",
        "completions" => "openai_chat",
        "messages" => "anthropic",
        "models" => "models",
        _ => "other",
    };
    (client, protocol.to_owned())
}

/// Listener wrapper that keeps the live connection gauge in the monitor.
pub struct TrackedListener {
    pub listener: TcpListener,
    pub monitor: Monitor,
}

pub struct TrackedStream {
    stream: TcpStream,
    monitor: Monitor,
}

impl axum::serve::Listener for TrackedListener {
    type Io = TrackedStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.listener.accept().await {
                Ok((stream, addr)) => {
                    self.monitor.connection(true);
                    return (
                        TrackedStream {
                            stream,
                            monitor: self.monitor.clone(),
                        },
                        addr,
                    );
                }
                Err(error) => {
                    tracing::warn!(%error, "TCP accept failed");
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

impl Drop for TrackedStream {
    fn drop(&mut self) {
        self.monitor.connection(false);
    }
}

impl AsyncRead for TrackedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.stream).poll_read(cx, buf);
        self.monitor.traffic(buf.filled().len() - before, 0);
        result
    }
}

impl AsyncWrite for TrackedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.stream).poll_write(cx, buf);
        if let Poll::Ready(Ok(count)) = &result {
            self.monitor.traffic(0, *count);
        }
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
