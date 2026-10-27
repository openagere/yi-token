use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use yi_core::domain::{
    Capabilities, Client, DirectProfile, Effort, Modality, Model, ModelSource, ProfileModel,
    Protocol, Provider, ProxyProfile, RouteMode, StandardModel, Support,
};
use yi_core::terminal::clients::spec_for;
use yi_core::terminal::{ClientCtx, TerminalMode, apply, build_plan};
use yi_core::terminal::{Doc, DocFormat};

fn caps() -> Capabilities {
    Capabilities {
        input_modalities: Some(vec![Modality::Text, Modality::Image]),
        output_modalities: Some(vec![Modality::Text]),
        context_window: Some(200_000),
        max_output_tokens: Some(8_192),
        effort: Effort {
            support: Support::Supported,
            levels: vec!["low".into(), "medium".into(), "high".into()],
            default: Some("high".into()),
        },
    }
}

fn provider(id: &str, name: &str, protocol: Protocol) -> Provider {
    Provider {
        id: id.into(),
        provider_type: protocol,
        name: name.into(),
        base_url: "https://upstream.example/v1".into(),
        api_key: format!("{id}-secret"),
        enabled: true,
        thinking: None,
        is_default: id == "p1",
        sort_order: 0,
        extra: serde_json::json!({}),
        protocols: vec![
            Protocol::OpenaiChat,
            Protocol::Responses,
            Protocol::Anthropic,
        ],
    }
}

fn model(id: &str, provider_id: &str, upstream: &str, exposed: &str) -> Model {
    Model {
        id: id.into(),
        provider_id: provider_id.into(),
        standard_model_id: None,
        upstream_model: upstream.into(),
        exposed_name: exposed.into(),
        capabilities: caps(),
        non_standard: false,
    }
}

fn ctx(home: &Path) -> ClientCtx {
    ClientCtx {
        proxy_base: "http://127.0.0.1:11435".into(),
        providers: vec![
            provider("p1", "OpenAI", Protocol::OpenaiChat),
            provider("p2", "Responses", Protocol::Responses),
            provider("p3", "Anthropic", Protocol::Anthropic),
        ],
        models: vec![
            model("m1", "p1", "up-1", "real-one(one)"),
            model("m2", "p1", "up-2", "real-two(two)"),
            model("r1", "p2", "resp-model", "resp-model(resp)"),
            model("a1", "p3", "claude-model", "claude-model(claude)"),
        ],
        standard_models: Vec::<StandardModel>::new(),
        active_engine: None,
        home: home.to_path_buf(),
        env: HashMap::new(),
    }
}

fn proxy_profile(client: Client, protocol: Protocol) -> ProxyProfile {
    ProxyProfile {
        client,
        protocol,
        route_mode: RouteMode::Fixed,
        models: vec![
            ProfileModel {
                provider_id: "p1".into(),
                model: "real-one(one)".into(),
            },
            ProfileModel {
                provider_id: "p1".into(),
                model: "real-two(two)".into(),
            },
        ],
        default_model: Some("real-one(one)".into()),
    }
}

fn auto_profile(client: Client, protocol: Protocol) -> ProxyProfile {
    ProxyProfile {
        client,
        protocol,
        route_mode: RouteMode::Auto,
        models: Vec::new(),
        default_model: None,
    }
}

fn ctx_with_engine(home: &Path, model_id: Option<&str>) -> ClientCtx {
    let mut ctx = ctx(home);
    let engine = model_id.and_then(|id| ctx.models.iter().find(|model| model.id == id).cloned());
    ctx.active_engine = engine;
    ctx
}

fn direct_profile(client: Client) -> DirectProfile {
    let provider_id = match client {
        Client::Codex => "p2",
        Client::ClaudeCode => "p3",
        _ => "p1",
    };
    DirectProfile {
        client,
        provider_id: provider_id.into(),
        model_source: ModelSource::Provider,
        model: None,
    }
}

fn primary_path(client: Client, ctx: &ClientCtx) -> PathBuf {
    spec_for(client).files(ctx)[0].0.clone()
}

fn normalized(home: &Path, text: &str) -> String {
    let raw = home.display().to_string();
    let escaped = raw.replace('\\', "\\\\");
    let slashed = raw.replace('\\', "/");
    text.replace(&escaped, "[HOME]")
        .replace(&raw, "[HOME]")
        .replace(&slashed, "[HOME]")
}

fn render(client: Client, mode: TerminalMode<'_>, ctx: &ClientCtx) -> String {
    let plan = build_plan(spec_for(client), mode, ctx).unwrap();
    let mut files: Vec<String> = plan
        .changes
        .iter()
        .filter_map(|change| {
            let content = normalized(&ctx.home, change.after.as_ref()?);
            Some(format!(
                "=== {} ===\n{}",
                normalized(
                    &ctx.home,
                    &change.path.display().to_string().replace('\\', "/")
                ),
                content
            ))
        })
        .collect();
    files.sort();
    files.join("\n")
}

fn write_original(client: Client, ctx: &ClientCtx, text: &str) {
    let path = primary_path(client, ctx);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn original_for(client: Client) -> &'static str {
    match client {
        Client::Codex => {
            "# user comment\n[model_providers.user]\nname = \"keep\"\nbase_url = \"https://user.example/v1\"\n"
        }
        Client::ClaudeCode => "{\"user\":true,\"env\":{\"USER_TOKEN\":\"keep\"}}\n",
        Client::Opencode => {
            "{\"provider\":{\"user\":{\"name\":\"keep\",\"npm\":\"@ai-sdk/openai\"}}}\n"
        }
        Client::Pi => {
            "{\"providers\":{\"user\":{\"name\":\"keep\",\"baseUrl\":\"https://user.example/v1\"}}}\n"
        }
        Client::DeepseekHarness => "- id: user\n  config:\n    root: keep\n",
    }
}

fn yi_llm_original(client: Client) -> &'static str {
    match client {
        Client::Codex => {
            "# YI_LLM_PROXY\n[model_providers.yi]\nname = \"yi\"\n[model_providers.yi_direct_legacy]\nname = \"yi-llm direct:legacy:Legacy\"\n"
        }
        Client::ClaudeCode => {
            "{\"env\":{\"ANTHROPIC_BASE_URL\":\"https://yi-llm.example\",\"YI_LLM_PROXY\":\"yi\"},\"yi\":{\"name\":\"yi\"},\"yi_direct_legacy\":{\"name\":\"yi-llm direct:legacy\"}}\n"
        }
        Client::Opencode => {
            "{\"provider\":{\"yi\":{\"name\":\"yi\",\"npm\":\"@ai-sdk/openai-compatible\"},\"yi_direct_legacy\":{\"name\":\"yi-llm direct:legacy\"},\"env\":{\"YI_LLM_PROXY\":\"yi\"}}}\n"
        }
        Client::Pi => {
            "{\"providers\":{\"yi\":{\"name\":\"yi\",\"api\":\"openai-completions\"},\"yi_direct_legacy\":{\"name\":\"yi-llm direct:legacy\"},\"env\":{\"YI_LLM_PROXY\":\"yi\"}}}\n"
        }
        Client::DeepseekHarness => {
            "- id: llm-pi-ai\n  config:\n    providers:\n      yi:\n        name: yi\n      yi_direct_legacy:\n        name: 'yi-llm direct:legacy'\n      env:\n        YI_LLM_PROXY: yi\n"
        }
    }
}

fn clients() -> [Client; 5] {
    Client::ALL
}

#[test]
fn proxy_fresh_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ctx(dir.path());
    for client in clients() {
        let profile = proxy_profile(client, client.default_protocol());
        let rendered = render(client, TerminalMode::Proxy(&profile), &ctx);
        insta::with_settings!({snapshot_suffix => client.as_str().replace('-', "_")}, {
            insta::assert_snapshot!(rendered);
        });
    }
}

#[test]
fn proxy_preserves_user_content() {
    for client in clients() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(dir.path());
        write_original(client, &ctx, original_for(client));
        let profile = proxy_profile(client, client.default_protocol());
        let rendered = render(client, TerminalMode::Proxy(&profile), &ctx);
        assert!(rendered.contains("keep"), "{client:?}: {rendered}");
        if client == Client::Codex {
            assert!(rendered.contains("# user comment"));
        }
        assert_eq!(
            fs::read_to_string(primary_path(client, &ctx)).unwrap(),
            original_for(client),
            "planning must not modify the file"
        );
    }
}

#[test]
fn proxy_preserves_yi_llm_entries() {
    for client in clients() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(dir.path());
        write_original(client, &ctx, yi_llm_original(client));
        let profile = proxy_profile(client, client.default_protocol());
        let rendered = render(client, TerminalMode::Proxy(&profile), &ctx);
        assert!(rendered.contains("yi-llm direct:legacy"), "{client:?}");
        assert!(rendered.contains("yi_direct_legacy"), "{client:?}");
        assert!(rendered.contains("YI_LLM_PROXY"), "{client:?}");
        assert!(
            rendered.contains("\"yi\"")
                || rendered.contains("name = \"yi\"")
                || rendered.contains("name: yi"),
            "{client:?}: {rendered}"
        );
    }
}

#[test]
fn proxy_rerender_is_idempotent() {
    for client in clients() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(dir.path());
        let profile = proxy_profile(client, client.default_protocol());
        let first = build_plan(spec_for(client), TerminalMode::Proxy(&profile), &ctx).unwrap();
        assert!(!first.changes.is_empty(), "{client:?}");
        apply(&first, || Ok(())).unwrap();
        let second = build_plan(spec_for(client), TerminalMode::Proxy(&profile), &ctx).unwrap();
        assert!(
            second.changes.is_empty(),
            "{client:?}: {:?}",
            second.changes
        );
    }
}

#[test]
fn proxy_auto_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ctx_with_engine(dir.path(), Some("m1"));
    for client in clients() {
        let profile = auto_profile(client, client.default_protocol());
        let rendered = render(client, TerminalMode::Proxy(&profile), &ctx);
        let suffix = format!("{}_proxy_auto", client.as_str().replace('-', "_"));
        insta::with_settings!({snapshot_suffix => suffix.as_str()}, {
            insta::assert_snapshot!(rendered);
        });
    }
}

#[test]
fn codex_proxy_writes_the_selected_engine_capabilities() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx_with_engine(dir.path(), Some("m1"));
    let engine = ctx.active_engine.as_mut().unwrap();
    engine.capabilities.context_window = Some(128_000);
    engine.capabilities.max_output_tokens = Some(4_096);
    engine.capabilities.input_modalities = Some(vec![Modality::Text]);
    let profile = auto_profile(Client::Codex, Protocol::Responses);
    let rendered = render(Client::Codex, TerminalMode::Proxy(&profile), &ctx);
    assert!(rendered.contains("model = \"yi-token\""));
    assert!(rendered.contains("\"slug\": \"yi-token\""));
    assert!(rendered.contains("\"context_window\": 128000"));
    assert!(rendered.contains("\"max_context_window\": 128000"));
    assert!(rendered.contains("\"text\""));
    assert!(!rendered.contains("\"image\""));
    insta::assert_snapshot!(
        "codex_proxy_writes_the_selected_engine_capabilities",
        rendered
    );

    let claude = render(
        Client::ClaudeCode,
        TerminalMode::Proxy(&auto_profile(Client::ClaudeCode, Protocol::Anthropic)),
        &ctx,
    );
    assert!(claude.contains("\"model\": \"yi-token\""));
    assert!(claude.contains("\"CLAUDE_CODE_MAX_CONTEXT_TOKENS\": \"128000\""));
    assert!(claude.contains("\"CLAUDE_CODE_MAX_OUTPUT_TOKENS\": \"4096\""));
    assert!(!claude.contains("Auto"));
}

#[test]
fn codex_auto_metadata_without_engine_uses_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ctx_with_engine(dir.path(), None);
    let profile = auto_profile(Client::Codex, Protocol::Responses);
    let rendered = render(Client::Codex, TerminalMode::Proxy(&profile), &ctx);
    insta::assert_snapshot!("codex_auto_metadata_without_engine_uses_defaults", rendered);
}

#[test]
fn direct_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ctx(dir.path());
    for client in clients() {
        let mut profile = direct_profile(client);
        let suffix = format!("{}_provider", client.as_str().replace('-', "_"));
        let rendered = render(client, TerminalMode::Direct(&profile), &ctx);
        insta::with_settings!({snapshot_suffix => suffix.as_str()}, {
            insta::assert_snapshot!(rendered);
        });
        if matches!(client, Client::Codex | Client::ClaudeCode) {
            profile.model_source = ModelSource::Native;
            let suffix = format!("{}_native", client.as_str().replace('-', "_"));
            let rendered = render(client, TerminalMode::Direct(&profile), &ctx);
            insta::with_settings!({snapshot_suffix => suffix.as_str()}, {
                insta::assert_snapshot!(rendered);
            });
        }
    }
}

#[test]
fn multi_protocol_clients_use_selected_protocol() {
    for client in [Client::Opencode, Client::Pi, Client::DeepseekHarness] {
        for protocol in Protocol::all() {
            let dir = tempfile::tempdir().unwrap();
            let ctx = ctx(dir.path());
            let profile = proxy_profile(client, protocol);
            let plan = build_plan(spec_for(client), TerminalMode::Proxy(&profile), &ctx).unwrap();
            let main = plan
                .changes
                .iter()
                .find(|change| change.path == primary_path(client, &ctx))
                .and_then(|change| change.after.as_ref())
                .unwrap();
            match client {
                Client::Opencode => {
                    let value: Value = serde_json::from_str(main).unwrap();
                    let npm = match protocol {
                        Protocol::OpenaiChat => "@ai-sdk/openai-compatible",
                        Protocol::Responses => "@ai-sdk/openai",
                        Protocol::Anthropic => "@ai-sdk/anthropic",
                    };
                    assert_eq!(value["provider"]["yitoken"]["npm"], npm);
                    assert_eq!(
                        value["provider"]["yitoken"]["options"]["baseURL"],
                        format!("http://127.0.0.1:11435/clients/opencode/v1")
                    );
                }
                Client::Pi => {
                    let value: Value = serde_json::from_str(main).unwrap();
                    let api = match protocol {
                        Protocol::OpenaiChat => "openai-completions",
                        Protocol::Responses => "openai-responses",
                        Protocol::Anthropic => "anthropic-messages",
                    };
                    assert_eq!(value["providers"]["yitoken"]["api"], api);
                }
                Client::DeepseekHarness => {
                    let value: Value = serde_norway::from_str(main).unwrap();
                    let api = match protocol {
                        Protocol::OpenaiChat => "openai-completions",
                        Protocol::Responses => "openai-responses",
                        Protocol::Anthropic => "anthropic-messages",
                    };
                    assert_eq!(value[0]["config"]["providers"]["yitoken"]["api"], api);
                }
                _ => unreachable!(),
            }
            let rendered = render(client, TerminalMode::Proxy(&profile), &ctx);
            let suffix = format!(
                "{}_{}",
                client.as_str().replace('-', "_"),
                protocol.as_str()
            );
            insta::with_settings!({snapshot_suffix => suffix.as_str()}, {
                insta::assert_snapshot!(rendered);
            });
        }
    }
}

#[test]
fn env_overrides_config_path() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(dir.path());
    let custom = dir.path().join("custom");
    ctx.env.insert(
        "CODEX_HOME".into(),
        custom.join("codex").display().to_string(),
    );
    ctx.env.insert(
        "CLAUDE_CONFIG_DIR".into(),
        custom.join("claude").display().to_string(),
    );
    ctx.env.insert(
        "OPENCODE_CONFIG".into(),
        custom.join("opencode.jsonc").display().to_string(),
    );
    ctx.env
        .insert("DSH_HOME".into(), custom.join("dsh").display().to_string());
    fs::create_dir_all(custom.join("dsh/profiles/web")).unwrap();

    assert!(primary_path(Client::Codex, &ctx).ends_with("codex/config.toml"));
    assert!(primary_path(Client::ClaudeCode, &ctx).ends_with("claude/settings.json"));
    let opencode = spec_for(Client::Opencode).files(&ctx);
    assert_eq!(opencode[0].0, custom.join("opencode.jsonc"));
    assert_eq!(opencode[0].1, DocFormat::Jsonc);
    assert!(primary_path(Client::DeepseekHarness, &ctx).ends_with("profiles/web/cordis.patch.yml"));

    let mut xdg = ctx.clone();
    xdg.env.remove("OPENCODE_CONFIG");
    xdg.env.insert(
        "XDG_CONFIG_HOME".into(),
        custom.join("xdg").display().to_string(),
    );
    fs::create_dir_all(custom.join("xdg/opencode")).unwrap();
    fs::write(custom.join("xdg/opencode/opencode.jsonc"), "{}").unwrap();
    assert_eq!(
        primary_path(Client::Opencode, &xdg),
        custom.join("xdg/opencode/opencode.jsonc")
    );

    let mut pi = ctx.clone();
    pi.env.insert(
        "PI_CODING_AGENT_DIR".into(),
        custom.join("pi/agent").display().to_string(),
    );
    assert!(primary_path(Client::Pi, &pi).ends_with("pi/agent/models.json"));
}

#[test]
fn direct_native_leaves_no_model_or_metadata_reference_for_codex() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ctx(dir.path());
    let mut profile = direct_profile(Client::Codex);
    profile.model_source = ModelSource::Native;
    let plan = build_plan(
        spec_for(Client::Codex),
        TerminalMode::Direct(&profile),
        &ctx,
    )
    .unwrap();
    assert_eq!(plan.changes.len(), 1);
    assert!(
        plan.changes
            .iter()
            .all(|change| !change.path.ends_with("yi-token-models.json"))
    );
    let config = plan
        .changes
        .iter()
        .find(|change| change.path == primary_path(Client::Codex, &ctx))
        .and_then(|change| change.after.as_ref())
        .unwrap();
    let doc: Doc = Doc::parse(DocFormat::Toml, config).unwrap();
    assert!(doc.get(&["model"]).is_none());
    assert!(doc.get(&["model_catalog_json"]).is_none());
    assert_eq!(
        doc.get(&["model_provider"]).unwrap(),
        serde_json::json!("yitoken_direct_p2")
    );
}

#[test]
fn direct_native_removes_proxy_model_selection_for_claude_code() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ctx(dir.path());
    write_original(
        Client::ClaudeCode,
        &ctx,
        r#"{"model":"claude-up(zz)","modelPicker":{"options":[{"model":"claude-up(zz)","label":"claude-up(zz)"}]}}"#,
    );
    let mut profile = direct_profile(Client::ClaudeCode);
    profile.model_source = ModelSource::Native;
    let plan = build_plan(
        spec_for(Client::ClaudeCode),
        TerminalMode::Direct(&profile),
        &ctx,
    )
    .unwrap();
    let config = plan
        .changes
        .iter()
        .find(|change| change.path == primary_path(Client::ClaudeCode, &ctx))
        .and_then(|change| change.after.as_ref())
        .unwrap();
    let doc: Doc = Doc::parse(DocFormat::Json, config).unwrap();
    assert!(doc.get(&["model"]).is_none());
    assert!(doc.get(&["modelPicker"]).is_none());
}

#[test]
fn empty_existing_config_is_treated_as_empty_document() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ctx(dir.path());
    write_original(Client::ClaudeCode, &ctx, "  \n");
    let profile = direct_profile(Client::ClaudeCode);
    let plan = build_plan(
        spec_for(Client::ClaudeCode),
        TerminalMode::Direct(&profile),
        &ctx,
    );
    assert!(plan.is_ok(), "empty config must not block apply: {plan:?}");
}
