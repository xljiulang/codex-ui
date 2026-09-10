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
use sources::{FactSource, FullEntrySource};

/// 弹窗里的来源标记：命中了哪些数据源。
const SOURCE_OFFICIAL: &str = "official";
const SOURCE_MODELS_DEV: &str = "models_dev";
const SOURCE_OPENROUTER: &str = "openrouter";

/// 启动时后台刷新各字段源的运行时缓存；失败静默。
pub use sources::refresh_caches as refresh_source_caches;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ModelCatalogModelOption {
    pub id: String,
    pub display_name: String,
    /// 是否命中可用资料（未命中的模型不会生成条目）
    pub matched: bool,
    /// 命中了官方条目（整条复用）
    pub official: bool,
    /// 命中了 models.dev 资料
    pub models_dev: bool,
    /// 命中了 OpenRouter 资料
    pub openrouter: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ModelCatalogGenerateResult {
    pub catalog: String,
    pub total: usize,
    pub matched: usize,
    pub skipped: usize,
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
    let full_entry_sources = sources::full_entry_sources(base_url);
    let fact_sources = sources::fact_sources(app_dir, base_url);

    let mut entries: Vec<Value> = Vec::new();
    let mut models: Vec<ModelCatalogModelOption> = Vec::with_capacity(ids.len());
    for id in ids {
        let display_name = template::display_name_from_slug(id);

        if let Some((source_id, entry)) = lookup_full_entry(&full_entry_sources, id) {
            let entry = finalize_full_entry(entry, id, entries.len() + 1)?;
            entries.push(entry);
            models.push(model_option(id, display_name, &[source_id]));
            continue;
        }

        let (facts, hits) = collect_facts(id, &fact_sources);
        if hits.is_empty() {
            models.push(model_option(id, display_name, &[]));
            continue;
        }
        entries.push(template::render(id, &facts, entries.len() + 1)?);
        models.push(model_option(id, display_name, &hits));
    }

    if entries.is_empty() {
        return Err(format!(
            "模型提供者返回的 {} 个模型均未匹配到可用资料，未生成模型目录",
            ids.len()
        ));
    }

    let matched = models.iter().filter(|model| model.matched).count();
    let skipped = models.len().saturating_sub(matched);
    let catalog = serde_json::to_string_pretty(&json!({ "models": entries }))
        .map_err(|e| format!("生成模型目录 JSON 失败: {e}"))?;
    Ok(ModelCatalogGenerateResult {
        catalog,
        total: models.len(),
        matched,
        skipped,
        models,
    })
}

/// 完整条目源：命中即短路，不进入字段合并与模板渲染。
fn lookup_full_entry(
    sources: &[Box<dyn FullEntrySource>],
    model_id: &str,
) -> Option<(&'static str, Value)> {
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

/// 逐字段源提取并按注册顺序合并（靠后的源覆盖靠前的源）。
fn collect_facts(model_id: &str, sources: &[Box<dyn FactSource>]) -> (ModelFacts, Vec<&'static str>) {
    let mut facts = ModelFacts::default();
    let mut hits = Vec::new();
    for source in sources {
        if let Some(partial) = source.extract(model_id) {
            hits.push(source.id());
            facts.merge_from(source.id(), partial);
        }
    }
    (facts, hits)
}

fn model_option(
    id: &str,
    display_name: String,
    hits: &[&'static str],
) -> ModelCatalogModelOption {
    ModelCatalogModelOption {
        id: id.to_string(),
        display_name,
        matched: !hits.is_empty(),
        official: hits.contains(&SOURCE_OFFICIAL),
        models_dev: hits.contains(&SOURCE_MODELS_DEV),
        openrouter: hits.contains(&SOURCE_OPENROUTER),
    }
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

        fn extract(&self, _model_id: &str) -> Option<ModelFacts> {
            Some(ModelFacts {
                context_window: Some(999),
                ..ModelFacts::default()
            })
        }
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
    fn pipeline_merges_sources_in_registration_order() {
        let dir = tempdir().unwrap();
        let base_url = "https://api.deepseek.com/v1";
        let sources: Vec<Box<dyn FactSource>> = vec![
            Box::new(sources::openrouter::OpenRouterSource::load(dir.path())),
            Box::new(sources::models_dev::ModelsDevSource::load(
                dir.path(),
                base_url,
            )),
            Box::new(FakeSource),
        ];
        let (facts, hits) = collect_facts("deepseek-flash", &sources);
        assert_eq!(hits, vec!["openrouter", "models_dev", "fake"]);
        assert_eq!(facts.context_window, Some(999), "最后的源覆盖上下文");
        assert_eq!(facts.source_of("context_window"), Some("fake"));
        assert_eq!(
            facts.source_of("input_modalities"),
            Some("models_dev"),
            "未提供该字段的源不应覆盖"
        );
        assert_eq!(facts.source_of("support_verbosity"), Some("openrouter"));
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
        assert_eq!(result.matched, 1);
        assert!(result.models[0].official);
        assert!(!result.models[0].models_dev);
        assert!(!result.models[0].openrouter);
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
        assert!(entry["description"]
            .as_str()
            .unwrap()
            .contains("Experimental multimodal"));
        assert!(!result.models[0].official);
        assert!(result.models[0].models_dev);
        assert!(result.models[0].openrouter);
    }

    #[test]
    fn official_entry_is_reused_verbatim_for_gpt_models() {
        let dir = tempdir().unwrap();
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
        assert!(result.models[0].official);
        assert!(!result.models[0].models_dev);
        assert!(!result.models[0].openrouter);
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
        assert_eq!(result.matched, 1);
        assert_eq!(result.skipped, 1);
        assert!(!result.models[1].matched);
        let catalog: Value = serde_json::from_str(&result.catalog).unwrap();
        assert_eq!(catalog["models"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn all_unmatched_reports_error() {
        let dir = tempdir().unwrap();
        let error = build_catalog_result(
            &["definitely-not-a-real-model-xyz".to_string()],
            dir.path(),
            "https://relay.example.com/v1",
        )
        .unwrap_err();
        assert!(error.contains("均未匹配到可用资料"));
    }

    #[test]
    fn priorities_follow_selection_order() {
        let dir = tempdir().unwrap();
        let result = build_catalog_result(
            &["deepseek-flash".to_string(), "gpt-5.6-sol".to_string()],
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
