use std::collections::BTreeSet;
use yi_core::AppPaths;
use yi_core::domain::{LogLevel, Protocol, Provider, Thinking, UsageEvent};
use yi_core::error::ErrorCode;
use yi_core::logging;
use yi_core::services::{
    ModelInput, ProtocolSupport, ProviderInput, Services, UsageQueryInput, dto::CatalogInfo,
};
use yi_core::storage::{providers, usage};

fn provider(short_code: &str) -> ProviderInput {
    ProviderInput {
        id: String::new(),
        provider_type: Protocol::OpenaiChat,
        name: format!("Provider {short_code}"),
        base_url: "https://example.com/v1".into(),
        api_key: "sk-test".into(),
        enabled: true,
        thinking: Thinking::Medium,
        extra: serde_json::json!({}),
        is_default: false,
        protocol_support: ProtocolSupport::default(),
    }
}

fn model(upstream: &str) -> ModelInput {
    ModelInput {
        id: String::new(),
        provider_id: String::new(),
        route_id: String::new(),
        name: upstream.into(),
        standard_model_id: None,
        capabilities: None,
        non_standard: false,
    }
}

#[tokio::test]
async fn save_provider_generates_uuid_and_lists() {
    let home = tempfile::tempdir().unwrap();
    let svc = Services::for_test(home.path());
    let mut input = provider("oa");
    input.id = "client-made".into();
    let saved = svc
        .save_provider(input, vec![model("gpt-5")])
        .await
        .unwrap();
    // A non-empty id that does not exist yet is ignored.
    assert_ne!(saved.provider.id, "client-made");
    assert!(uuid::Uuid::parse_str(&saved.provider.id).is_ok());
    assert!(saved.provider.protocol_support.openai_chat);
    assert_eq!(saved.models.len(), 1);
    assert_eq!(saved.models[0].route_id, "Provider-oa/gpt-5");
    assert_eq!(saved.models[0].name, "gpt-5");
    assert!(uuid::Uuid::parse_str(&saved.models[0].id).is_ok());
    assert_eq!(saved.models[0].provider_id, saved.provider.id);

    // Updating keeps the id and the model ids.
    let mut again = provider("oa");
    again.id = saved.provider.id.clone();
    again.name = "Renamed".into();
    let updated = svc
        .save_provider(again, vec![model("gpt-5"), model("gpt-5-mini")])
        .await
        .unwrap();
    assert_eq!(updated.provider.id, saved.provider.id);
    assert_eq!(updated.models[0].id, saved.models[0].id);

    let listed = svc.list_providers().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0], updated);
    assert_eq!(listed[0].provider.name, "Renamed");

    svc.delete_provider(updated.provider.id).await.unwrap();
    assert!(svc.list_providers().await.unwrap().is_empty());
}

#[tokio::test]
async fn save_provider_duplicate_short_code_is_conflict() {
    let home = tempfile::tempdir().unwrap();
    let svc = Services::for_test(home.path());
    svc.save_provider(provider("dup"), vec![]).await.unwrap();
    let err = svc
        .save_provider(provider("DUP"), vec![])
        .await
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::Conflict);
}

#[tokio::test]
async fn standard_models_catalog_and_uuid() {
    let home = tempfile::tempdir().unwrap();
    let svc = Services::for_test(home.path());
    let info: CatalogInfo = svc.get_model_catalog_info().await.unwrap();
    assert_eq!(info.source, "builtin");
    let listed = svc.list_standard_models().await.unwrap();
    assert_eq!(listed.len(), info.count);

    let mut custom = listed[0].clone();
    custom.id = String::new();
    custom.name = "my-custom-model".into();
    let saved = svc.save_standard_model(custom).await.unwrap();
    assert!(uuid::Uuid::parse_str(&saved.id).is_ok());
    assert_eq!(saved.provider_count, 0);
    svc.delete_standard_model(saved.id.clone()).await.unwrap();
    assert!(
        !svc.list_standard_models()
            .await
            .unwrap()
            .iter()
            .any(|m| m.id == saved.id)
    );
}

#[tokio::test]
async fn save_settings_rejects_bad_host() {
    let home = tempfile::tempdir().unwrap();
    let svc = Services::for_test(home.path());
    let err = svc
        .save_settings("not-an-ip".into(), 12000, LogLevel::Info)
        .await
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::Validation);
    assert_eq!(svc.get_settings().await.unwrap().port, 11435);
}

#[tokio::test]
async fn bad_catalog_override_falls_back_to_builtin() {
    let home = tempfile::tempdir().unwrap();
    let bad = home.path().join("bad-catalog.json");
    std::fs::write(&bad, "{ not json").unwrap();
    let mut paths = AppPaths::under(home.path().to_path_buf());
    paths.catalog_override = Some(bad);
    let svc = Services::start(paths).unwrap();
    let info = svc.get_model_catalog_info().await.unwrap();
    assert_eq!((info.source.as_str(), info.path), ("builtin", None));
    assert_eq!(svc.list_standard_models().await.unwrap().len(), info.count);
}

#[tokio::test]
async fn null_thinking_is_viewed_as_medium() {
    let home = tempfile::tempdir().unwrap();
    let svc = Services::for_test(home.path());
    let stored = Provider {
        id: "p-null".into(),
        provider_type: Protocol::OpenaiChat,
        name: "n".into(),
        base_url: "https://example.com".into(),
        api_key: "k".into(),
        enabled: true,
        thinking: None,
        is_default: false,
        sort_order: 0,
        extra: serde_json::json!({}),
        protocols: vec![Protocol::OpenaiChat],
    };
    svc.db()
        .with_tx(|tx| providers::upsert(tx, &stored, &[]))
        .unwrap();
    let listed = svc.list_providers().await.unwrap();
    assert_eq!(listed[0].provider.thinking, Thinking::Medium);
    let v = serde_json::to_value(&listed[0]).unwrap();
    assert_eq!(v["thinking"], "medium");
}

fn event(requested_at: i64, model: &str, input: u32) -> UsageEvent {
    UsageEvent {
        requested_at,
        client: None,
        route_mode: None,
        provider_name: "p".into(),
        protocol: Protocol::OpenaiChat,
        model: model.into(),
        input_tokens: input,
        output_tokens: 1,
        cached_tokens: 0,
        reasoning_tokens: 0,
    }
}

#[tokio::test]
async fn get_usage_by_date_range() {
    let home = tempfile::tempdir().unwrap();
    let svc = Services::for_test(home.path());
    // Local noon of three consecutive days, independent of the machine's time zone.
    let noon = |day: &str| -> i64 {
        svc.db()
            .with_conn(|c| {
                Ok(c.query_row(
                    "SELECT CAST(strftime('%s', ?1, 'utc') AS INTEGER) * 1000",
                    [format!("{day} 12:00:00")],
                    |r| r.get(0),
                )?)
            })
            .unwrap()
    };
    let events = [
        event(noon("2026-03-10"), "a", 10),
        event(noon("2026-03-11"), "a", 20),
        event(noon("2026-03-11"), "b", 30),
        event(noon("2026-03-12"), "a", 40),
    ];
    svc.db()
        .with_conn(|c| usage::insert_batch(c, &events))
        .unwrap();

    let range = |start: &str, end: &str, model: Option<&str>| UsageQueryInput {
        days: None,
        model: model.map(str::to_owned),
        start_date: Some(start.into()),
        end_date: Some(end.into()),
    };
    let report = svc
        .get_usage(range("2026-03-10", "2026-03-11", None))
        .await
        .unwrap()
        .report;
    assert_eq!(report.total_requests, 3);
    assert_eq!(report.input_tokens, 60);
    assert_eq!(
        report
            .days
            .iter()
            .map(|d| d.day.as_str())
            .collect::<Vec<_>>(),
        ["2026-03-10", "2026-03-11"]
    );
    assert!(uuid::Uuid::parse_str(&report.recent[0].id).is_ok());

    let only_a = svc
        .get_usage(range("2026-03-11", "2026-03-12", Some("a")))
        .await
        .unwrap()
        .report;
    assert_eq!((only_a.total_requests, only_a.input_tokens), (2, 60));

    let single = svc
        .get_usage(range("2026-03-12", "2026-03-12", None))
        .await
        .unwrap()
        .report;
    assert_eq!(single.total_requests, 1);

    let half = UsageQueryInput {
        start_date: Some("2026-03-10".into()),
        ..UsageQueryInput::default()
    };
    assert_eq!(
        svc.get_usage(half).await.unwrap_err().code(),
        ErrorCode::Validation
    );
    assert!(
        svc.get_usage(range("bad", "2026-03-12", None))
            .await
            .is_err()
    );
    assert!(
        svc.get_usage(range("2026-03-12", "2026-03-10", None))
            .await
            .is_err()
    );
    // Default window (last 7 days) is accepted and excludes 2026 test data.
    let recent = svc.get_usage(UsageQueryInput::default()).await.unwrap();
    assert_eq!(recent.report.total_requests, 0);
}

#[test]
fn logs_tail_respects_line_limit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("yi-token.log.2026-01-01");
    let prefix = "中".repeat(180_000);
    let lines: String = (0..1200).map(|i| format!("line {i}: 日志\r\n")).collect();
    std::fs::write(&path, format!("{prefix}\n{lines}")).unwrap();
    let tail = logging::tail(dir.path(), 512 * 1024, 1000).unwrap();
    assert_eq!(tail.len(), 1000);
    assert_eq!(tail[0], "line 200: 日志");
    assert_eq!(tail[999], "line 1199: 日志");
    assert!(tail.iter().all(|l| !l.contains('\u{fffd}')));
    // The byte cap drops the oversized prefix line and any partial first line.
    let capped = logging::tail(dir.path(), 64, 1000).unwrap();
    assert!(capped.len() <= 4 && capped.last().unwrap() == "line 1199: 日志");
    assert!(
        logging::tail(&dir.path().join("none"), 1024, 10)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn get_logs_reads_log_dir() {
    let home = tempfile::tempdir().unwrap();
    let svc = Services::for_test(home.path());
    let dir = home.path().join("logs");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("yi-token.log.2099-01-01"), "a\nb\nc\n").unwrap();
    let logs = svc.get_logs().await.unwrap();
    assert_eq!(logs.lines, ["a", "b", "c"]);
}

fn keys(v: &serde_json::Value) -> BTreeSet<String> {
    v.as_object().unwrap().keys().cloned().collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| (*s).to_owned()).collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// Asserts the exact key set of each DTO as serialized for the frontend
/// (v0.1.8 `types.ts`, plus the intentional additions).
#[tokio::test]
async fn dto_field_names_match_frontend() {
    let home = tempfile::tempdir().unwrap();
    let svc = Services::for_test(home.path());
    let mut input = provider("fe");
    input.thinking = Thinking::High;
    let saved = svc.save_provider(input, vec![model("m1")]).await.unwrap();
    let v = serde_json::to_value(&saved).unwrap();
    assert_eq!(
        keys(&v),
        set(&[
            "id",
            "provider_type",
            "name",
            "base_url",
            "api_key",
            "enabled",
            "thinking",
            "extra",
            "is_default",
            "protocol_support",
            "models",
            "terminal_config_warnings",
        ])
    );
    assert_eq!(v["thinking"], "high");
    assert_eq!(v["terminal_config_warnings"], serde_json::json!([]));
    assert_eq!(
        keys(&v["protocol_support"]),
        set(&["anthropic", "openai_chat", "responses"])
    );
    let m = &v["models"][0];
    assert_eq!(
        keys(m),
        set(&[
            "id",
            "provider_id",
            "route_id",
            "name",
            "standard_model_id",
            "capabilities",
            "non_standard",
        ])
    );
    assert!(m["id"].is_string());

    let st = serde_json::to_value(svc.get_settings().await.unwrap()).unwrap();
    assert_eq!(keys(&st), set(&["host", "port", "log_level"]));

    let info = serde_json::to_value(svc.get_model_catalog_info().await.unwrap()).unwrap();
    assert_eq!(
        keys(&info),
        set(&["source", "path", "version", "count", "loaded_at"])
    );

    svc.db()
        .with_conn(|c| usage::insert_batch(c, &[event(now_ms(), "m1", 1)]))
        .unwrap();
    let u = serde_json::to_value(svc.get_usage(UsageQueryInput::default()).await.unwrap()).unwrap();
    assert_eq!(
        keys(&u),
        set(&[
            "total_requests",
            "input_tokens",
            "output_tokens",
            "cached_tokens",
            "reasoning_tokens",
            "days",
            "daily_models",
            "models",
            "by_client",
            "recent",
        ])
    );
    let rec = &u["recent"][0];
    assert_eq!(
        keys(rec),
        set(&[
            "id",
            "requested_at",
            "client",
            "route_mode",
            "provider_name",
            "protocol",
            "model",
            "input_tokens",
            "output_tokens",
            "cached_tokens",
            "reasoning_tokens",
        ])
    );
    assert!(rec["id"].is_string());

    let logs = serde_json::to_value(svc.get_logs().await.unwrap()).unwrap();
    assert_eq!(keys(&logs), set(&["lines"]));
}

#[tokio::test]
async fn catalog_edit_refreshes_route_capabilities() {
    let home = tempfile::tempdir().unwrap();
    let catalog = home.path().join("catalog.json");
    let catalog_with = |window: u32| {
        format!(
            r#"{{"version":1,"models":[{{"name":"alpha","protocol":"responses","capabilities":{{"context_window":{window}}}}}]}}"#
        )
    };
    std::fs::write(&catalog, catalog_with(1000)).unwrap();
    let mut paths = AppPaths::under(home.path().to_path_buf());
    paths.catalog_override = Some(catalog.clone());
    let svc = Services::start(paths).unwrap();

    let standard = svc
        .list_standard_models()
        .await
        .unwrap()
        .into_iter()
        .find(|m| m.name == "alpha")
        .unwrap();
    let mut provider_input = provider("cat");
    provider_input.provider_type = Protocol::Responses;
    svc.save_provider(
        provider_input,
        vec![ModelInput {
            standard_model_id: Some(standard.id.clone()),
            ..model("up")
        }],
    )
    .await
    .unwrap();
    let window_of = |svc: &Services| {
        svc.routes()
            .load()
            .models
            .values()
            .find(|m| m.upstream_model == "up")
            .unwrap()
            .capabilities
            .context_window
    };
    assert_eq!(window_of(&svc), Some(1000));

    std::fs::write(&catalog, catalog_with(2000)).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&catalog)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(60))
        .unwrap();
    svc.list_standard_models().await.unwrap();
    assert_eq!(window_of(&svc), Some(2000));
}
