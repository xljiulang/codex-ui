//! 模型目录生成：完整条目源优先，其次多字段源按注册顺序合并，最后套模板。
//!
//! 管线（见 `sources` 模块）：
//! 1. 依次询问完整条目源（`official`）：命中即整条复用，解决自定义目录里
//!    GPT 类模型不如 codex 自带条目、或厂商条目与公开数据源不一致的问题；
//! 2. 否则逐字段源提取（`openrouter` → `models_dev`，后者覆盖前者）；
//! 3. 合并结果套 [`template`] 渲染成目录条目。
//!
//! 数据源来自提供方 `/models` 的模型 ID 列表，元数据由上述源补全；新增数据源只需
//! 在 `sources::fact_sources` 里追加一行。

mod codex_models;
mod facts;
mod matching;
mod sources;
mod template;

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use facts::ModelFacts;
use sources::{FactMatch, FactSource, FullEntryMatch, FullEntrySource};

/// 启动时后台刷新各字段源的运行时缓存；失败静默。
pub use sources::refresh_caches as refresh_source_caches;
/// 启动时后台导出本机 codex 自带的官方条目；失败保留旧缓存。
pub use codex_models::refresh_codex_models;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelCatalogModelStatus {
    Ready,
    Incompatible,
    Unmatched,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ModelCatalogSourceEvidence {
    pub source: String,
    pub matched_id: String,
    pub match_kind: String,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ModelCatalogModelOption {
    pub id: String,
    pub display_name: String,
    pub status: ModelCatalogModelStatus,
    pub sources: Vec<ModelCatalogSourceEvidence>,
    pub warnings: Vec<String>,
    pub selectable: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ModelCatalogGenerateResult {
    pub catalog: String,
    pub total: usize,
    pub ready: usize,
    pub incompatible: usize,
    pub unmatched: usize,
    pub models: Vec<ModelCatalogModelOption>,
}

/// 从模型提供方的 `/models` 获取模型 ID，并用各数据源生成完整模型目录。
pub async fn generate_from_provider(
    app_dir: &Path,
    base_url: &str,
    api_key: &str,
) -> Result<ModelCatalogGenerateResult, String> {
    let base_url = base_url.trim();
    let api_key = api_key.trim();
    if base_url.is_empty() {
        return Err("模型提供者 base_url 不能为空".to_string());
    }
    if api_key.is_empty() {
        return Err("模型提供者 API Key 不能为空".to_string());
    }

    let url = models_url(base_url)?;
    let ids = fetch_model_ids(&url, api_key).await?;
    build_catalog_result(&ids, app_dir, base_url)
}

fn build_catalog_result(
    ids: &[String],
    app_dir: &Path,
    base_url: &str,
) -> Result<ModelCatalogGenerateResult, String> {
    let full_entry_sources = sources::full_entry_sources(app_dir, base_url);
    let fact_sources = sources::fact_sources(app_dir);

    let mut entries: Vec<Value> = Vec::new();
    let mut models: Vec<ModelCatalogModelOption> = Vec::with_capacity(ids.len());
    for id in ids {
        let display_name = template::display_name_from_slug(id);

        if let Some((source_id, matched)) = lookup_full_entry(&full_entry_sources, id) {
            let source_evidence = full_entry_evidence(source_id, &matched);
            let entry = finalize_full_entry(matched.entry, id, entries.len() + 1)?;
            entries.push(entry);
            models.push(ModelCatalogModelOption {
                id: id.clone(),
                display_name,
                status: ModelCatalogModelStatus::Ready,
                selectable: true,
                sources: vec![source_evidence],
                warnings: Vec::new(),
            });
            continue;
        }

        let collected = collect_facts(id, &fact_sources);
        if collected.sources.is_empty() {
            models.push(ModelCatalogModelOption {
                id: id.clone(),
                display_name,
                status: ModelCatalogModelStatus::Unmatched,
                selectable: false,
                sources: Vec::new(),
                warnings: vec!["未匹配到可用资料".to_string()],
            });
            continue;
        }

        let mut warnings = Vec::new();
        let incompatible_reason = incompatible_reason(&collected.facts);
        let status = if let Some(reason) = incompatible_reason {
            warnings.push(reason);
            ModelCatalogModelStatus::Incompatible
        } else {
            if collected
                .facts
                .status
                .as_deref()
                .is_some_and(|status| status.eq_ignore_ascii_case("beta"))
            {
                warnings.push("上游资料标记为 beta".to_string());
            }
            entries.push(template::render(id, &collected.facts, entries.len() + 1)?);
            ModelCatalogModelStatus::Ready
        };
        models.push(ModelCatalogModelOption {
            id: id.clone(),
            display_name,
            status,
            selectable: status == ModelCatalogModelStatus::Ready,
            sources: collected.sources,
            warnings,
        });
    }

    let ready = models
        .iter()
        .filter(|model| model.status == ModelCatalogModelStatus::Ready)
        .count();
    let incompatible = models
        .iter()
        .filter(|model| model.status == ModelCatalogModelStatus::Incompatible)
        .count();
    let unmatched = models
        .iter()
        .filter(|model| model.status == ModelCatalogModelStatus::Unmatched)
        .count();
    let catalog = serde_json::to_string_pretty(&json!({ "models": entries }))
        .map_err(|e| format!("生成模型目录 JSON 失败: {e}"))?;
    Ok(ModelCatalogGenerateResult {
        catalog,
        total: models.len(),
        ready,
        incompatible,
        unmatched,
        models,
    })
}

/// 完整条目源：命中即短路，不进入字段合并与模板渲染。
fn lookup_full_entry(
    sources: &[Box<dyn FullEntrySource>],
    model_id: &str,
) -> Option<(&'static str, FullEntryMatch)> {
    sources
        .iter()
        .find_map(|source| source.full_entry(model_id).map(|entry| (source.id(), entry)))
}

/// 只覆盖身份与顺序：`slug` 用提供方返回的原始 id，`priority` 按选择顺序。
fn finalize_full_entry(mut entry: Value, model_id: &str, priority: usize) -> Result<Value, String> {
    let object = entry
        .as_object_mut()
        .ok_or_else(|| "完整条目源返回的条目不是 JSON 对象".to_string())?;
    object.insert("slug".to_string(), json!(model_id));
    object.insert("priority".to_string(), json!(priority as i64));
    Ok(entry)
}

struct CollectedFacts {
    facts: ModelFacts,
    sources: Vec<ModelCatalogSourceEvidence>,
}

/// 逐字段源提取，并按来源权威性与匹配质量合并。
fn collect_facts(model_id: &str, sources: &[Box<dyn FactSource>]) -> CollectedFacts {
    let mut facts = ModelFacts::default();
    let mut evidence = Vec::new();
    for source in sources {
        if let Some(matched) = source.extract(model_id) {
            facts.merge_from(
                source.id(),
                matched.quality(source.quality_base()),
                matched.facts.clone(),
            );
            evidence.push(fact_evidence(source.id(), &matched));
        }
    }
    CollectedFacts {
        facts,
        sources: evidence,
    }
}

fn full_entry_evidence(
    source: &'static str,
    matched: &FullEntryMatch,
) -> ModelCatalogSourceEvidence {
    ModelCatalogSourceEvidence {
        source: source.to_string(),
        matched_id: matched.matched_id.clone(),
        match_kind: matched.kind.as_str().to_string(),
        score: 1.0,
    }
}

fn fact_evidence(source: &'static str, matched: &FactMatch) -> ModelCatalogSourceEvidence {
    ModelCatalogSourceEvidence {
        source: source.to_string(),
        matched_id: matched.matched_id.clone(),
        match_kind: matched.kind.as_str().to_string(),
        score: matched.score,
    }
}

fn incompatible_reason(facts: &ModelFacts) -> Option<String> {
    if facts.supports_tool_calls == Some(false) {
        return Some("上游明确标记 tool_call=false，不兼容 Codex 工具调用".to_string());
    }
    if facts
        .status
        .as_deref()
        .is_some_and(|status| status.eq_ignore_ascii_case("deprecated"))
    {
        return Some("上游资料标记为 deprecated".to_string());
    }
    None
}

fn models_url(base_url: &str) -> Result<String, String> {
    let mut url = reqwest::Url::parse(base_url.trim())
        .map_err(|e| format!("模型提供者 base_url 无效: {e}"))?;
    let path = url.path().trim_end_matches('/');
    let path = if path.ends_with("/models") {
        path.to_string()
    } else if path.is_empty() {
        "/models".to_string()
    } else {
        format!("{path}/models")
    };
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

async fn fetch_model_ids(url: &str, api_key: &str) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("创建模型提供者请求失败: {e}"))?;
    let response = client
        .get(url)
        .bearer_auth(api_key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("请求模型提供者 /models 失败: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let body = truncate(body.trim(), 200);
        return Err(if body.is_empty() {
            format!("模型提供者 /models 请求失败（HTTP {status}）")
        } else {
            format!("模型提供者 /models 请求失败（HTTP {status}）：{body}")
        });
    }
    let value = response
        .json::<Value>()
        .await
        .map_err(|e| format!("模型提供者 /models 返回不是合法 JSON: {e}"))?;
    model_ids(&value)
}

/// 兼容 `data[].id` 与 `models[].id/slug`，去重保序。
fn model_ids(value: &Value) -> Result<Vec<String>, String> {
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    for key in ["data", "models"] {
        let Some(items) = value.get(key).and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            let id = item
                .as_str()
                .or_else(|| item.get("id").and_then(Value::as_str))
                .or_else(|| item.get("slug").and_then(Value::as_str));
            let Some(id) = id else {
                continue;
            };
            let id = id.trim();
            if !id.is_empty() && seen.insert(id.to_string()) {
                ids.push(id.to_string());
            }
        }
    }
    if ids.is_empty() {
        return Err("模型提供者 /models 未返回任何模型".to_string());
    }
    Ok(ids)
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max_chars).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// 测试用第三个字段源：证明新增数据源只需实现 trait，无需改动管线代码。
    struct FakeSource;

    impl FactSource for FakeSource {
        fn id(&self) -> &'static str {
            "fake"
        }

        /// 测试用高权威源：验证「来源基础分更高者覆盖前序来源」。
        fn quality_base(&self) -> u8 {
            50
        }

        fn extract(&self, model_id: &str) -> Option<FactMatch> {
            Some(FactMatch {
                facts: ModelFacts {
                    context_window: Some(999),
                    ..ModelFacts::default()
                },
                matched_id: model_id.to_string(),
                kind: matching::MatchKind::Exact,
                score: 1.0,
            })
        }
    }

    fn has_source(model: &ModelCatalogModelOption, source: &str) -> bool {
        model.sources.iter().any(|item| item.source == source)
    }

    fn catalog_entry(result: &ModelCatalogGenerateResult, slug: &str) -> Value {
        let catalog: Value = serde_json::from_str(&result.catalog).unwrap();
        catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["slug"] == json!(slug))
            .cloned()
            .unwrap_or_else(|| panic!("目录里没有 {slug}"))
    }

    #[test]
    fn conflicting_source_values_do_not_warn() {
        let dir = tempdir().unwrap();
        // models.dev 与 OpenRouter 对该模型给出的上下文不同，但字段差异不再产生候选警告
        let result = build_catalog_result(
            &["deepseek-v4-flash-vision-exp".to_string()],
            dir.path(),
            "https://api.deepseek.com/v1",
        )
        .unwrap();
        let model = &result.models[0];
        assert_eq!(model.status, ModelCatalogModelStatus::Ready);
        assert!(has_source(model, "models_dev"));
        assert!(has_source(model, "openrouter"));
        assert!(
            model.warnings.is_empty(),
            "字段差异不应再进入候选警告：{:?}",
            model.warnings
        );
    }

    #[test]
    fn models_url_appends_models_without_duplicate_suffix() {
        assert_eq!(
            models_url("https://api.example.com/v1").unwrap(),
            "https://api.example.com/v1/models"
        );
        assert_eq!(
            models_url("https://api.example.com/v1/").unwrap(),
            "https://api.example.com/v1/models"
        );
        assert_eq!(
            models_url("https://api.example.com/v1/models").unwrap(),
            "https://api.example.com/v1/models"
        );
    }

    #[test]
    fn model_ids_accepts_openai_and_codex_shapes() {
        let value = json!({
            "data": [{"id": "a"}, {"id": "a"}, {"id": "b"}],
            "models": [{"slug": "c"}]
        });
        assert_eq!(model_ids(&value).unwrap(), vec!["a", "b", "c"]);
    }

    #[test]
    fn pipeline_merges_sources_by_quality_and_records_evidence() {
        let dir = tempdir().unwrap();
        let sources: Vec<Box<dyn FactSource>> = vec![
            Box::new(sources::openrouter::OpenRouterSource::load(dir.path())),
            Box::new(sources::models_dev::ModelsDevSource::load(dir.path())),
            Box::new(FakeSource),
        ];
        let collected = collect_facts("deepseek-flash", &sources);
        assert_eq!(
            collected
                .sources
                .iter()
                .map(|source| source.source.as_str())
                .collect::<Vec<_>>(),
            vec!["openrouter", "models_dev", "fake"]
        );
        assert_eq!(collected.facts.context_window, Some(999));
        assert_eq!(collected.facts.source_of("context_window"), Some("fake"));
        assert_eq!(
            collected.facts.source_of("input_modalities"),
            Some("models_dev"),
            "未提供该字段的源不应覆盖"
        );
        assert_eq!(
            collected.facts.source_of("support_verbosity"),
            Some("openrouter")
        );
    }

    #[test]
    fn local_proxy_base_url_still_matches_models_dev_ids() {
        let dir = tempdir().unwrap();
        // 本机代理的 base_url 无法映射到 models.dev 的任何分组，匹配必须只看模型 ID
        let result = build_catalog_result(
            &["big-pickle".to_string()],
            dir.path(),
            "http://127.0.0.1:9000/v1",
        )
        .unwrap();
        assert_eq!(result.ready, 1);
        assert_eq!(result.unmatched, 0);
        let model = &result.models[0];
        assert_eq!(model.display_name, "Big-Pickle");
        assert!(model.selectable);
        assert!(has_source(model, "models_dev"));

        let entry = catalog_entry(&result, "big-pickle");
        assert_eq!(entry["context_window"], json!(200_000));
        assert_eq!(entry["max_context_window"], json!(200_000));
        // 160000 / 200000 → 80%，压缩阈值取输入上限的 90%
        assert_eq!(entry["effective_context_window_percent"], json!(80));
        assert_eq!(entry["auto_compact_token_limit"], json!(144_000));

        // 候选证据不体现提供方 / 作用域
        let evidence = serde_json::to_value(&model.sources).unwrap();
        assert!(evidence[0].get("scope").is_none());
    }

    #[test]
    fn official_entry_is_reused_for_models_added_to_official_pool() {
        let dir = tempdir().unwrap();
        let result = build_catalog_result(
            &["deepseek-flash".to_string()],
            dir.path(),
            "https://api.deepseek.com/v1",
        )
        .unwrap();
        let entry = catalog_entry(&result, "deepseek-flash");
        // 官方条目池里的 deepseek 条目（开发者手工追加）整条生效，不再走多源合并
        let pool: Value =
            serde_json::from_str(include_str!("../../../resources/official-models.json")).unwrap();
        let expected = pool["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|model| model["slug"] == json!("deepseek-flash"))
            .expect("官方条目池缺少 deepseek-flash");
        for (key, value) in expected.as_object().unwrap() {
            if matches!(key.as_str(), "slug" | "priority" | "prefer_websockets") {
                continue;
            }
            assert_eq!(entry.get(key), Some(value), "键 {key} 应原样复用官方条目");
        }
        assert_eq!(result.ready, 1);
        assert_eq!(result.models[0].status, ModelCatalogModelStatus::Ready);
        assert!(result.models[0].selectable);
        assert!(has_source(&result.models[0], "official"));
        assert!(!has_source(&result.models[0], "models_dev"));
        assert!(!has_source(&result.models[0], "openrouter"));
    }

    #[test]
    fn models_dev_overrides_openrouter_when_no_official_entry() {
        let dir = tempdir().unwrap();
        let result = build_catalog_result(
            &["deepseek-v4-flash-vision-exp".to_string()],
            dir.path(),
            "https://api.deepseek.com/v1",
        )
        .unwrap();
        let entry = catalog_entry(&result, "deepseek-v4-flash-vision-exp");
        // models.dev 给 1000000，OpenRouter 给 1048576：前者应覆盖后者
        assert_eq!(entry["context_window"], json!(1_000_000));
        assert_eq!(entry["max_context_window"], json!(1_000_000));
        // 全局索引里该 ID 有多份副本（分组 id 字典序第一份），描述随之取那一份
        assert!(entry["description"]
            .as_str()
            .unwrap()
            .contains("DeepSeek V4 Flash"));
        assert!(!has_source(&result.models[0], "official"));
        assert!(has_source(&result.models[0], "models_dev"));
        assert!(has_source(&result.models[0], "openrouter"));
    }

    #[test]
    fn official_entry_is_reused_verbatim_for_gpt_models() {
        let dir = tempdir().unwrap();
        // GPT/codex 基线条目来自本机 codex 的运行期导出，不再内置在资源里
        codex_models::write_export_for_test(
            dir.path(),
            vec![json!({
                "slug": "gpt-5.6-sol",
                "max_context_window": 872_000,
                "tool_mode": "code_mode_only",
                "prefer_websockets": true,
                "model_messages": { "instructions_template": "You are Codex" }
            })],
        );
        let result = build_catalog_result(
            &["gpt-5.6-sol".to_string()],
            dir.path(),
            "https://relay.example.com/v1",
        )
        .unwrap();
        let entry = catalog_entry(&result, "gpt-5.6-sol");
        assert_eq!(entry["max_context_window"], json!(872000));
        assert_eq!(entry["tool_mode"], json!("code_mode_only"));
        assert_eq!(entry["prefer_websockets"], json!(false));
        assert!(entry["model_messages"]["instructions_template"]
            .as_str()
            .unwrap()
            .starts_with("You are Codex"));
        assert!(has_source(&result.models[0], "official"));
        assert!(!has_source(&result.models[0], "models_dev"));
        assert!(!has_source(&result.models[0], "openrouter"));
    }

    #[test]
    fn unmatched_models_are_skipped_but_reported() {
        let dir = tempdir().unwrap();
        let result = build_catalog_result(
            &[
                "deepseek-flash".to_string(),
                "definitely-not-a-real-model-xyz".to_string(),
            ],
            dir.path(),
            "https://api.deepseek.com/v1",
        )
        .unwrap();
        assert_eq!(result.total, 2);
        assert_eq!(result.ready, 1);
        assert_eq!(result.incompatible, 0);
        assert_eq!(result.unmatched, 1);
        assert_eq!(result.models[1].status, ModelCatalogModelStatus::Unmatched);
        assert!(!result.models[1].selectable);
        let catalog: Value = serde_json::from_str(&result.catalog).unwrap();
        assert_eq!(catalog["models"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn all_unmatched_still_returns_candidate_details() {
        let dir = tempdir().unwrap();
        let result = build_catalog_result(
            &["definitely-not-a-real-model-xyz".to_string()],
            dir.path(),
            "https://relay.example.com/v1",
        )
        .unwrap();
        assert_eq!(result.ready, 0);
        assert_eq!(result.unmatched, 1);
        assert_eq!(result.models[0].warnings, vec!["未匹配到可用资料"]);
        let catalog: Value = serde_json::from_str(&result.catalog).unwrap();
        assert!(catalog["models"].as_array().unwrap().is_empty());
    }

    #[test]
    fn explicit_tool_incompatibility_and_deprecated_status_are_rejected() {
        let no_tools = ModelFacts {
            supports_tool_calls: Some(false),
            ..ModelFacts::default()
        };
        assert!(incompatible_reason(&no_tools).unwrap().contains("tool_call=false"));

        let deprecated = ModelFacts {
            status: Some("deprecated".to_string()),
            ..ModelFacts::default()
        };
        assert!(incompatible_reason(&deprecated)
            .unwrap()
            .contains("deprecated"));
    }

    #[test]
    fn priorities_follow_selection_order() {
        let dir = tempdir().unwrap();
        let result = build_catalog_result(
            &["deepseek-flash".to_string(), "deepseek-v4-pro".to_string()],
            dir.path(),
            "https://api.deepseek.com/v1",
        )
        .unwrap();
        let catalog: Value = serde_json::from_str(&result.catalog).unwrap();
        let entries = catalog["models"].as_array().unwrap();
        assert_eq!(entries[0]["priority"], json!(1));
        assert_eq!(entries[1]["priority"], json!(2));
    }
}
