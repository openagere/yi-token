mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use common::TestEnv;
use yi_core::AppPaths;
use yi_core::proxy::ProxyPhase;
use yi_core::proxy::monitor::{CAPACITY, INTERVAL_MS, Monitor};
use yi_core::services::Services;

#[test]
fn monitor_buckets_rotate_after_721() {
    let clock = Arc::new(AtomicU64::new(0));
    let source = Arc::clone(&clock);
    let monitor = Monitor::with_clock(move || source.load(Ordering::SeqCst));

    monitor.snapshot(Some(60));
    clock.store(720 * INTERVAL_MS, Ordering::SeqCst);
    monitor.snapshot(Some(60));
    assert_eq!(monitor.bucket_count(), CAPACITY);

    clock.store(721 * INTERVAL_MS, Ordering::SeqCst);
    monitor.snapshot(Some(60));
    assert_eq!(monitor.bucket_count(), CAPACITY);
    let snapshot = monitor.snapshot(Some(60));
    assert_eq!(
        snapshot.samples.last().unwrap().timestamp,
        721 * INTERVAL_MS
    );
    assert_eq!(
        snapshot.samples.first().unwrap().timestamp,
        INTERVAL_MS,
        "the oldest bucket was rotated out"
    );
    assert_eq!(snapshot.samples.len(), CAPACITY);
}

#[test]
fn monitor_route_stats_by_client_protocol() {
    let monitor = Monitor::default();
    let codex = monitor.begin("/clients/codex/v1/responses");
    monitor.traffic(100, 250);
    monitor.payload(&codex, 100, 250);
    monitor.finish(&codex, false, 42);
    let chat = monitor.begin("/v1/chat/completions");
    monitor.finish(&chat, true, 8);
    let messages = monitor.begin("/clients/claude-code/v1/messages");
    monitor.finish(&messages, false, 3);

    let snapshot = monitor.snapshot(Some(5));
    assert_eq!(snapshot.totals.requests, 3);
    assert_eq!(snapshot.totals.completed, 3);
    assert_eq!(snapshot.totals.errors, 1);
    assert_eq!(snapshot.totals.active_requests, 0);
    assert_eq!(snapshot.totals.received_bytes, 100);
    assert_eq!(snapshot.totals.sent_bytes, 250);

    let find = |client: &str, protocol: &str| {
        snapshot
            .routes
            .iter()
            .find(|r| r.client == client && r.protocol == protocol)
            .unwrap_or_else(|| panic!("missing route {client}:{protocol}"))
    };
    let codex = find("codex", "responses");
    assert_eq!(
        (
            codex.requests,
            codex.completed,
            codex.errors,
            codex.received_bytes,
            codex.sent_bytes,
            codex.duration_ms
        ),
        (1, 1, 0, 100, 250, 42)
    );
    let chat = find("api", "openai_chat");
    assert_eq!((chat.requests, chat.errors, chat.duration_ms), (1, 1, 8));
    let messages = find("claude-code", "anthropic");
    assert_eq!((messages.requests, messages.errors), (1, 0));
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

async fn wait_healthy(port: u16) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let ok = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{port}/health"))
            .send()
            .await
            .is_ok();
        if ok || std::time::Instant::now() >= deadline {
            assert!(ok, "proxy is not listening on {port}");
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn save_settings_port_change_restarts_proxy() {
    let home = tempfile::tempdir().unwrap();
    let services = Services::for_test(home.path());
    let first = free_port();
    let second = free_port();
    services
        .save_settings("127.0.0.1".into(), first, yi_core::domain::LogLevel::Info)
        .await
        .unwrap();
    services.control_proxy("start").await.unwrap();
    assert_eq!(
        services.get_proxy_runtime().await.unwrap().address.unwrap(),
        format!("127.0.0.1:{first}")
    );
    wait_healthy(first).await;

    services
        .save_settings("127.0.0.1".into(), second, yi_core::domain::LogLevel::Info)
        .await
        .unwrap();
    assert_eq!(
        services.get_proxy_runtime().await.unwrap().address.unwrap(),
        format!("127.0.0.1:{second}")
    );
    wait_healthy(second).await;
    services.shutdown();
}

#[tokio::test]
async fn app_starts_when_port_busy() {
    let home = tempfile::tempdir().unwrap();
    let port = free_port();
    {
        let services = Services::for_test(home.path());
        services
            .save_settings("127.0.0.1".into(), port, yi_core::domain::LogLevel::Info)
            .await
            .unwrap();
        services.shutdown();
    }
    let blocker = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    let services = Services::start(AppPaths::under(home.path().to_path_buf())).unwrap();
    let runtime = services.get_proxy_runtime().await.unwrap();
    assert_eq!(runtime.phase, ProxyPhase::Error);
    assert!(runtime.address.is_none());
    assert!(
        runtime
            .last_error
            .as_deref()
            .is_some_and(|e| e.contains(&port.to_string())),
        "{:?}",
        runtime.last_error
    );

    drop(blocker);
    services.control_proxy("start").await.unwrap();
    wait_healthy(port).await;
    services.shutdown();
}

#[tokio::test]
async fn request_log_updates_monitor() {
    let env = TestEnv::start().await;
    let resp = env
        .post(
            "/v1/responses",
            &serde_json::json!({"model": "does-not-exist", "input": "hi"}),
        )
        .await;
    assert_eq!(resp.status(), 404);
    assert_eq!(env.wait_idle(Duration::from_secs(2)).await, 0);
    let snapshot = env.state.monitor.snapshot(Some(5));
    assert_eq!(snapshot.totals.requests, 1);
    assert_eq!(snapshot.totals.errors, 1);
    let route = snapshot
        .routes
        .iter()
        .find(|r| r.client == "api" && r.protocol == "responses")
        .expect("global responses route");
    assert_eq!(
        (route.requests, route.errors, route.active_requests),
        (1, 1, 0)
    );
}
