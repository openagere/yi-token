//! One client's declarative configuration surface and the shared plan builder.

use super::doc::{Doc, DocFormat};
use super::plan::TerminalPlan;
use crate::domain::{
    Client, DirectProfile, Model, Protocol, Provider, ProxyProfile, StandardModel,
};
use crate::error::{CoreError, CoreResult};
use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Immutable inputs shared by every client renderer. Tests inject `home` and
/// `env` so snapshots never touch the real user directory.
#[derive(Clone)]
pub struct ClientCtx {
    /// `http://host:port`, without a trailing slash.
    pub proxy_base: String,
    pub providers: Vec<Provider>,
    pub models: Vec<Model>,
    pub standard_models: Vec<StandardModel>,
    pub active_engine: Option<Model>,
    pub home: PathBuf,
    pub env: HashMap<String, String>,
}

pub enum TerminalMode<'a> {
    Proxy(&'a ProxyProfile),
    Direct(&'a DirectProfile),
    /// Remove yi-token owned proxy and direct settings, leaving the user's other config in place.
    Clear(Client),
}

impl TerminalMode<'_> {
    pub fn client(&self) -> Client {
        match self {
            Self::Proxy(profile) => profile.client,
            Self::Direct(profile) => profile.client,
            Self::Clear(client) => *client,
        }
    }
}

/// Every managed terminal client implements this trait; adding a client means
/// adding one implementation and one arm in [`super::clients::spec_for`].
pub trait ClientSpec: Sync {
    fn client(&self) -> Client;

    /// Files this client owns, main configuration first.
    fn files(&self, ctx: &ClientCtx) -> Vec<(PathBuf, DocFormat)>;

    fn render_proxy(
        &self,
        docs: &mut [Doc],
        profile: &ProxyProfile,
        ctx: &ClientCtx,
    ) -> CoreResult<Vec<String>>;

    fn render_direct(
        &self,
        docs: &mut [Doc],
        profile: &DirectProfile,
        ctx: &ClientCtx,
    ) -> CoreResult<Vec<String>>;

    /// Strip yi-token owned entries. Each flag reports whether that file actually changed;
    /// unchanged files are left byte-for-byte so a second disconnect is a no-op.
    fn render_clear(&self, docs: &mut [Doc], ctx: &ClientCtx) -> CoreResult<Vec<bool>>;
}

/// The proxy endpoint for one client and protocol. Anthropic clients receive
/// the bare client root because their SDK appends `/v1/messages`; OpenCode's
/// AI SDK only appends `/messages`, so it receives the `/v1` base.
pub fn proxy_endpoint(client: Client, protocol: Protocol, proxy_base: &str) -> String {
    let suffix = if protocol == Protocol::Anthropic && client != Client::Opencode {
        ""
    } else {
        "/v1"
    };
    format!(
        "{}/clients/{}{suffix}",
        proxy_base.trim_end_matches('/'),
        client.as_str()
    )
}

pub fn build_plan(
    spec: &dyn ClientSpec,
    mode: TerminalMode<'_>,
    ctx: &ClientCtx,
) -> CoreResult<TerminalPlan> {
    let client = spec.client();
    if mode.client() != client {
        return Err(CoreError::Internal("终端配置与客户端不匹配".into()));
    }
    let files = spec.files(ctx);
    let Some((config_path, _)) = files.first() else {
        return Err(CoreError::Internal("终端配置未声明任何文件".into()));
    };
    let sources = files
        .iter()
        .map(|(path, _)| read_optional(path))
        .collect::<CoreResult<Vec<_>>>()?;
    let mut docs = files
        .iter()
        .zip(&sources)
        .map(|((path, format), source)| match source {
            Some(source) if !source.trim().is_empty() => Doc::parse_at(path, *format, source),
            _ => Ok(Doc::empty(*format)),
        })
        .collect::<CoreResult<Vec<_>>>()?;

    let (secrets, changed_flags) = match mode {
        TerminalMode::Proxy(profile) => (spec.render_proxy(&mut docs, profile, ctx)?, None),
        TerminalMode::Direct(profile) => (spec.render_direct(&mut docs, profile, ctx)?, None),
        TerminalMode::Clear(_) => {
            let flags = spec.render_clear(&mut docs, ctx)?;
            if flags.len() != docs.len() {
                return Err(CoreError::Internal("断开连接时的文件标记数量不正确".into()));
            }
            (Vec::new(), Some(flags))
        }
    };

    let mut changes = Vec::new();
    let mut rendered = Vec::new();
    for (index, (((path, format), source), doc)) in files.iter().zip(sources).zip(&docs).enumerate()
    {
        let after = doc.render(*format);
        let untouched = changed_flags
            .as_ref()
            .is_some_and(|flags| flags.get(index).copied() != Some(true));
        if !(untouched || (source.is_none() && after.trim() == "{}")) {
            rendered.push((path.display().to_string(), after.clone()));
        }
        if untouched || source.as_deref() == Some(after.as_str()) {
            continue;
        }
        if source.is_none() && after.trim() == "{}" {
            continue;
        }
        changes.push(super::plan::FileChange {
            path: path.clone(),
            format: *format,
            before: source,
            after: Some(after),
            secrets: secrets.clone(),
        });
    }

    Ok(TerminalPlan {
        client,
        config_path: config_path.clone(),
        endpoint: endpoint(spec, mode, ctx)?,
        changes,
        rendered,
        secrets,
    })
}

fn endpoint(spec: &dyn ClientSpec, mode: TerminalMode<'_>, ctx: &ClientCtx) -> CoreResult<String> {
    match mode {
        TerminalMode::Proxy(profile) => Ok(proxy_endpoint(
            spec.client(),
            profile.protocol,
            &ctx.proxy_base,
        )),
        TerminalMode::Direct(profile) => ctx
            .providers
            .iter()
            .find(|provider| provider.id == profile.provider_id)
            .map(|provider| provider.base_url.clone())
            .ok_or_else(|| CoreError::NotFound("直连 Provider 不存在，请重新选择".into())),
        TerminalMode::Clear(_) => Ok(String::new()),
    }
}

pub(super) fn read_optional(path: &Path) -> CoreResult<Option<String>> {
    match fs::read_to_string(path) {
        Ok(source) => Ok(Some(source)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CoreError::Io(format!(
            "读取 {} 失败：{error}",
            path.display()
        ))),
    }
}
