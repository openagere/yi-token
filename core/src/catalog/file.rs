//! Catalog file format and loading.

use crate::domain::{Capabilities, Protocol, StandardModel};
use crate::error::{CoreError, CoreResult};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::Path;

const SUPPORTED_VERSION: u32 = 1;
const BUILTIN: &str = include_str!("../../../models/catalog.json");

/// A parsed catalog. Entry ids come from the file and are not authoritative:
/// `sync::apply` keeps the stored id for known `(name, protocol)` pairs.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogFile {
    pub version: String,
    pub entries: Vec<StandardModel>,
}

#[derive(Deserialize)]
struct RawFile {
    version: u32,
    models: Vec<RawEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntry {
    #[serde(default)]
    id: String,
    name: String,
    protocol: Protocol,
    #[serde(default)]
    brand: String,
    #[serde(default)]
    capabilities: Capabilities,
}

fn parse(text: &str) -> CoreResult<CatalogFile> {
    let raw: RawFile = serde_json::from_str(text)
        .map_err(|e| CoreError::Validation(format!("模型目录 JSON 无效：{e}")))?;
    if raw.version != SUPPORTED_VERSION {
        return Err(CoreError::Validation(format!(
            "不支持模型目录版本 {}，当前支持版本 {SUPPORTED_VERSION}",
            raw.version
        )));
    }
    let mut seen = HashSet::new();
    let mut entries = Vec::with_capacity(raw.models.len());
    for m in raw.models {
        let name = m.name.trim().to_owned();
        if name.is_empty() {
            return Err(CoreError::Validation("标准模型名称不能为空".into()));
        }
        m.capabilities
            .validate(m.protocol)
            .map_err(|e| CoreError::Validation(format!("模型 {name}：{e}")))?;
        if !seen.insert((name.clone(), m.protocol)) {
            return Err(CoreError::Validation(format!(
                "同一协议下的标准模型名称重复：{name}"
            )));
        }
        entries.push(StandardModel {
            id: m.id,
            name,
            protocol: m.protocol,
            brand: m.brand,
            capabilities: m.capabilities,
        });
    }
    Ok(CatalogFile {
        version: raw.version.to_string(),
        entries,
    })
}

/// Loads the catalog at `path`, or the built-in one when `path` is `None`.
pub fn load(path: Option<&Path>) -> CoreResult<CatalogFile> {
    match path {
        None => parse(BUILTIN),
        Some(p) => {
            let text = std::fs::read_to_string(p)
                .map_err(|e| CoreError::Io(format!("读取模型目录 {} 失败：{e}", p.display())))?;
            parse(&text)
        }
    }
}
