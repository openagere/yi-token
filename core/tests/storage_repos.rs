use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};
use yi_core::domain::{
    Capabilities, Client, Company, DirectProfile, LogLevel, Model, ModelSource, ProfileModel,
    Protocol, Provider, ProxyProfile, RouteMode, Settings, StandardModel, TrajectorySettings,
    UsageEvent,
};
use yi_core::storage::usage::UsageQuery;
use yi_core::storage::{Db, engines, migrations, models, providers, settings, standard_models};
use yi_core::storage::{terminal, usage};
use yi_core::{CoreError, ErrorCode};

fn provider(id: &str, _code: &str) -> Provider {
    Provider {
        id: id.into(),
        provider_type: Protocol::OpenaiChat,
        name: format!("name-{id}"),
        base_url: "https://api.example.com/v1".into(),
        api_key: "sk".into(),
        enabled: true,
        thinking: None,
        is_default: false,
        sort_order: 0,
        extra: serde_json::json!({"k": 1}),
        protocols: vec![Protocol::OpenaiChat],
    }
}

fn model(id: &str, upstream: &str) -> Model {
    Model {
        id: id.into(),
        provider_id: String::new(),
        standard_model_id: None,
        protocol: None,
        upstream_model: upstream.into(),
        exposed_name: String::new(),
        capabilities: Capabilities::default(),
        non_standard: false,
    }
}

fn standard(id: &str, name: &str) -> StandardModel {
    StandardModel {
        id: id.into(),
        name: name.into(),
        company: Company::Openai,
        brand: "brand".into(),
        capabilities: Capabilities::default(),
    }
}

fn save(db: &Db, p: &Provider, ms: &[Model]) -> Result<(), CoreError> {
    db.with_tx(|tx| providers::upsert(tx, p, ms))
}

fn user_version(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

fn count(db: &Db, table: &str) -> i64 {
    db.with_conn(|c| Ok(c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?))
        .unwrap()
}

#[test]
fn migration_sets_user_version_4() {
    let db = Db::open_in_memory().unwrap();
    db.with_conn(|c| {
        assert_eq!(user_version(c), 6);
        let fk: i64 = c.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
        assert_eq!(fk, 1);
        for index in ["idx_usage_requested_at", "idx_usage_client"] {
            let n: i64 = c.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
                [index],
                |r| r.get(0),
            )?;
            assert_eq!(n, 1, "{index}");
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn file_database_uses_wal_and_foreign_keys() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("nested").join("t.db")).unwrap();
    fn check(c: &Connection) {
        let mode: String = c
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        let fk: i64 = c
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1);
        let busy: i64 = c
            .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
            .unwrap();
        assert_eq!(busy, 5000);
    }
    // Hold three pooled connections at once so each is a distinct connection.
    db.with_conn(|a| {
        db.with_conn(|b| {
            db.with_conn(|c| {
                for conn in [a, b, c] {
                    check(conn);
                }
                Ok(())
            })
        })
    })
    .unwrap();
    // Reopening an existing database keeps the version and data.
    drop(db);
    let db = Db::open(&dir.path().join("nested").join("t.db")).unwrap();
    db.with_conn(|c| {
        assert_eq!(user_version(c), 6);
        Ok(())
    })
    .unwrap();
}

/// Test-only helper: the real v1 followed by a v2 that creates a table and then fails.
fn migrations_with_failing_v2() -> Migrations<'static> {
    Migrations::new(vec![
        migrations::v1(),
        M::up("CREATE TABLE half_done (x INTEGER); THIS IS NOT SQL;"),
    ])
}

#[test]
fn migration_is_atomic() {
    let mut conn = Connection::open_in_memory().unwrap();
    Migrations::new(vec![migrations::v1()]).to_latest(&mut conn).unwrap();
    assert_eq!(user_version(&conn), 1);
    assert!(migrations_with_failing_v2().to_latest(&mut conn).is_err());
    assert_eq!(user_version(&conn), 1);
    let leaked: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'half_done'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(leaked, 0, "failed migration must roll back its statements");
}

#[test]
fn provider_roundtrip_with_protocols_and_models() {
    let db = Db::open_in_memory().unwrap();
    let mut p = provider("p1", "oa");
    p.thinking = Some(yi_core::domain::Thinking::High);
    p.protocols = vec![Protocol::OpenaiChat, Protocol::Anthropic];
    save(&db, &p, &[model("m1", "gpt-5"), model("m2", "gpt-4")]).unwrap();
    db.with_conn(|c| {
        assert_eq!(providers::get(c, "p1")?.unwrap(), p);
        assert_eq!(providers::list(c)?, vec![p.clone()]);
        assert!(providers::get(c, "nope")?.is_none());
        let ms = models::list_by_provider(c, "p1")?;
        let names: Vec<_> = ms.iter().map(|m| m.exposed_name.as_str()).collect();
        assert_eq!(names, ["name-p1/gpt-5", "name-p1/gpt-4"]);
        assert_eq!(ms[0].provider_id, "p1");
        assert_eq!(models::get(c, "m2")?.unwrap().upstream_model, "gpt-4");
        assert_eq!(models::list(c)?.len(), 2);
        Ok(())
    })
    .unwrap();
}

#[test]
fn set_default_keeps_a_single_default() {
    let db = Db::open_in_memory().unwrap();
    save(&db, &provider("a", "a"), &[]).unwrap();
    save(&db, &provider("b", "b"), &[]).unwrap();
    db.with_conn(|c| {
        providers::set_default(c, "a")?;
        providers::set_default(c, "b")?;
        let defaults: Vec<_> = providers::list(c)?
            .into_iter()
            .filter(|p| p.is_default)
            .map(|p| p.id)
            .collect();
        assert_eq!(defaults, ["b"]);
        assert_eq!(
            providers::set_default(c, "zzz").unwrap_err().code(),
            ErrorCode::NotFound
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn provider_names_are_unique_as_route_prefixes() {
    let db = Db::open_in_memory().unwrap();
    let mut first = provider("p1", "");
    first.name = "Deep Seek".into();
    save(&db, &first, &[]).unwrap();
    let mut second = provider("p2", "");
    second.name = "deep-seek".into();
    let err = save(&db, &second, &[]).unwrap_err();
    assert!(matches!(err, CoreError::Conflict(_)), "{err:?}");
    // Re-saving the owner is fine.
    save(&db, &first, &[]).unwrap();
    assert_eq!(count(&db, "providers"), 1);
}

#[test]
fn exposed_name_unique() {
    let db = Db::open_in_memory().unwrap();
    let err = save(
        &db,
        &provider("p1", "oa"),
        &[model("m1", "gpt-5"), model("m2", " gpt-5 ")],
    )
    .unwrap_err();
    assert!(matches!(err, CoreError::Conflict(_)), "{err:?}");
    assert_eq!(count(&db, "providers"), 0, "failed upsert rolls back");
    save(&db, &provider("p1", "oa"), &[model("m1", "gpt-5")]).unwrap();
    // The schema enforces uniqueness independently of the repo checks.
    let raw = db.with_conn(|c| {
        Ok(c.execute(
            "INSERT INTO models (id, provider_id, upstream_model, exposed_name) \
             VALUES ('mx', 'p1', 'other', 'name-p1/gpt-5')",
            [],
        ))
    });
    assert!(raw.unwrap().is_err());
}

#[test]
fn upsert_keeps_model_id_for_same_upstream() {
    let db = Db::open_in_memory().unwrap();
    save(
        &db,
        &provider("p1", "oa"),
        &[model("m1", "gpt-5"), model("m2", "gpt-4")],
    )
    .unwrap();
    db.with_conn(|c| engines::set(c, Client::Pi, Some("m1")))
        .unwrap();
    // gpt-5 stays (with a fresh candidate id that must be ignored), gpt-4 goes, o3 is new;
    // the short code changes, which rewrites exposed names.
    let mut renamed = provider("p1", "work");
    renamed.name = "renamed".into();
    save(
        &db,
        &renamed,
        &[model("ignored", "gpt-5"), model("m3", "o3")],
    )
    .unwrap();
    db.with_conn(|c| {
        let ms = models::list_by_provider(c, "p1")?;
        let ids: Vec<_> = ms
            .iter()
            .map(|m| (m.id.as_str(), m.exposed_name.as_str()))
            .collect();
        assert_eq!(ids, [("m1", "renamed/gpt-5"), ("m3", "renamed/o3")]);
        assert!(models::get(c, "m2")?.is_none());
        assert_eq!(engines::get(c, Client::Pi)?, Some("m1".into()));
        assert_eq!(providers::get(c, "p1")?.unwrap().name, "renamed");
        Ok(())
    })
    .unwrap();
}

#[test]
fn delete_provider_cascades_models_and_direct_profile_and_nulls_engine() {
    let db = Db::open_in_memory().unwrap();
    let mut p = provider("p1", "oa");
    p.protocols = vec![Protocol::OpenaiChat, Protocol::Responses];
    save(&db, &p, &[model("m1", "gpt-5")]).unwrap();
    save(&db, &provider("p2", "keep"), &[model("m9", "x")]).unwrap();
    db.with_conn(|c| {
        terminal::save_direct(
            c,
            &DirectProfile {
                client: Client::Codex,
                provider_id: "p1".into(),
                model_source: ModelSource::Provider,
                model: Some("gpt-5".into()),
            },
        )?;
        engines::set(c, Client::Pi, Some("m1"))?;
        engines::set(c, Client::Opencode, Some("m9"))?;
        providers::delete(c, "p1")
    })
    .unwrap();
    assert_eq!(count(&db, "provider_protocols"), 1);
    assert_eq!(count(&db, "models"), 1);
    assert_eq!(count(&db, "terminal_direct_profiles"), 0);
    db.with_conn(|c| {
        assert_eq!(engines::get(c, Client::Pi)?, None);
        assert_eq!(engines::get(c, Client::Opencode)?, Some("m9".into()));
        // The row survives with a NULL model ("not selected").
        assert!(engines::list(c)?.contains(&(Client::Pi, None)));
        Ok(())
    })
    .unwrap();
}

#[test]
fn delete_standard_model_in_use_is_conflict() {
    let db = Db::open_in_memory().unwrap();
    db.with_conn(|c| standard_models::upsert(c, &standard("s1", "GPT")))
        .unwrap();
    let mut bound = model("m1", "gpt-5");
    bound.standard_model_id = Some("s1".into());
    save(&db, &provider("p1", "oa"), &[bound]).unwrap();
    let err = db
        .with_conn(|c| standard_models::delete(c, "s1"))
        .unwrap_err();
    assert!(matches!(err, CoreError::Conflict(_)), "{err:?}");
    assert_eq!(
        err.message(),
        "标准模型 GPT 仍被 1 个服务商模型引用，请先解除关联"
    );
    // The schema's RESTRICT backs the repo check.
    let raw = db.with_conn(|c| Ok(c.execute("DELETE FROM standard_models WHERE id = 's1'", [])));
    assert!(raw.unwrap().is_err());
    // After unlinking, deletion works.
    save(&db, &provider("p1", "oa"), &[model("m1", "gpt-5")]).unwrap();
    db.with_conn(|c| standard_models::delete(c, "s1")).unwrap();
    assert_eq!(count(&db, "standard_models"), 0);
}

#[test]
fn bound_model_reads_standard_capabilities() {
    let db = Db::open_in_memory().unwrap();
    let mut s = standard("s1", "GPT");
    s.capabilities.context_window = Some(1000);
    db.with_conn(|c| standard_models::upsert(c, &s)).unwrap();
    let mut bound = model("m1", "gpt-5");
    bound.standard_model_id = Some("s1".into());
    save(&db, &provider("p1", "oa"), &[bound]).unwrap();
    s.capabilities.context_window = Some(2000);
    db.with_conn(|c| {
        standard_models::upsert(c, &s)?;
        assert_eq!(
            models::get(c, "m1")?.unwrap().capabilities.context_window,
            Some(2000)
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn standard_model_name_unique_per_company() {
    let db = Db::open_in_memory().unwrap();
    db.with_conn(|c| {
        standard_models::upsert(c, &standard("s1", "GPT"))?;
        let mut other_company = standard("s2", "GPT");
        other_company.company = Company::Anthropic;
        standard_models::upsert(c, &other_company)?;
        let err = standard_models::upsert(c, &standard("s3", "GPT")).unwrap_err();
        assert_eq!(err.code(), ErrorCode::Conflict);
        assert_eq!(standard_models::list(c)?.len(), 2);
        Ok(())
    })
    .unwrap();
}

#[test]
fn replace_catalog_entries_syncs_table() {
    let db = Db::open_in_memory().unwrap();
    db.with_tx(|tx| {
        standard_models::replace_catalog_entries(tx, &[standard("a", "A"), standard("b", "B")])
    })
    .unwrap();
    // Swap names, drop "b", add "c".
    db.with_tx(|tx| {
        standard_models::replace_catalog_entries(tx, &[standard("a", "B"), standard("c", "A")])
    })
    .unwrap();
    db.with_conn(|c| {
        let list = standard_models::list(c)?;
        let pairs: Vec<_> = list
            .iter()
            .map(|s| (s.id.as_str(), s.name.as_str()))
            .collect();
        assert_eq!(pairs, [("c", "A"), ("a", "B")]);
        Ok(())
    })
    .unwrap();
    // Referenced entries cannot be dropped; the transaction leaves the table untouched.
    let mut bound = model("m1", "gpt-5");
    bound.standard_model_id = Some("a".into());
    save(&db, &provider("p1", "oa"), &[bound]).unwrap();
    let err = db
        .with_tx(|tx| standard_models::replace_catalog_entries(tx, &[standard("c", "A")]))
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::Conflict);
    assert_eq!(count(&db, "standard_models"), 2);
}

#[test]
fn settings_default_row() {
    let db = Db::open_in_memory().unwrap();
    db.with_conn(|c| {
        assert_eq!(settings::get(c)?, Settings::default());
        let s = Settings {
            host: "0.0.0.0".into(),
            port: 9000,
            log_level: LogLevel::Debug,
        };
        settings::save(c, &s)?;
        assert_eq!(settings::get(c)?, s);
        let second = c.execute("INSERT INTO settings (id) VALUES (2)", []);
        assert!(second.is_err(), "CHECK (id = 1)");
        Ok(())
    })
    .unwrap();
    assert_eq!(count(&db, "settings"), 1);
}

#[test]
fn proxy_profile_roundtrip() {
    let db = Db::open_in_memory().unwrap();
    let fixed = ProxyProfile {
        client: Client::Pi,
        protocol: Protocol::Anthropic,
        route_mode: RouteMode::Fixed,
        models: vec![ProfileModel {
            provider_id: "p1".into(),
            model: "m".into(),
        }],
        default_model: Some("m".into()),
    };
    let auto = ProxyProfile {
        client: Client::Codex,
        protocol: Protocol::Responses,
        route_mode: RouteMode::Auto,
        models: vec![],
        default_model: None,
    };
    db.with_conn(|c| {
        terminal::save_proxy(c, &fixed)?;
        terminal::save_proxy(c, &auto)?;
        assert_eq!(terminal::get_proxy(c, "pi")?.unwrap(), fixed);
        assert_eq!(terminal::get_proxy(c, "codex")?.unwrap(), auto);
        assert!(terminal::get_proxy(c, "opencode")?.is_none());
        assert_eq!(terminal::list_proxy(c)?, vec![fixed.clone(), auto.clone()]);
        // Saving again replaces the row.
        let mut changed = fixed.clone();
        changed.protocol = Protocol::OpenaiChat;
        terminal::save_proxy(c, &changed)?;
        assert_eq!(terminal::get_proxy(c, "pi")?.unwrap(), changed);
        assert_eq!(terminal::list_proxy(c)?.len(), 2);
        // Invalid profiles are rejected before touching the table.
        let mut bad = fixed.clone();
        bad.client = Client::Codex;
        assert_eq!(
            terminal::save_proxy(c, &bad).unwrap_err().code(),
            ErrorCode::Validation
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn direct_profile_roundtrip_and_requires_provider() {
    let db = Db::open_in_memory().unwrap();
    let profile = DirectProfile {
        client: Client::ClaudeCode,
        provider_id: "p1".into(),
        model_source: ModelSource::Native,
        model: None,
    };
    let err = db
        .with_conn(|c| terminal::save_direct(c, &profile))
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::Conflict);
    save(&db, &provider("p1", "oa"), &[]).unwrap();
    db.with_conn(|c| {
        terminal::save_direct(c, &profile)?;
        assert_eq!(terminal::get_direct(c, "claude-code")?.unwrap(), profile);
        assert_eq!(terminal::list_direct(c)?, vec![profile.clone()]);
        assert!(terminal::get_direct(c, "pi")?.is_none());
        Ok(())
    })
    .unwrap();
}

#[test]
fn engine_set_and_clear() {
    let db = Db::open_in_memory().unwrap();
    save(&db, &provider("p1", "oa"), &[model("m1", "gpt-5")]).unwrap();
    db.with_conn(|c| {
        assert_eq!(engines::get(c, Client::Pi)?, None);
        engines::set(c, Client::Pi, Some("m1"))?;
        assert_eq!(engines::get(c, Client::Pi)?, Some("m1".into()));
        engines::set(c, Client::Pi, None)?;
        assert_eq!(engines::get(c, Client::Pi)?, None);
        let err = engines::set(c, Client::Pi, Some("missing")).unwrap_err();
        assert_eq!(err.code(), ErrorCode::Conflict);
        Ok(())
    })
    .unwrap();
}

fn assert_uuid_v4(s: &str) {
    let id = uuid::Uuid::parse_str(s).unwrap_or_else(|e| panic!("{s} is not a uuid: {e}"));
    assert_eq!(id.get_version_num(), 4, "{s}");
    assert_eq!(id.to_string(), s, "canonical lowercase form");
}

#[test]
fn model_ids_are_generated_when_empty_and_kept_when_supplied() {
    let db = Db::open_in_memory().unwrap();
    save(
        &db,
        &provider("p1", "oa"),
        &[
            model("", "gpt-5"),
            model("", "gpt-4"),
            model("custom-id", "o3"),
        ],
    )
    .unwrap();
    let before = db.with_conn(|c| models::list_by_provider(c, "p1")).unwrap();
    assert_eq!(before.len(), 3);
    assert_uuid_v4(&before[0].id);
    assert_uuid_v4(&before[1].id);
    assert_ne!(before[0].id, before[1].id);
    assert_eq!(before[2].id, "custom-id");
    // Re-saving with empty ids keeps the existing ones (matched by upstream).
    save(
        &db,
        &provider("p1", "oa"),
        &[model("", "gpt-5"), model("", "gpt-4"), model("", "o3")],
    )
    .unwrap();
    let after = db.with_conn(|c| models::list_by_provider(c, "p1")).unwrap();
    let ids = |v: &[Model]| v.iter().map(|m| m.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&before), ids(&after));
}

#[test]
fn usage_ids_are_uuid_v4_strings() {
    let db = Db::open_in_memory().unwrap();
    db.with_conn(|c| {
        usage::insert_batch(
            c,
            &[event(DAY1, None, "a", 1, 1), event(DAY1, None, "a", 1, 1)],
        )?;
        let r = usage::query(
            c,
            UsageQuery {
                start_ms: 0,
                end_ms: i64::MAX,
                model: None,
                client: None,
            },
        )?;
        assert_eq!(r.recent.len(), 2);
        for rec in &r.recent {
            assert_uuid_v4(&rec.id);
        }
        assert_ne!(r.recent[0].id, r.recent[1].id);
        Ok(())
    })
    .unwrap();
}

const DAY1: i64 = 1_767_355_200_000; // 2026-01-02T12:00:00Z
const DAY2: i64 = DAY1 + 86_400_000;

fn event(at: i64, client: Option<Client>, model: &str, input: u32, output: u32) -> UsageEvent {
    UsageEvent {
        requested_at: at,
        client,
        route_mode: client.map(|_| RouteMode::Auto),
        provider_name: "P".into(),
        protocol: Protocol::Responses,
        model: model.into(),
        input_tokens: input,
        output_tokens: output,
        cached_tokens: 1,
        reasoning_tokens: 2,
        step_id: None,
        session_id: None,
        ttft_ms: None,
        duration_ms: None,
        status: None,
    }
}

/// The schema as it stood before v4 (v1–v3 only).
fn migrations_through_v3() -> Migrations<'static> {
    Migrations::new(vec![migrations::v1(), migrations::v2(), migrations::v3()])
}

#[test]
fn v4_adds_usage_columns_and_keeps_old_rows() {
    let mut conn = Connection::open_in_memory().unwrap();
    migrations_through_v3().to_latest(&mut conn).unwrap();
    assert_eq!(user_version(&conn), 3);
    conn.execute(
        "INSERT INTO usage (id, requested_at, client, route_mode, provider_name, protocol, model, \
           input_tokens, output_tokens, cached_tokens, reasoning_tokens) \
         VALUES ('old', 1, 'pi', 'fixed', 'p', 'openai_chat', 'm', 1, 2, 3, 4)",
        [],
    )
    .unwrap();
    migrations::migrations().to_latest(&mut conn).unwrap();
    assert_eq!(user_version(&conn), 6);

    let r = usage::query(
        &conn,
        UsageQuery {
            start_ms: 0,
            end_ms: i64::MAX,
            model: None,
            client: None,
        },
    )
    .unwrap();
    assert_eq!(r.recent.len(), 1);
    let old = &r.recent[0];
    assert_eq!(old.id, "old");
    assert_eq!((old.input_tokens, old.output_tokens), (1, 2));
    assert_eq!(old.step_id, None);
    assert_eq!(old.session_id, None);
    assert_eq!(old.ttft_ms, None);
    assert_eq!(old.duration_ms, None);
    assert_eq!(old.status, None);
    assert_eq!(
        settings::get_trajectory(&conn).unwrap(),
        TrajectorySettings {
            enabled: true,
            disabled_clients: vec![],
            disabled_global: false,
            retention_days: 30,
            max_bytes: 2_147_483_648,
            notice_seen: false,
        }
    );
}

#[test]
fn trajectory_settings_round_trip_and_validation() {
    let db = Db::open_in_memory().unwrap();
    db.with_conn(|c| {
        let defaults = settings::get_trajectory(c)?;
        assert_eq!(
            defaults,
            TrajectorySettings {
                enabled: true,
                disabled_clients: vec![],
                disabled_global: false,
                retention_days: 30,
                max_bytes: 2_147_483_648,
                notice_seen: false,
            }
        );
        let saved = TrajectorySettings {
            enabled: false,
            disabled_clients: vec![Client::Pi, Client::Codex],
            disabled_global: true,
            retention_days: 3650,
            max_bytes: 64 * 1024 * 1024,
            notice_seen: true,
        };
        settings::save_trajectory(c, &saved)?;
        assert_eq!(settings::get_trajectory(c)?, saved);

        let mut bad = saved.clone();
        bad.retention_days = 0;
        assert_eq!(
            settings::save_trajectory(c, &bad).unwrap_err().code(),
            ErrorCode::Validation
        );
        bad.retention_days = 3651;
        assert_eq!(
            settings::save_trajectory(c, &bad).unwrap_err().code(),
            ErrorCode::Validation
        );
        bad = saved.clone();
        bad.max_bytes = 64 * 1024 * 1024 - 1;
        assert_eq!(
            settings::save_trajectory(c, &bad).unwrap_err().code(),
            ErrorCode::Validation
        );
        bad.max_bytes = 1024_u64.pow(4) + 1;
        assert_eq!(
            settings::save_trajectory(c, &bad).unwrap_err().code(),
            ErrorCode::Validation
        );
        // Rejected saves leave the stored row untouched.
        assert_eq!(settings::get_trajectory(c)?, saved);
        Ok(())
    })
    .unwrap();
}

#[test]
fn usage_insert_persists_new_columns() {
    let db = Db::open_in_memory().unwrap();
    let mut e = event(DAY1, Some(Client::Pi), "a", 10, 5);
    e.step_id = Some("step-1".into());
    e.session_id = Some("session-1".into());
    e.ttft_ms = Some(120);
    e.duration_ms = Some(900);
    e.status = Some(200);
    db.with_conn(|c| {
        usage::insert_batch(c, std::slice::from_ref(&e))?;
        let r = usage::query(
            c,
            UsageQuery {
                start_ms: 0,
                end_ms: i64::MAX,
                model: None,
                client: None,
            },
        )?;
        let rec = &r.recent[0];
        assert_eq!(rec.step_id.as_deref(), Some("step-1"));
        assert_eq!(rec.session_id.as_deref(), Some("session-1"));
        assert_eq!(rec.ttft_ms, Some(120));
        assert_eq!(rec.duration_ms, Some(900));
        assert_eq!(rec.status, Some(200));
        Ok(())
    })
    .unwrap();
}

#[test]
fn usage_query_aggregates_by_day_model_and_client() {
    let db = Db::open_in_memory().unwrap();
    db.with_conn(|c| {
        usage::insert_batch(
            c,
            &[
                event(DAY1, Some(Client::Pi), "a", 10, 5),
                event(DAY1 + 1000, Some(Client::Codex), "b", 20, 10),
                event(DAY2, Some(Client::Pi), "a", 100, 50),
                event(DAY2 + 1000, None, "a", 1, 1),
                event(DAY2 + 86_400_000 * 5, None, "out-of-range", 9, 9),
            ],
        )?;
        usage::insert_batch(c, &[])?;
        let q = |model: Option<&str>| UsageQuery {
            start_ms: DAY1 - 1000,
            end_ms: DAY2 + 86_400_000,
            model: model.map(str::to_owned),
            client: None,
        };
        let r = usage::query(c, q(None))?;
        assert_eq!(r.total_requests, 4);
        assert_eq!((r.input_tokens, r.output_tokens), (131, 66));
        assert_eq!((r.cached_tokens, r.reasoning_tokens), (4, 8));
        assert_eq!(r.days.len(), 2);
        assert_eq!((r.days[0].input_tokens, r.days[1].input_tokens), (30, 101));
        let dm: Vec<_> = r
            .daily_models
            .iter()
            .map(|d| (d.model.as_str(), d.tokens))
            .collect();
        assert_eq!(dm, [("b", 30), ("a", 15), ("a", 152)]);
        let by_model: Vec<_> = r
            .models
            .iter()
            .map(|m| (m.model.as_str(), m.requests))
            .collect();
        assert_eq!(by_model, [("a", 3), ("b", 1)]);
        let by_client: Vec<_> = r.by_client.iter().map(|c| (c.client, c.requests)).collect();
        assert_eq!(
            by_client,
            [(Some(Client::Pi), 2), (Some(Client::Codex), 1), (None, 1)]
        );
        assert_eq!(r.recent.len(), 4);
        assert_eq!(r.recent[0].requested_at, DAY2 + 1000);
        assert_eq!(r.recent[0].client, None);
        assert_eq!(r.recent[0].route_mode, None);
        assert_eq!(r.recent[1].client, Some(Client::Pi));
        assert_eq!(r.recent[1].route_mode, Some(RouteMode::Auto));

        let only_a = usage::query(c, q(Some("a")))?;
        assert_eq!(only_a.total_requests, 3);
        assert_eq!(only_a.models.len(), 1);
        // End is exclusive.
        let narrow = usage::query(
            c,
            UsageQuery {
                start_ms: DAY1,
                end_ms: DAY1 + 1000,
                model: None,
                client: None,
            },
        )?;
        assert_eq!(narrow.total_requests, 1);
        let empty = usage::query(
            c,
            UsageQuery {
                start_ms: 0,
                end_ms: 1,
                model: None,
                client: None,
            },
        )?;
        assert_eq!(
            (empty.total_requests, empty.days.len(), empty.recent.len()),
            (0, 0, 0)
        );
        Ok(())
    })
    .unwrap();
}

fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |r| r.get::<_, i64>(0),
    )
    .unwrap()
        == 1
}

#[test]
fn trajectory_db_creates_schema_with_incremental_vacuum() {
    let tmp = tempfile::tempdir().unwrap();
    let db = yi_core::storage::trajectory::TrajectoryDb::open(&tmp.path().join("trajectory.db"))
        .unwrap();
    db.0.with_conn(|c| {
        for table in ["sessions", "threads", "steps", "step_items", "blobs"] {
            assert!(table_exists(c, table), "missing table {table}");
        }
        // v2 is the change-cursor meta migration (`meta.last_seq`).
        assert_eq!(user_version(c), 2);
        let auto_vacuum: i64 = c.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))?;
        assert_eq!(auto_vacuum, 2);
        Ok(())
    })
    .unwrap();
}

#[test]
fn main_and_trajectory_migrations_are_independent() {
    let tmp = tempfile::tempdir().unwrap();
    let main = Db::open(&tmp.path().join("yi-token.db")).unwrap();
    let traj = yi_core::storage::trajectory::TrajectoryDb::open(&tmp.path().join("trajectory.db"))
        .unwrap();
    main.with_conn(|c| {
        assert!(!table_exists(c, "steps"));
        assert!(table_exists(c, "providers"));
        Ok(())
    })
    .unwrap();
    traj.0
        .with_conn(|c| {
            assert!(!table_exists(c, "providers"));
            assert!(table_exists(c, "steps"));
            Ok(())
        })
        .unwrap();
}

fn migrations_through_v5() -> Migrations<'static> {
    Migrations::new(vec![
        migrations::v1(),
        migrations::v2(),
        migrations::v3(),
        migrations::v4(),
        migrations::v5(),
    ])
}

/// Chat and Responses rows with the same name fold into one OpenAI row (Responses wins),
/// and models follow their survivor. Anthropic rows keep their own company.
#[test]
fn v6_merges_protocol_standards_into_companies() {
    let mut conn = Connection::open_in_memory().unwrap();
    migrations_through_v5().to_latest(&mut conn).unwrap();
    conn.execute_batch(
        "INSERT INTO standard_models (id, name, protocol, brand, capabilities) VALUES
           ('gpt-chat', 'gpt-x', 'openai_chat', '', '{}'),
           ('gpt-resp', 'gpt-x', 'responses', 'openai', '{}'),
           ('claude', 'claude-y', 'anthropic', '', '{}');
         INSERT INTO providers (id, type, name, short_code, base_url) VALUES
           ('p1', 'responses', 'Alpha', 'p1', 'https://a.test');
         INSERT INTO models (id, provider_id, standard_model_id, upstream_model, exposed_name) VALUES
           ('m-chat', 'p1', 'gpt-chat', 'gpt-x', 'Alpha/gpt-x'),
           ('m-resp', 'p1', 'gpt-resp', 'gpt-x-2', 'Alpha/gpt-x-2'),
           ('m-claude', 'p1', 'claude', 'claude-y', 'Alpha/claude-y');",
    )
    .unwrap();
    migrations::migrations().to_latest(&mut conn).unwrap();
    assert_eq!(user_version(&conn), 6);

    let rows: Vec<(String, String, String)> = conn
        .prepare("SELECT id, name, company FROM standard_models ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            ("claude".into(), "claude-y".into(), "anthropic".into()),
            ("gpt-resp".into(), "gpt-x".into(), "openai".into()),
        ]
    );
    let linked: Vec<(String, Option<String>)> = conn
        .prepare("SELECT id, standard_model_id FROM models ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        linked,
        vec![
            ("m-chat".into(), Some("gpt-resp".into())),
            ("m-claude".into(), Some("claude".into())),
            ("m-resp".into(), Some("gpt-resp".into())),
        ]
    );
    let overrides: i64 = conn
        .query_row("SELECT COUNT(*) FROM models WHERE protocol IS NOT NULL", [], |r| r.get(0))
        .unwrap();
    assert_eq!(overrides, 0);
}

