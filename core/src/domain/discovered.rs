use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One entry of a provider's models API, with whatever metadata the provider publishes.
///
/// Providers differ: OpenAI returns bare ids, Anthropic adds `display_name`, Volcengine
/// adds `name` (the id without its date), `status`, `domain`, `task_type`, `token_limits`
/// and `modalities`, OpenRouter adds `context_length` and `architecture`. Missing fields stay empty.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/generated/")]
pub struct DiscoveredModel {
    pub id: String,
    /// Family name without the snapshot date, when the provider gives one.
    pub name: Option<String>,
    pub display_name: Option<String>,
    /// Lifecycle as published, e.g. `Shutdown`, `Retiring`.
    pub status: Option<String>,
    /// Model category, e.g. `LLM`, `VLM`, `Embedding`, `ImageGeneration`.
    pub domain: Option<String>,
    pub task_types: Vec<String>,
    #[ts(type = "number | null")]
    pub context_window: Option<u64>,
    #[ts(type = "number | null")]
    pub max_output_tokens: Option<u64>,
    pub input_modalities: Vec<String>,
    pub output_modalities: Vec<String>,
    pub tools: Option<bool>,
    /// Unix seconds.
    #[ts(type = "number | null")]
    pub created: Option<i64>,
}

fn text(item: &Value, key: &str) -> Option<String> {
    item.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn texts(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default()
}

fn number(value: Option<&Value>) -> Option<u64> {
    value.and_then(Value::as_u64).filter(|n| *n > 0)
}

impl DiscoveredModel {
    pub fn parse(item: &Value) -> Option<Self> {
        if let Some(id) = item.as_str() {
            let id = id.trim();
            return (!id.is_empty()).then(|| Self {
                id: id.to_owned(),
                ..Self::default()
            });
        }
        let id = text(item, "id")?;
        let limits = item.get("token_limits");
        let modalities = item.get("modalities").or_else(|| item.get("architecture"));
        let task_types = match item.get("task_type") {
            Some(Value::String(one)) => vec![one.clone()],
            other => texts(other),
        };
        Some(Self {
            name: text(item, "name").filter(|name| name != &id),
            display_name: text(item, "display_name"),
            status: text(item, "status"),
            domain: text(item, "domain"),
            task_types,
            context_window: number(limits.and_then(|l| l.get("context_window")))
                .or_else(|| number(item.get("context_length")))
                .or_else(|| number(item.get("context_window"))),
            max_output_tokens: number(limits.and_then(|l| l.get("max_output_token_length")))
                .or_else(|| {
                    number(
                        item.get("top_provider")
                            .and_then(|p| p.get("max_completion_tokens")),
                    )
                })
                .or_else(|| number(item.get("max_output_tokens"))),
            input_modalities: texts(modalities.and_then(|m| m.get("input_modalities"))),
            output_modalities: texts(modalities.and_then(|m| m.get("output_modalities"))),
            tools: item
                .get("features")
                .and_then(|f| f.get("tools"))
                .and_then(|t| t.get("function_calling"))
                .and_then(Value::as_bool),
            created: item.get("created").and_then(Value::as_i64),
            id,
        })
    }

    /// Reads `{data:[...]}` or `{models:[...]}`; duplicates keep their first entry.
    pub fn parse_list(body: &Value) -> Vec<Self> {
        let list = body
            .get("data")
            .or_else(|| body.get("models"))
            .and_then(Value::as_array);
        let mut out: Vec<Self> = Vec::new();
        for item in list.into_iter().flatten() {
            if let Some(model) = Self::parse(item)
                && !out.iter().any(|existing| existing.id == model.id)
            {
                out.push(model);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_bare_and_rich_entries() {
        let body = json!({"data": ["m1", {"id": "m1"}, {
            "id": "deepseek-v4-1-flash-261017", "name": "deepseek-v4-1-flash", "status": "Retiring",
            "domain": "LLM", "task_type": ["TextGeneration"],
            "token_limits": {"context_window": 1048576, "max_output_token_length": 393216},
            "modalities": {"input_modalities": ["text"], "output_modalities": ["text"]},
            "features": {"tools": {"function_calling": true}}, "created": 1760000000
        }]});
        let list = DiscoveredModel::parse_list(&body);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "m1");
        let rich = &list[1];
        assert_eq!(rich.name.as_deref(), Some("deepseek-v4-1-flash"));
        assert_eq!(rich.context_window, Some(1_048_576));
        assert_eq!(rich.max_output_tokens, Some(393_216));
        assert_eq!(rich.tools, Some(true));
        assert_eq!(rich.task_types, ["TextGeneration"]);
    }
}
