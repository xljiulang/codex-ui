//! OpenRouter 字段源（基础源）：提供上下文、输入模态、推理档位、描述与参数能力。
//!
//! 数据来自启动时刷新的缓存，缺失或损坏时回退内置 `openrouter-models.json`。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use super::{FactMatch, FactSource, write_atomic};
use crate::codex::model_catalog::facts::ModelFacts;
use crate::codex::model_catalog::matching::{Candidate, CandidateStore};

const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";
const BUNDLED_OPENROUTER_MODELS: &str = include_str!("../../../../resources/openrouter-models.json");

pub struct OpenRouterSource {
    store: CandidateStore,
}

impl OpenRouterSource {
    pub fn load(app_dir: &Path) -> Self {
        let cached = std::fs::read_to_string(cache_path(app_dir)).ok();
        let models = cached
            .as_deref()
            .and_then(|text| parse_models(text).ok())
            .or_else(|| parse_models(BUNDLED_OPENROUTER_MODELS).ok())
            .unwrap_or_default();
        Self {
            store: build_store(models),
        }
    }
}

impl FactSource for OpenRouterSource {
    fn id(&self) -> &'static str {
        "openrouter"
    }

    fn extract(&self, model_id: &str) -> Option<FactMatch> {
        let matched = self.store.lookup(model_id)?;
        Some(FactMatch {
            facts: facts_from_model(matched.value),
            matched_id: matched.matched_id.to_string(),
            kind: matched.kind,
            score: matched.score,
        })
    }
}

/// 解析 OpenRouter 模型列表：兼容 `data[]` 与 `models[]` 两种外壳。
fn parse_models(text: &str) -> Result<Vec<Value>, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| format!("OpenRouter 模型数据不是合法 JSON: {e}"))?;
    let mut models = Vec::new();
    for key in ["data", "models"] {
        let Some(items) = value.get(key).and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            let has_id = item
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|id| !id.is_empty());
            if has_id {
                models.push(item.clone());
            }
        }
    }
    if models.is_empty() {
        return Err("OpenRouter 模型数据为空或缺少 id".to_string());
    }
    Ok(models)
}

fn build_store(models: Vec<Value>) -> CandidateStore {
    CandidateStore::new(
        models
            .into_iter()
            .filter_map(|model| {
                let id = model
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|id| !id.is_empty())?
                    .to_string();
                let canonical_slug = model
                    .get("canonical_slug")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|slug| !slug.is_empty())
                    .map(str::to_string);
                let mut aliases = Vec::new();
                if let Some(canonical_slug) = canonical_slug {
                    aliases.push(canonical_slug);
                }
                if let Some(alias_target) = model
                    .pointer("/alias_target/slug")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|slug| !slug.is_empty())
                {
                    aliases.push(alias_target.to_string());
                }
                let release = model.get("created").and_then(Value::as_i64);
                let status = model.get("status").and_then(Value::as_str);
                Some((
                    Candidate::new(id.clone(), aliases, release, id).with_status(status),
                    model,
                ))
            })
            .collect(),
    )
}

fn facts_from_model(model: &Value) -> ModelFacts {
    let mut facts = ModelFacts::default();
    let context_window = model
        .get("context_length")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0);
    facts.context_window = context_window;
    facts.max_context_window = context_window;
    facts.input_modalities = input_modalities(model);
    facts.reasoning_levels = reasoning_levels(model);
    facts.default_reasoning_level = model
        .pointer("/reasoning/default_effort")
        .and_then(Value::as_str)
        .map(|effort| effort.trim().to_ascii_lowercase())
        .filter(|effort| !effort.is_empty());
    facts.supports_reasoning = reasoning_present(model);
    facts.status = model.get("status").and_then(Value::as_str)
        .map(|status| status.trim().to_ascii_lowercase()).filter(|status| !status.is_empty());
    facts.description = model
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .map(str::to_string);
    // `supported_parameters` 是 OpenRouter 的显式能力声明：出现即视为“源有信息”，
    // 缺失该参数即明确不支持。
    if let Some(parameters) = supported_parameters(model) {
        facts.support_verbosity = Some(parameters.contains("verbosity"));
        facts.supports_search_tool = Some(
            parameters.contains("web_search_options") || parameters.contains("web_search"),
        );
        facts.supports_tool_calls = Some(parameters.contains("tools"));
    }
    facts
}

fn input_modalities(model: &Value) -> Option<Vec<String>> {
    let items = model
        .pointer("/architecture/input_modalities")
        .and_then(Value::as_array)?;
    if !items.iter().all(Value::is_string) { return None; }
    let mut modalities = Vec::new();
    for item in items {
        let Some(modality) = item.as_str().map(|value| value.trim().to_ascii_lowercase()) else {
            continue;
        };
        if matches!(modality.as_str(), "text" | "image" | "audio")
            && !modalities.iter().any(|existing| existing == &modality)
        {
            modalities.push(modality);
        }
    }
    // 注意：空数组是「显式声明没有可用输入模态」，不是「未声明」——与推理档位不同，
    // `template::render` 有专门用例保证它不会继承模板的 `["text"]`。
    Some(modalities)
}

fn reasoning_levels(model: &Value) -> Option<Vec<String>> {
    let items = model
        .pointer("/reasoning/supported_efforts")
        .and_then(Value::as_array)?;
    if !items.iter().all(Value::is_string) { return None; }
    let mut levels: Vec<String> = items
        .iter()
        .filter_map(Value::as_str)
        .map(|effort| effort.trim().to_ascii_lowercase())
        .filter(|effort| !effort.is_empty())
        .collect();
    let mut seen = HashSet::new();
    levels.retain(|level| seen.insert(level.clone()));
    // 空数组：条目声明了推理却没有档位（toggle 型）时视为"未知"，交给其它来源补；
    // 未声明推理时才当作"明确无档位"——与 models.dev 的规则保持一致。
    if levels.is_empty() && reasoning_present(model) == Some(true) {
        return None;
    }
    Some(levels)
}

fn reasoning_present(model: &Value) -> Option<bool> {
    let reasoning = model.get("reasoning")?.as_object()?;
    let mandatory = reasoning
        .get("mandatory")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let has_efforts = reasoning
        .get("supported_efforts")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty());
    let has_default = reasoning
        .get("default_effort")
        .and_then(Value::as_str)
        .is_some_and(|effort| !effort.is_empty());
    Some(mandatory || has_efforts || has_default)
}

fn supported_parameters(model: &Value) -> Option<HashSet<String>> {
    let items = model.get("supported_parameters").and_then(Value::as_array)?;
    if !items.iter().all(Value::is_string) { return None; }
    Some(
        items
            .iter()
            .filter_map(Value::as_str)
            .map(|value| value.trim().to_ascii_lowercase())
            .collect(),
    )
}

fn cache_path(app_dir: &Path) -> PathBuf {
    app_dir.join("cache").join("openrouter-models.json")
}

/// 是否需要重新下载：缓存缺失、超过 24 小时或内容不可解析（损坏视为过期）。
fn needs_refresh(app_dir: &Path) -> bool {
    !super::cache_is_reusable(&cache_path(app_dir), |text| parse_models(text).is_ok())
}

/// 启动时后台刷新缓存（24 小时内跳过）；失败静默保留旧缓存/内置资源。
pub async fn refresh_cache(app_dir: &Path) {
    if !needs_refresh(app_dir) {
        return;
    }
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
    {
        Ok(client) => client,
        Err(_) => return,
    };
    let response = match client
        .get(OPENROUTER_MODELS_URL)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => return,
    };
    if !response.status().is_success() {
        return;
    }
    let text = match response.text().await {
        Ok(text) => text,
        Err(_) => return,
    };
    let _ = store_response(app_dir, &text);
}

fn store_response(app_dir: &Path, text: &str) -> Result<(), String> {
    parse_models(text)?;
    write_atomic(&cache_path(app_dir), text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn model(id: &str) -> Value {
        json!({
            "id": id,
            "canonical_slug": format!("{id}-20260101"),
            "created": 1_700_000_000,
            "description": "描述",
            "context_length": 128_000,
            "architecture": { "input_modalities": ["text", "image", "video"] },
            "reasoning": { "supported_efforts": ["low", "HIGH"], "default_effort": "high" },
            "supported_parameters": ["verbosity", "temperature"]
        })
    }

    #[test]
    fn empty_efforts_with_reasoning_declared_is_unknown() {
        // 声明了推理却没有档位（toggle 型）→ 未知，交给其它来源补，
        // 避免以更高的来源质量清掉别的来源给出的档位（与 models.dev 规则一致）
        let mut entry = model("relay/demo");
        entry["reasoning"] = json!({ "mandatory": true, "supported_efforts": [] });
        assert_eq!(facts_from_model(&entry).reasoning_levels, None);
        // 未声明推理的空数组 → 保持"明确无档位"
        let mut entry = model("relay/demo");
        entry["reasoning"] = json!({ "mandatory": false, "supported_efforts": [] });
        assert_eq!(facts_from_model(&entry).reasoning_levels, Some(Vec::new()));
    }

    #[test]
    fn parse_models_accepts_openai_and_codex_shapes() {
        let value = json!({
            "data": [{"id": "a"}, {"id": ""}],
            "models": [{"id": "b"}]
        });
        let models = parse_models(&value.to_string()).unwrap();
        assert_eq!(models.len(), 2);
        assert!(parse_models("{\"data\":[]}").is_err());
    }

    #[test]
    fn extract_maps_openrouter_fields() {
        let source = OpenRouterSource {
            store: build_store(vec![model("vendor/model-x")]),
        };
        let facts = source.extract("vendor/model-x").unwrap().facts;
        assert_eq!(facts.context_window, Some(128_000));
        assert_eq!(facts.max_context_window, Some(128_000));
        assert_eq!(facts.input_modalities, Some(vec!["text".into(), "image".into()]));
        assert_eq!(facts.reasoning_levels, Some(vec!["low".into(), "high".into()]));
        assert_eq!(facts.default_reasoning_level.as_deref(), Some("high"));
        assert_eq!(facts.supports_reasoning, Some(true));
        assert_eq!(facts.description.as_deref(), Some("描述"));
        assert_eq!(facts.support_verbosity, Some(true));
        assert_eq!(facts.supports_search_tool, Some(false));
        assert_eq!(facts.supports_tool_calls, Some(false));
    }

    #[test]
    fn extracts_status_and_explicit_empty_efforts() {
        let facts = facts_from_model(&json!({
            "id": "model", "status": " DEPRECATED ",
            "reasoning": { "supported_efforts": [] }
        }));
        assert_eq!(facts.status.as_deref(), Some("deprecated"));
        assert_eq!(facts.reasoning_levels, Some(vec![]));
        assert_eq!(facts_from_model(&json!({"id":"model"})).reasoning_levels, None);
        let invalid = facts_from_model(&json!({
            "supported_parameters":[1], "reasoning":{"supported_efforts":[1]},
            "architecture":{"input_modalities":[1]}
        }));
        assert_eq!(invalid.supports_tool_calls, None);
        assert_eq!(invalid.reasoning_levels, None);
        assert_eq!(invalid.input_modalities, None);
        let empty = facts_from_model(&json!({"architecture":{"input_modalities":[]}}));
        assert_eq!(empty.input_modalities, Some(vec![]));
    }

    #[test]
    fn extract_is_silent_about_verbosity_without_parameter_list() {
        let source = OpenRouterSource {
            store: build_store(vec![json!({ "id": "vendor/model-y" })]),
        };
        let facts = source.extract("vendor/model-y").unwrap().facts;
        assert_eq!(facts.support_verbosity, None);
        assert_eq!(facts.supports_search_tool, None);
        assert_eq!(facts.supports_tool_calls, None);
        assert_eq!(facts.context_window, None);
        assert_eq!(facts.supports_reasoning, None);
    }

    #[test]
    fn load_falls_back_to_bundled_resource_without_cache() {
        let dir = tempdir().unwrap();
        let source = OpenRouterSource::load(dir.path());
        assert!(source.store.len() > 100);
        assert!(source.extract("deepseek/deepseek-v4-flash").is_some());
        let qwen = source.extract("qwen4-coder").unwrap();
        assert_eq!(qwen.matched_id, "qwen/qwen3-coder");
        assert_eq!(
            qwen.kind,
            crate::codex::model_catalog::matching::MatchKind::Fuzzy
        );
    }

    #[test]
    fn load_ignores_broken_cache_file() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("cache")).unwrap();
        std::fs::write(cache_path(dir.path()), "{ not json").unwrap();
        let source = OpenRouterSource::load(dir.path());
        assert!(source.store.len() > 100);
    }

    #[test]
    fn cache_write_rejects_invalid_json_without_touching_existing_file() {
        let dir = tempdir().unwrap();
        let valid = r#"{"data":[{"id":"a"}]}"#;
        store_response(dir.path(), valid).unwrap();
        assert_eq!(
            std::fs::read_to_string(cache_path(dir.path())).unwrap(),
            valid
        );
        assert!(store_response(dir.path(), r#"{"data":[]}"#).is_err());
        assert_eq!(
            std::fs::read_to_string(cache_path(dir.path())).unwrap(),
            valid
        );
    }

    #[test]
    fn needs_refresh_follows_ttl_and_content_validity() {
        let dir = tempdir().unwrap();
        let path = cache_path(dir.path());
        let valid = r#"{"data":[{"id":"vendor/model-x"}]}"#;

        // 没有缓存文件 → 需要下载
        assert!(needs_refresh(dir.path()));

        // 新鲜且合法 → 24 小时内不重复下载
        super::super::test_support::write_cache(&path, valid, Duration::from_secs(60 * 60));
        assert!(!needs_refresh(dir.path()));

        // 新鲜但损坏 → 视为过期，需要重新下载
        super::super::test_support::write_cache(&path, "{ broken", Duration::from_secs(60 * 60));
        assert!(needs_refresh(dir.path()));

        // 超过 24 小时 → 需要重新下载
        super::super::test_support::write_cache(&path, valid, Duration::from_secs(25 * 60 * 60));
        assert!(needs_refresh(dir.path()));
    }
}
