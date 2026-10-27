//! Format-preserving configuration documents for terminal client files.
//!
//! TOML is edited through `toml_edit` so comments and key order survive. JSON,
//! JSONC and YAML are edited as values; comments in those formats are not
//! preserved (the same behavior as yi-llm).

use crate::error::{CoreError, CoreResult};
use serde_json::{Map, Value, json};
use std::path::Path;
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, Value as TomlValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocFormat {
    Toml,
    Json,
    Jsonc,
    Yaml,
}

impl DocFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Toml => "toml",
            Self::Json => "json",
            Self::Jsonc => "jsonc",
            Self::Yaml => "yaml",
        }
    }
}

#[derive(Debug, Clone)]
pub enum Doc {
    Toml(DocumentMut),
    Json(Value),
    Yaml(Value),
}

impl Doc {
    pub fn parse(format: DocFormat, text: &str) -> CoreResult<Self> {
        Self::parse_inner(format, text)
    }

    /// Parses `text` and includes `path` in every validation error.
    pub fn parse_at(path: &Path, format: DocFormat, text: &str) -> CoreResult<Self> {
        Self::parse_inner(format, text).map_err(|error| {
            CoreError::Validation(format!(
                "无法解析 {}：{error}，未做任何修改",
                path.display()
            ))
        })
    }

    fn parse_inner(format: DocFormat, text: &str) -> CoreResult<Self> {
        match format {
            DocFormat::Toml => text
                .parse::<DocumentMut>()
                .map(Self::Toml)
                .map_err(|error| CoreError::Validation(error.to_string())),
            DocFormat::Json => {
                let value: Value = serde_json::from_str(text)
                    .map_err(|error| CoreError::Validation(error.to_string()))?;
                require_object(value).map(Self::Json)
            }
            DocFormat::Jsonc => {
                let value: Value = json5::from_str(text)
                    .map_err(|error| CoreError::Validation(error.to_string()))?;
                require_object(value).map(Self::Json)
            }
            DocFormat::Yaml => {
                let value: Value = serde_norway::from_str(text)
                    .map_err(|error| CoreError::Validation(error.to_string()))?;
                Ok(Self::Yaml(value))
            }
        }
    }

    pub fn empty(format: DocFormat) -> Self {
        match format {
            DocFormat::Toml => Self::Toml(DocumentMut::new()),
            DocFormat::Json | DocFormat::Jsonc => Self::Json(json!({})),
            DocFormat::Yaml => Self::Yaml(Value::Null),
        }
    }

    pub fn render(&self, format: DocFormat) -> String {
        let mut text = match (self, format) {
            (Self::Toml(document), DocFormat::Toml) => document.to_string(),
            (Self::Json(value), DocFormat::Json | DocFormat::Jsonc) => {
                serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into())
            }
            (Self::Yaml(value), DocFormat::Yaml) => {
                serde_norway::to_string(value).unwrap_or_else(|_| "{}\n".into())
            }
            (doc, _) => match doc {
                Self::Toml(document) => document.to_string(),
                Self::Json(value) | Self::Yaml(value) => {
                    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into())
                }
            },
        };
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text
    }

    pub fn get(&self, path: &[&str]) -> Option<Value> {
        match self {
            Self::Toml(document) => {
                let mut table = document.as_table();
                let (last, parents) = path.split_last()?;
                for key in parents {
                    table = item_table(table.get(key)?)?;
                }
                item_to_json(table.get(last)?)
            }
            Self::Json(value) | Self::Yaml(value) => json_get(value, path).cloned(),
        }
    }

    /// Sets `path` to `value`. A `null` value removes the key.
    pub fn set(&mut self, path: &[&str], value: Value) -> CoreResult<()> {
        if path.is_empty() {
            return Err(CoreError::Validation("配置路径不能为空".into()));
        }
        if value.is_null() {
            return self.remove(path);
        }
        match self {
            Self::Toml(document) => {
                let mut table = document.as_table_mut();
                let (last, parents) = path.split_last().expect("path is not empty");
                for key in parents {
                    if !table.contains_key(key) {
                        table.insert(key, Item::Table(Table::new()));
                    }
                    table = table
                        .get_mut(key)
                        .and_then(Item::as_table_mut)
                        .ok_or_else(|| {
                            CoreError::Validation(format!("配置 {key} 不是 TOML 表，未做任何修改"))
                        })?;
                }
                let item = json_to_item(value)?;
                table.insert(last, item);
                Ok(())
            }
            Self::Json(root) | Self::Yaml(root) => json_set(root, path, value),
        }
    }

    pub fn remove(&mut self, path: &[&str]) -> CoreResult<()> {
        if path.is_empty() {
            return Err(CoreError::Validation("配置路径不能为空".into()));
        }
        match self {
            Self::Toml(document) => {
                let mut table = document.as_table_mut();
                let (last, parents) = path.split_last().expect("path is not empty");
                for key in parents {
                    let Some(next) = table.get_mut(key).and_then(Item::as_table_mut) else {
                        return Ok(());
                    };
                    table = next;
                }
                table.remove(last);
                Ok(())
            }
            Self::Json(root) | Self::Yaml(root) => {
                let (last, parents) = path.split_last().expect("path is not empty");
                let Some(map) = json_object_at_mut(root, parents) else {
                    return Ok(());
                };
                map.remove(*last);
                Ok(())
            }
        }
    }

    /// Retains only entries at `path` for which `keep(key, value)` is true.
    /// Missing paths and non-object values are left untouched.
    pub fn retain_in(
        &mut self,
        path: &[&str],
        keep: impl Fn(&str, &Value) -> bool,
    ) -> CoreResult<()> {
        if path.is_empty() {
            return Err(CoreError::Validation("配置路径不能为空".into()));
        }
        match self {
            Self::Toml(document) => {
                let mut table = document.as_table_mut();
                for key in path {
                    let Some(next) = table.get_mut(key).and_then(Item::as_table_mut) else {
                        return Ok(());
                    };
                    table = next;
                }
                let keys: Vec<String> = table.iter().map(|(key, _)| key.to_owned()).collect();
                for key in keys {
                    let remove = table
                        .get(&key)
                        .and_then(item_to_json)
                        .is_some_and(|value| !keep(&key, &value));
                    if remove {
                        table.remove(&key);
                    }
                }
                Ok(())
            }
            Self::Json(root) | Self::Yaml(root) => {
                let Some(map) = json_object_at_mut(root, path) else {
                    return Ok(());
                };
                map.retain(|key, value| keep(key, value));
                Ok(())
            }
        }
    }
}

fn require_object(value: Value) -> CoreResult<Value> {
    if value.is_object() {
        Ok(value)
    } else {
        Err(CoreError::Validation("终端配置必须是 JSON 对象".into()))
    }
}

fn json_get<'a>(mut value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    for key in path {
        value = value.as_object()?.get(*key)?;
    }
    Some(value)
}

fn json_object_at_mut<'a>(
    mut value: &'a mut Value,
    path: &[&str],
) -> Option<&'a mut Map<String, Value>> {
    for key in path {
        value = value.as_object_mut()?.get_mut(*key)?;
    }
    value.as_object_mut()
}

fn json_set(root: &mut Value, path: &[&str], value: Value) -> CoreResult<()> {
    let (last, parents) = path.split_last().expect("path is not empty");
    let mut current = root;
    for key in parents {
        let map = current
            .as_object_mut()
            .ok_or_else(|| CoreError::Validation("配置结构不是对象，未做任何修改".into()))?;
        current = map.entry((*key).to_owned()).or_insert_with(|| json!({}));
    }
    let map = current
        .as_object_mut()
        .ok_or_else(|| CoreError::Validation("配置结构不是对象，未做任何修改".into()))?;
    map.insert((*last).to_owned(), value);
    Ok(())
}

fn item_table(item: &Item) -> Option<&Table> {
    match item {
        Item::Table(table) => Some(table),
        _ => None,
    }
}

fn item_to_json(item: &Item) -> Option<Value> {
    match item {
        Item::None => None,
        Item::Value(value) => Some(value_to_json(value)),
        Item::Table(table) => Some(table_to_json(table)),
        Item::ArrayOfTables(array) => Some(Value::Array(
            array.iter().map(table_to_json).collect::<Vec<_>>(),
        )),
    }
}

fn table_to_json(table: &Table) -> Value {
    Value::Object(
        table
            .iter()
            .filter_map(|(key, item)| Some((key.to_owned(), item_to_json(item)?)))
            .collect(),
    )
}

fn value_to_json(value: &TomlValue) -> Value {
    match value {
        TomlValue::String(value) => Value::String(value.value().clone()),
        TomlValue::Integer(value) => Value::Number((*value.value()).into()),
        TomlValue::Float(value) => serde_json::Number::from_f64(*value.value())
            .map(Value::Number)
            .unwrap_or(Value::Null),
        TomlValue::Boolean(value) => Value::Bool(*value.value()),
        TomlValue::Datetime(value) => Value::String(value.to_string()),
        TomlValue::Array(array) => Value::Array(array.iter().map(value_to_json).collect()),
        TomlValue::InlineTable(table) => Value::Object(
            table
                .iter()
                .map(|(key, value)| (key.to_owned(), value_to_json(value)))
                .collect(),
        ),
    }
}

fn json_to_item(value: Value) -> CoreResult<Item> {
    Ok(match value {
        Value::Null => Item::None,
        Value::Bool(value) => Item::Value(TomlValue::from(value)),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                Item::Value(TomlValue::from(value))
            } else if let Some(value) = number.as_f64() {
                Item::Value(TomlValue::from(value))
            } else {
                return Err(CoreError::Validation("数值超出 TOML 范围".into()));
            }
        }
        Value::String(value) => Item::Value(TomlValue::from(value)),
        Value::Array(values) => {
            if values.iter().all(Value::is_object) {
                let mut array = ArrayOfTables::new();
                for value in values {
                    let Value::Object(map) = value else {
                        unreachable!("checked above")
                    };
                    array.push(json_object_to_table(map));
                }
                Item::ArrayOfTables(array)
            } else {
                let mut array = Array::new();
                for value in values {
                    if value.is_array() || value.is_object() {
                        return Err(CoreError::Validation(
                            "TOML 数组不能混入嵌套数组或对象".into(),
                        ));
                    }
                    array.push(json_to_value(value)?);
                }
                Item::Value(TomlValue::Array(array))
            }
        }
        Value::Object(map) => Item::Table(json_object_to_table(map)),
    })
}

fn json_to_value(value: Value) -> CoreResult<TomlValue> {
    match json_to_item(value)? {
        Item::Value(value) => Ok(value),
        _ => Err(CoreError::Validation(
            "TOML 数组不能混入嵌套数组或对象".into(),
        )),
    }
}

fn json_object_to_table(values: Map<String, Value>) -> Table {
    let mut table = Table::new();
    for (key, value) in values {
        if let Ok(item) = json_to_item(value) {
            table.insert(&key, item);
        }
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_set_preserves_comments_and_order() {
        let source = "# top\nfirst = 1 # keep me\n[model_providers.old]\nname = \"old\"\n";
        let mut doc = Doc::parse(DocFormat::Toml, source).unwrap();
        doc.set(
            &["model_providers", "yitoken"],
            json!({"name": "yi-token", "base_url": "http://127.0.0.1:11435"}),
        )
        .unwrap();
        let text = doc.render(DocFormat::Toml);
        assert!(text.starts_with("# top\nfirst = 1 # keep me\n"));
        assert!(text.contains("[model_providers.old]"));
        assert!(text.contains("[model_providers.yitoken]"));
        assert_eq!(
            doc.get(&["model_providers", "yitoken", "name"]).unwrap(),
            json!("yi-token")
        );
    }
}
