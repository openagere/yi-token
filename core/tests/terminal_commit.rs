use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use yi_core::domain::{
    Client, DirectProfile, Model, ModelSource, ProfileModel, Protocol, ProxyProfile, RouteMode,
};
use yi_core::error::{CoreError, CoreResult, ErrorCode};
use yi_core::terminal::{
    ClientCtx, ClientSpec, Doc, DocFormat, FileChange, TerminalMode, TerminalPlan, apply,
    build_plan,
};

struct Dummy {
    files: Vec<(PathBuf, DocFormat)>,
    secrets: Vec<String>,
}

impl ClientSpec for Dummy {
    fn client(&self) -> Client {
        Client::Codex
    }

    fn files(&self, _ctx: &ClientCtx) -> Vec<(PathBuf, DocFormat)> {
        self.files.clone()
    }

    fn render_proxy(
        &self,
        docs: &mut [Doc],
        _profile: &ProxyProfile,
        ctx: &ClientCtx,
    ) -> CoreResult<Vec<String>> {
        docs[0].set(
            &["yitoken"],
            json!({"api_key": "sk-secret", "base_url": ctx.proxy_base}),
        )?;
        if let Some(doc) = docs.get_mut(1) {
            doc.set(&["version"], json!(1))?;
        }
        Ok(self.secrets.clone())
    }

    fn render_direct(
        &self,
        docs: &mut [Doc],
        _profile: &DirectProfile,
        _ctx: &ClientCtx,
    ) -> CoreResult<Vec<String>> {
        docs[0].set(&["direct"], json!(true))?;
        Ok(self.secrets.clone())
    }

    fn render_clear(&self, docs: &mut [Doc], _ctx: &ClientCtx) -> CoreResult<Vec<bool>> {
        Ok(vec![false; docs.len()])
    }
}

fn ctx(home: &Path) -> ClientCtx {
    ClientCtx {
        proxy_base: "http://127.0.0.1:11435".into(),
        providers: Vec::new(),
        models: Vec::<Model>::new(),
        standard_models: Vec::new(),
        active_engine: None,
        home: home.to_path_buf(),
        env: HashMap::new(),
    }
}

fn profile() -> ProxyProfile {
    ProxyProfile {
        client: Client::Codex,
        protocol: Protocol::Responses,
        route_mode: RouteMode::Fixed,
        models: vec![ProfileModel {
            provider_id: "p1".into(),
            model: "m1".into(),
        }],
        default_model: Some("m1".into()),
    }
}

fn dummy(path: &Path, format: DocFormat) -> Dummy {
    Dummy {
        files: vec![(path.to_path_buf(), format)],
        secrets: vec!["sk-secret".into()],
    }
}

fn plan(dummy: &Dummy, ctx: &ClientCtx, profile: &ProxyProfile) -> TerminalPlan {
    build_plan(dummy, TerminalMode::Proxy(profile), ctx).unwrap()
}

#[test]
fn json_retain_removes_only_marked_entries() {
    let mut doc = Doc::parse(
        DocFormat::Json,
        r#"{
            "provider": {
                "yitoken": {"name": "ours"},
                "yitoken_direct_p1": {"name": "ours direct"},
                "yi": {"name": "yi-llm"},
                "user": {"name": "user"}
            }
        }"#,
    )
    .unwrap();
    doc.retain_in(&["provider"], |key, _| key.starts_with("yitoken"))
        .unwrap();
    assert_eq!(
        doc.get(&["provider"]).unwrap(),
        json!({
            "yitoken": {"name": "ours"},
            "yitoken_direct_p1": {"name": "ours direct"}
        })
    );
}

#[test]
fn parse_error_does_not_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    fs::write(&path, "{ broken").unwrap();
    let before = fs::read_to_string(&path).unwrap();
    let dummy = dummy(&path, DocFormat::Json);
    let error = build_plan(&dummy, TerminalMode::Proxy(&profile()), &ctx(dir.path())).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Validation);
    assert!(error.to_string().contains(&path.display().to_string()));
    assert!(error.to_string().contains("未做任何修改"));
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn missing_file_is_created() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/config.json");
    let dummy = dummy(&path, DocFormat::Json);
    let prepared = plan(&dummy, &ctx(dir.path()), &profile());
    let applied = apply(&prepared, || Ok(())).unwrap();
    assert!(applied.backup_paths.is_empty());
    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(written["yitoken"]["api_key"], "sk-secret");
}

#[test]
fn apply_creates_backup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    fs::write(&path, "{\"user\":true}\n").unwrap();
    let dummy = dummy(&path, DocFormat::Json);
    let prepared = plan(&dummy, &ctx(dir.path()), &profile());
    let applied = apply(&prepared, || Ok(())).unwrap();
    assert_eq!(applied.backup_paths.len(), 1);
    let backup = PathBuf::from(&applied.backup_paths[0]);
    assert_eq!(backup.parent(), Some(dir.path()));
    let name = backup.file_name().unwrap().to_string_lossy();
    assert!(name.starts_with("config.json.bak-"), "{name}");
    assert_eq!(fs::read_to_string(backup).unwrap(), "{\"user\":true}\n");
    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(written["user"], true);
    assert_eq!(written["yitoken"]["api_key"], "sk-secret");
}

#[test]
fn apply_rolls_back_on_db_failure() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("config.json");
    let second = dir.path().join("meta.json");
    fs::write(&first, "{\"user\":true}\n").unwrap();
    let dummy = Dummy {
        files: vec![
            (first.clone(), DocFormat::Json),
            (second.clone(), DocFormat::Json),
        ],
        secrets: vec!["sk-secret".into()],
    };
    let prepared = plan(&dummy, &ctx(dir.path()), &profile());
    let error = apply(&prepared, || Err(CoreError::Database("db down".into()))).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Database);
    assert!(error.to_string().contains("已恢复原配置"));
    assert_eq!(fs::read_to_string(&first).unwrap(), "{\"user\":true}\n");
    assert!(!second.exists());
}

#[test]
fn apply_rolls_back_on_second_file_failure() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("config.json");
    let blocked = dir.path().join("blocked");
    fs::write(&first, "{\"user\":true}\n").unwrap();
    fs::write(&blocked, "not a directory").unwrap();
    let second = blocked.join("config.json");
    let prepared = TerminalPlan {
        client: Client::Codex,
        config_path: first.clone(),
        endpoint: "http://127.0.0.1:11435/clients/codex/v1".into(),
        changes: vec![
            FileChange {
                path: first.clone(),
                format: DocFormat::Json,
                before: Some("{\"user\":true}\n".into()),
                after: Some("{\"yitoken\":{}}\n".into()),
                secrets: Vec::new(),
            },
            FileChange {
                path: second,
                format: DocFormat::Json,
                before: None,
                after: Some("{}\n".into()),
                secrets: Vec::new(),
            },
        ],
        rendered: Vec::new(),
        secrets: Vec::new(),
    };
    assert!(apply(&prepared, || Ok(())).is_err());
    assert_eq!(fs::read_to_string(&first).unwrap(), "{\"user\":true}\n");
}

#[test]
fn preview_masks_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let dummy = dummy(&path, DocFormat::Json);
    let prepared = plan(&dummy, &ctx(dir.path()), &profile());
    let preview = prepared.preview();
    assert_eq!(preview.files.len(), 1);
    assert!(!preview.files[0].content.contains("sk-secret"));
    assert!(preview.files[0].content.contains("••••••"));
    assert_eq!(preview.endpoint, "http://127.0.0.1:11435/clients/codex/v1");
    assert_eq!(preview.config_path, path.display().to_string());
}

#[test]
fn direct_plan_uses_provider_endpoint() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let dummy = dummy(&path, DocFormat::Json);
    let mut ctx = ctx(dir.path());
    ctx.providers.push(yi_core::domain::Provider {
        id: "p1".into(),
        provider_type: Protocol::OpenaiChat,
        name: "OpenAI".into(),
        base_url: "https://api.openai.com/v1".into(),
        api_key: "sk-real".into(),
        enabled: true,
        thinking: None,
        is_default: false,
        sort_order: 0,
        extra: json!({}),
    });
    let direct = DirectProfile {
        client: Client::Codex,
        provider_id: "p1".into(),
        model_source: ModelSource::Provider,
        model: None,
    };
    let prepared = build_plan(&dummy, TerminalMode::Direct(&direct), &ctx).unwrap();
    assert_eq!(prepared.endpoint, "https://api.openai.com/v1");
}
