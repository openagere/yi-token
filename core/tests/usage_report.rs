//! Usage report: client filter, grouping, totals and timing averages.

use std::sync::Arc;

use yi_core::domain::{Client, Protocol, UsageEvent, UsageGroupBy};
use yi_core::services::{Services, UsageQueryInput, UsageReportView};
use yi_core::storage::usage;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// Window covering "now" (today plus the day before), independent of time zone.
fn window() -> UsageQueryInput {
    UsageQueryInput {
        days: Some(2),
        ..UsageQueryInput::default()
    }
}

fn ev(client: Option<Client>, provider: &str, model: &str, input: u32, cached: u32) -> UsageEvent {
    UsageEvent {
        requested_at: now_ms(),
        client,
        route_mode: None,
        provider_name: provider.into(),
        protocol: Protocol::OpenaiChat,
        model: model.into(),
        input_tokens: input,
        output_tokens: 1,
        cached_tokens: cached,
        reasoning_tokens: 0,
        step_id: None,
        session_id: None,
        ttft_ms: None,
        duration_ms: None,
        status: None,
    }
}

fn timed(mut e: UsageEvent, ttft: u32, duration: u32) -> UsageEvent {
    e.ttft_ms = Some(ttft);
    e.duration_ms = Some(duration);
    e
}

fn seed(svc: &Services, events: &[UsageEvent]) {
    svc.db()
        .with_conn(|c| usage::insert_batch(c, events))
        .unwrap();
}

async fn report(svc: &Arc<Services>, input: UsageQueryInput) -> UsageReportView {
    svc.get_usage(input).await.unwrap()
}

#[tokio::test]
async fn client_filter_limits_rows() {
    let home = tempfile::tempdir().unwrap();
    let svc = Services::for_test(home.path());
    seed(
        &svc,
        &[
            ev(Some(Client::Codex), "p", "m", 10, 0),
            ev(Some(Client::Codex), "p", "m", 20, 0),
            ev(Some(Client::ClaudeCode), "p", "m", 40, 0),
            ev(None, "p", "m", 80, 0),
        ],
    );

    let all = report(&svc, window()).await;
    assert_eq!(all.report.total_requests, 4);
    assert_eq!(all.totals.requests, 4);

    let codex = report(
        &svc,
        UsageQueryInput {
            client: Some(Client::Codex),
            ..window()
        },
    )
    .await;
    assert_eq!(codex.report.total_requests, 2);
    assert_eq!(codex.report.input_tokens, 30);
    assert_eq!(codex.report.recent.len(), 2);
    assert!(
        codex
            .report
            .recent
            .iter()
            .all(|r| r.client == Some(Client::Codex))
    );
    assert_eq!(codex.totals.requests, 2);
    assert_eq!(codex.totals.input_tokens, 30);
    assert_eq!(codex.groups.iter().map(|g| g.requests).sum::<u64>(), 2);

    let claude = report(
        &svc,
        UsageQueryInput {
            client: Some(Client::ClaudeCode),
            ..window()
        },
    )
    .await;
    assert_eq!(claude.totals.requests, 1);
    assert_eq!(claude.totals.input_tokens, 40);
}

#[tokio::test]
async fn group_by_provider_aggregates() {
    let home = tempfile::tempdir().unwrap();
    let svc = Services::for_test(home.path());
    seed(
        &svc,
        &[
            ev(None, "alpha", "m1", 10, 5),
            ev(None, "alpha", "m2", 20, 5),
            ev(None, "beta", "m1", 30, 0),
        ],
    );

    let by_provider = report(
        &svc,
        UsageQueryInput {
            group_by: Some(UsageGroupBy::Provider),
            ..window()
        },
    )
    .await;
    assert_eq!(by_provider.groups.len(), 2);
    let alpha = by_provider
        .groups
        .iter()
        .find(|g| g.key == "alpha")
        .unwrap();
    assert_eq!(alpha.label, "alpha");
    assert_eq!(alpha.requests, 2);
    assert_eq!(alpha.input_tokens, 30);
    assert_eq!(alpha.output_tokens, 2);
    assert_eq!(alpha.cached_tokens, 10);
    assert_eq!(alpha.cache_hit_rate, Some(10.0 / 30.0));
    let beta = by_provider.groups.iter().find(|g| g.key == "beta").unwrap();
    assert_eq!(beta.requests, 1);
    assert_eq!(beta.input_tokens, 30);

    // Default grouping is by model.
    let by_model = report(&svc, window()).await;
    let mut keys: Vec<&str> = by_model.groups.iter().map(|g| g.key.as_str()).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["m1", "m2"]);
    let m1 = by_model.groups.iter().find(|g| g.key == "m1").unwrap();
    assert_eq!(m1.requests, 2);
    assert_eq!(m1.input_tokens, 40);
}

#[tokio::test]
async fn cache_hit_rate_none_when_no_input() {
    let home = tempfile::tempdir().unwrap();
    let svc = Services::for_test(home.path());
    seed(
        &svc,
        &[
            ev(Some(Client::Codex), "p", "empty", 0, 0),
            ev(Some(Client::ClaudeCode), "p", "full", 10, 5),
        ],
    );

    let empty = report(
        &svc,
        UsageQueryInput {
            client: Some(Client::Codex),
            ..window()
        },
    )
    .await;
    assert_eq!(empty.totals.input_tokens, 0);
    assert_eq!(empty.totals.cache_hit_rate, None);
    assert_eq!(empty.groups[0].cache_hit_rate, None);

    let all = report(&svc, window()).await;
    assert_eq!(all.totals.cache_hit_rate, Some(0.5));
    let full = all.groups.iter().find(|g| g.key == "full").unwrap();
    assert_eq!(full.cache_hit_rate, Some(0.5));
    let none = all.groups.iter().find(|g| g.key == "empty").unwrap();
    assert_eq!(none.cache_hit_rate, None);
}

#[tokio::test]
async fn averages_ignore_rows_without_timing() {
    let home = tempfile::tempdir().unwrap();
    let svc = Services::for_test(home.path());
    seed(
        &svc,
        &[
            timed(ev(None, "p", "m", 1, 0), 100, 1000),
            timed(ev(None, "p", "m", 1, 0), 300, 3000),
            // Legacy rows from before the timing migration: NULL, not zero.
            ev(None, "p", "m", 1, 0),
            ev(None, "p", "old", 1, 0),
        ],
    );

    let view = report(&svc, window()).await;
    assert_eq!(view.totals.requests, 4);
    assert_eq!(view.totals.timed_requests, 2);
    assert_eq!(view.totals.avg_ttft_ms, Some(200.0));
    assert_eq!(view.totals.avg_duration_ms, Some(2000.0));

    let m = view.groups.iter().find(|g| g.key == "m").unwrap();
    assert_eq!(m.requests, 3);
    assert_eq!(m.avg_duration_ms, Some(2000.0));

    let old = view.groups.iter().find(|g| g.key == "old").unwrap();
    assert_eq!(old.avg_duration_ms, None);
}
