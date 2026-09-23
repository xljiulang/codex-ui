//! 模型目录生成：完整条目源优先，其次按字段可信度合并，最后渲染并校验。
//!
//! 管线（见 `sources` 模块）：
//! 1. 依次询问完整条目源（`official`）：命中即整条复用，解决自定义目录里
//!    GPT 类模型不如 codex 自带条目、或厂商条目与公开数据源不一致的问题；
//! 2. 否则各字段源独立提取，匹配准确度优先于来源权威性；
//! 3. 合并结果套 [`template`] 渲染，按 [`validation`] 约束自动修正并隔离非法条目。
//!
//! 数据源来自提供方 `/models` 的模型 ID 列表，元数据由上述源补全；新增数据源只需
//! 在 `sources::fact_sources` 里追加一行。

mod codex_models;
mod facts;
mod matching;
mod sources;
mod template;
mod validation;

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use facts::{FieldProvenance, FieldQuality, ModelFacts, Provenance};
use sources::{FactMatch, FactSource, FullEntryMatch, FullEntrySource};

/// 启动时后台导出本机 codex 自带的官方条目；失败保留旧缓存。
pub use codex_models::refresh_codex_models;
/// 启动时后台刷新各字段源的运行时缓存；失败静默。
pub use sources::refresh_caches as refresh_source_caches;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelCatalogModelStatus {
    Ready,
    Incompatible,
    Unmatched,
    Invalid,
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
    /// 仅内部使用的字段来源依据（合并与校验依赖它），不发给前端。
    #[serde(skip)]
    pub field_provenance: Provenance,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ModelCatalogGenerateResult {
    pub catalog: String,
    pub total: usize,
    pub ready: usize,
    pub incompatible: usize,
    pub unmatched: usize,
    pub invalid: usize,
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
    let snapshot = codex_models::codex_snapshot(app_dir);
    let full_entry_sources = sources::full_entry_sources(&snapshot, base_url);
    let fact_sources = sources::fact_sources(app_dir);
    // 渲染模板随用户安装的 codex 版本走（基底 = 启动导出的第一条 + 模板资源覆盖）
    let render_template = template::from_snapshot(&snapshot)?;

    let mut entries: Vec<Value> = Vec::new();
    let mut models: Vec<ModelCatalogModelOption> = Vec::with_capacity(ids.len());
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        let display_name = template::display_name_from_slug(id);
        let mut warnings: Vec<String> = full_entry_sources
            .iter()
            .flat_map(|source| source.warnings(id))
            .collect();

        if let Some((source_id, matched)) = lookup_full_entry(&full_entry_sources, id) {
            let source_evidence = full_entry_evidence(source_id, &matched);
            let mut entry = finalize_full_entry(matched.entry, id, entries.len() + 1)?;
            template::ensure_keys(&mut entry, &render_template.entry);
            // 摘要策略在补键之后执行，避免基底又把它带回来；执行后再记录来源
            if let Some(object) = entry.as_object_mut() {
                apply_reasoning_summary_policy(object);
            }
            let mut provenance = Provenance::new();
            if let Some(object) = entry.as_object() {
                for key in validation::OUTPUT_FIELDS {
                    if let Some(value) = object.get(*key) {
                        let mut origin = FieldProvenance::new(
                            source_id,
                            &matched.matched_id,
                            matched.kind,
                            FieldQuality::from(100),
                        );
                        origin.value = value.clone();
                        origin.reason = "完整条目复用".to_string();
                        provenance.insert(*key, origin);
                    }
                }
            }
            let status = match validation::validate_entry(&mut entry, &mut warnings) {
                Ok(()) => {
                    validation::complete_provenance(&entry, &mut provenance);
                    entries.push(entry);
                    ModelCatalogModelStatus::Ready
                }
                Err(error) => {
                    warnings.push(error);
                    ModelCatalogModelStatus::Invalid
                }
            };
            models.push(ModelCatalogModelOption {
                id: id.clone(),
                display_name,
                status,
                selectable: status == ModelCatalogModelStatus::Ready,
                sources: vec![source_evidence],
                warnings,
                field_provenance: provenance,
            });
            continue;
        }

        let mut collected = collect_facts(id, &fact_sources);
        if collected.sources.is_empty() {
            warnings.push("未匹配到可用资料".to_string());
            models.push(ModelCatalogModelOption {
                id: id.clone(),
                display_name,
                status: ModelCatalogModelStatus::Unmatched,
                selectable: false,
                sources: Vec::new(),
                warnings,
                field_provenance: Provenance::new(),
            });
            continue;
        }

        warnings.append(&mut collected.facts.warnings);
        if collected
            .sources
            .iter()
            .any(|source| source.match_kind == "fuzzy")
        {
            warnings.push("部分资料继承自近似模型，未验证目标服务实际能力".to_string());
        }
        validation::reconcile_facts(&mut collected.facts, &mut warnings);
        let incompatible_reason = incompatible_reason(&collected.facts);
        let status = if let Some(reason) = incompatible_reason {
            warnings.push(reason);
            ModelCatalogModelStatus::Incompatible
        } else {
            let mut entry =
                template::render(&render_template, id, &collected.facts, entries.len() + 1)?;
            template::ensure_keys(&mut entry, &render_template.entry);
            match validation::validate_entry(&mut entry, &mut warnings) {
                Ok(()) => {
                    validation::complete_provenance(&entry, &mut collected.facts.provenance);
                    entries.push(entry);
                    ModelCatalogModelStatus::Ready
                }
                Err(error) => {
                    warnings.push(error);
                    ModelCatalogModelStatus::Invalid
                }
            }
        };
        models.push(ModelCatalogModelOption {
            id: id.clone(),
            display_name,
            status,
            selectable: status == ModelCatalogModelStatus::Ready,
            sources: collected.sources,
            warnings,
            field_provenance: collected.facts.provenance,
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
    let invalid = models
        .iter()
        .filter(|model| model.status == ModelCatalogModelStatus::Invalid)
        .count();
    let catalog = serde_json::to_string_pretty(&json!({ "models": entries }))
        .map_err(|e| format!("生成模型目录 JSON 失败: {e}"))?;
    Ok(ModelCatalogGenerateResult {
        catalog,
        total: models.len(),
        ready,
        incompatible,
        unmatched,
        invalid,
        models,
    })
}

/// 完整条目源：命中即短路，不进入字段合并与模板渲染。
fn lookup_full_entry(
    sources: &[Box<dyn FullEntrySource>],
    model_id: &str,
) -> Option<(&'static str, FullEntryMatch)> {
    sources.iter().find_map(|source| {
        source
            .full_entry(model_id)
            .map(|entry| (source.id(), entry))
    })
}

/// 覆盖身份、顺序与推理摘要策略：`slug` 用提供方原始 id，`priority` 按候选原顺序。
fn finalize_full_entry(mut entry: Value, model_id: &str, priority: usize) -> Result<Value, String> {
    let object = entry
        .as_object_mut()
        .ok_or_else(|| "完整条目源返回的条目不是 JSON 对象".to_string())?;
    object.insert("slug".to_string(), json!(model_id));
    object.insert("priority".to_string(), json!(priority as i64));
    Ok(entry)
}

/// 复用条目时打开推理摘要：官方 GPT 条目默认 `none`，而 GPT 系只回加密推理 + 摘要，
/// 不请求就等于思考过程空白；同时清掉会挡掉摘要请求的旧字段。
fn apply_reasoning_summary_policy(object: &mut serde_json::Map<String, Value>) {
    let reasoning = object
        .get("supported_reasoning_levels")
        .and_then(Value::as_array)
        .is_some_and(|levels| !levels.is_empty());
    if reasoning {
        object.insert("default_reasoning_summary".to_string(), json!("auto"));
    }
    if object.get("supports_reasoning_summary_parameter") == Some(&Value::Bool(false)) {
        object.remove("supports_reasoning_summary_parameter");
    }
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
        if let Some(mut matched) = source.extract(model_id) {
            let quality = matched.quality(source.quality_base());
            matched
                .facts
                .attach_match(source.id(), &matched.matched_id, matched.kind, quality);
            facts.merge_from(source.id(), quality, matched.facts.clone());
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
    let reliable = |field| {
        facts
            .provenance
            .get(field)
            .is_none_or(|origin| origin.match_kind != matching::MatchKind::Fuzzy)
    };
    if facts.supports_tool_calls == Some(false) && reliable("supports_tool_calls") {
        return Some("上游明确标记 tool_call=false，不兼容 Codex 工具调用".to_string());
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

    fn write_source_fixture(dir: &Path, openrouter: Value, models_dev: Value) {
        std::fs::create_dir_all(dir.join("cache")).unwrap();
        std::fs::write(
            dir.join("cache/openrouter-models.json"),
            openrouter.to_string(),
        )
        .unwrap();
        std::fs::write(dir.join("cache/models-dev.json"), models_dev.to_string()).unwrap();
    }

    #[test]
    fn precise_source_wins_over_fuzzy_source_per_field() {
        let dir = tempdir().unwrap();
        write_source_fixture(
            dir.path(),
            json!({"data":[{"id":"review-model-v5","context_length":128000,
                "supported_parameters":["tools"],"reasoning":{"supported_efforts":[]}}]}),
            json!({"providers":{"relay":{"models":{"review-model-v4":{
                "limit":{"context":1000000},"tool_call":false,
                "description":"继承描述","reasoning_options":[{"type":"effort","values":["high"]}]
            }}}}}),
        );
        let result = build_catalog_result(
            &["review-model-v5".into()],
            dir.path(),
            "https://relay.example.com",
        )
        .unwrap();
        assert_eq!(result.ready, 1);
        let entry = catalog_entry(&result, "review-model-v5");
        assert_eq!(entry["context_window"], json!(128000));
        assert_eq!(entry["supported_reasoning_levels"], json!([]));
        assert_eq!(entry["description"], json!("继承描述"));
        assert_eq!(
            result.models[0].field_provenance["context_window"].source,
            "openrouter"
        );
        assert_eq!(
            result.models[0].field_provenance["description"].source,
            "models_dev"
        );
        assert!(result.models[0]
            .warnings
            .iter()
            .any(|warning| warning.contains("近似")));
    }

    #[test]
    fn reused_official_entry_enables_reasoning_summary() {
        let dir = tempdir().unwrap();
        codex_models::write_export_for_test(
            dir.path(),
            vec![json!({
                "slug":"gpt-5.6-luna",
                "model_messages":{"instructions_template":"test"},
                "supported_reasoning_levels":[{"effort":"low","description":"low"}],
                "default_reasoning_summary":"none",
                "supports_reasoning_summary_parameter": false
            })],
        );
        let result = build_catalog_result(
            &["gpt-5.6-luna".into()],
            dir.path(),
            "https://relay.example.com",
        )
        .unwrap();
        // GPT 系只回加密推理 + 摘要，复用条目必须把官方默认的 none 打开
        let entry = catalog_entry(&result, "gpt-5.6-luna");
        assert_eq!(entry["default_reasoning_summary"], json!("auto"));
        assert!(entry.get("supports_reasoning_summary_parameter").is_none());
    }

    #[test]
    fn response_keeps_source_badges_but_hides_field_provenance() {
        let dir = tempdir().unwrap();
        write_source_fixture(
            dir.path(),
            json!({"data":[{"id":"review-model-v5","context_length":128000,
                "supported_parameters":["tools"]}]}),
            json!({"providers":{"relay":{"models":{"zzzzzzz":{"tool_call":true}}}}}),
        );
        let result = build_catalog_result(
            &["review-model-v5".into()],
            dir.path(),
            "https://relay.example.com",
        )
        .unwrap();
        // 候选行来源徽章仍需要 sources[]；字段来源面板已下线，故不再发给前端
        let payload = serde_json::to_value(&result).unwrap();
        let option = &payload["models"][0];
        assert!(option.get("sources").is_some(), "来源徽章数据必须保留");
        assert!(
            option.get("field_provenance").is_none(),
            "参数来源面板已下线"
        );
        assert!(result.models[0]
            .field_provenance
            .contains_key("context_window"));
    }

    #[test]
    fn upstream_status_is_ignored_for_exact_and_fuzzy_matches() {
        let dir = tempdir().unwrap();
        write_source_fixture(
            dir.path(),
            json!({"data":[
                {"id":"review-model-v4","status":" DEPRECATED ","supported_parameters":["tools"]},
                {"id":"other-ready","status":" BETA ","supported_parameters":["tools"]}
            ]}),
            json!({"providers":{"relay":{"models":{"zzzzzzz":{"tool_call":true}}}}}),
        );
        let result = build_catalog_result(
            &[
                "review-model-v4".into(),
                "review-model-v5".into(),
                "other-ready".into(),
            ],
            dir.path(),
            "https://relay.example.com",
        )
        .unwrap();
        // 第三方资料的 status 不再参与任何判定：精确命中的 deprecated / beta 与模糊继承的 v5 都可生成
        assert_eq!(result.ready, 3);
        for model in &result.models {
            assert_eq!(
                model.status,
                ModelCatalogModelStatus::Ready,
                "{:?}",
                model.warnings
            );
            assert!(model.selectable, "{:?}", model.warnings);
            assert!(
                !model
                    .warnings
                    .iter()
                    .any(|warning| { warning.contains("deprecated") || warning.contains("beta") }),
                "{:?}",
                model.warnings
            );
        }
        assert!(catalog_entry(&result, "review-model-v4")
            .get("status")
            .is_none());
    }

    #[test]
    fn invalid_entry_is_isolated_and_priorities_remain_contiguous() {
        let dir = tempdir().unwrap();
        codex_models::write_export_for_test(
            dir.path(),
            vec![
                json!({"slug":"invalid-model","model_messages":{"instructions_template":"test"},
                "context_window":0}),
                json!({"slug":"valid-model","model_messages":{"instructions_template":"test"}}),
            ],
        );
        let result = build_catalog_result(
            &[
                "invalid-model".into(),
                "valid-model".into(),
                "valid-model".into(),
            ],
            dir.path(),
            "https://relay.example.com",
        )
        .unwrap();
        assert_eq!(result.invalid, 1);
        assert_eq!(result.ready, 1);
        assert_eq!(result.total, 2);
        assert!(!result.models[0].selectable);
        assert_eq!(catalog_entry(&result, "valid-model")["priority"], json!(1));
        assert_eq!(
            result.total,
            result.ready + result.incompatible + result.unmatched + result.invalid
        );
    }

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

    /// 模板持有的精简提示词：未命中完整条目源的条目两个提示词字段都取这个值。
    fn template_prompt(dir: &Path) -> String {
        template::effective_template(dir).unwrap().entry["model_messages"]["instructions_template"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn source_differences_only_warn_for_constraints() {
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
            model
                .warnings
                .iter()
                .all(|warning| warning.contains("输入上限")),
            "普通来源差异不应逐项告警：{:?}",
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
    fn generated_entries_follow_codex_export_keys() {
        let dir = tempdir().unwrap();
        // 导出基底带一个覆盖清单里没有的键（模拟更高版本 codex 的新字段）
        codex_models::write_export_for_test(
            dir.path(),
            vec![json!({
                "slug": "gpt-5.7-sol",
                "node_repl_disabled": true,
                "shell_type": "shell_command",
                "visibility": "list",
                "model_messages": { "instructions_template": "You are Codex (new)" }
            })],
        );

        let result = build_catalog_result(
            &["deepseek-v4-flash-vision-exp".to_string()],
            dir.path(),
            "https://relay.example.com/v1",
        )
        .unwrap();
        let entry = catalog_entry(&result, "deepseek-v4-flash-vision-exp");
        assert_eq!(
            entry["node_repl_disabled"],
            json!(true),
            "生成条目应带上本机 codex 的新键"
        );
        assert_eq!(
            entry["model_messages"]["instructions_template"],
            json!(template_prompt(dir.path())),
            "未命中完整条目源的条目应使用模板提示词"
        );
        assert_eq!(
            entry["base_instructions"],
            json!(template_prompt(dir.path())),
            "base_instructions 与 instructions_template 同写模板提示词"
        );
    }

    #[test]
    fn reused_entries_are_filled_with_template_keys() {
        let dir = tempdir().unwrap();
        codex_models::write_export_for_test(
            dir.path(),
            vec![json!({
                "slug": "gpt-5.7-sol",
                "node_repl_disabled": true,
                "shell_type": "shell_command",
                "visibility": "list",
                "model_messages": { "instructions_template": "You are Codex (new)" }
            })],
        );

        // 手写的第三方官方条目本身缺少部分模板键，写出前应被补齐
        let result = build_catalog_result(
            &["deepseek-flash".to_string()],
            dir.path(),
            "https://api.deepseek.com/v1",
        )
        .unwrap();
        let entry = catalog_entry(&result, "deepseek-flash");
        assert!(has_source(&result.models[0], "official"));
        for key in ["available_in_plans", "service_tiers", "node_repl_disabled"] {
            assert!(entry.get(key).is_some(), "缺失键 {key} 应被补齐");
        }
        assert_eq!(entry["prefer_websockets"], json!(false));
    }

    #[test]
    fn generated_catalog_is_accepted_by_local_codex() {
        // 需要真实 codex：与其它集成用例同一门控（未设置 CODEX_BIN 时跳过）
        let Ok(codex) = std::env::var("CODEX_BIN") else {
            eprintln!("skip: 未设置 CODEX_BIN");
            return;
        };
        let ids = [
            "deepseek-flash".to_string(),
            "big-pickle".to_string(),
            "gpt-5.7".to_string(),
        ];

        // 1) 无导出：兜底基底 + 模板覆盖
        let fallback_dir = tempdir().unwrap();
        assert_catalog_accepted_by_codex(&codex, fallback_dir.path(), &ids);

        // 2) 有导出：基底 = 本机 codex 自带目录第一条（版本对齐路径）
        let export_dir = tempdir().unwrap();
        let exported = std::process::Command::new(&codex)
            .args(["debug", "models", "--bundled"])
            .output()
            .unwrap();
        assert!(exported.status.success(), "导出本机 codex 目录失败");
        let cache = codex_models::cache_path(export_dir.path());
        std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
        std::fs::write(&cache, &exported.stdout).unwrap();
        let exported_value: Value = serde_json::from_slice(&exported.stdout).unwrap();
        let mut runtime_ids = ids.to_vec();
        runtime_ids.push(
            exported_value["models"][0]["slug"]
                .as_str()
                .unwrap()
                .to_string(),
        );
        assert_catalog_accepted_by_codex(&codex, export_dir.path(), &runtime_ids);

        // 3) 精确来源胜过模糊来源、同级冲突补缺、自定义档位与 audio 的真实解析。
        let fixture_dir = tempdir().unwrap();
        write_source_fixture(
            fixture_dir.path(),
            json!({"data":[
                {"id":"review-audio-v1","context_length":32000,"supported_parameters":["tools"],
                 "architecture":{"input_modalities":["text","audio"]},
                 "reasoning":{"supported_efforts":["turbo"],"default_effort":"turbo"}},
                {"id":"review-conflict-v1","context_length":64000,"supported_parameters":["tools"]}
            ]}),
            json!({"providers":{
                "alpha":{"models":{
                    "review-audio-v0":{"limit":{"context":4096},"tool_call":true},
                    "review-conflict-v1":{"limit":{"context":100},"tool_call":true}
                }},
                "beta":{"models":{"review-conflict-v1":{"limit":{"context":200},"tool_call":true}}}
            }}),
        );
        let fixture_ids = [
            "review-audio-v1".to_string(),
            "review-conflict-v1".to_string(),
        ];
        let fixture_result = build_catalog_result(
            &fixture_ids,
            fixture_dir.path(),
            "https://relay.example.com",
        )
        .unwrap();
        assert_eq!(
            catalog_entry(&fixture_result, "review-audio-v1")["context_window"],
            json!(32000)
        );
        assert_eq!(
            catalog_entry(&fixture_result, "review-conflict-v1")["context_window"],
            json!(64000)
        );
        assert_catalog_accepted_by_codex(&codex, fixture_dir.path(), &fixture_ids);
    }

    /// 用给定 app_dir 生成目录，塞进临时 `CODEX_HOME` 交给真实 codex 解析，
    /// 并断言生成的模型都在结果里（缺任何必需字段都会让整份目录解析失败）。
    fn assert_catalog_accepted_by_codex(codex: &str, app_dir: &Path, ids: &[String]) {
        let result = build_catalog_result(ids, app_dir, "https://relay.example.com/v1").unwrap();
        assert_eq!(result.ready, ids.len());

        let home = tempdir().unwrap();
        let catalog = home.path().join("models.json");
        std::fs::write(&catalog, &result.catalog).unwrap();
        std::fs::write(
            home.path().join("config.toml"),
            format!("model_catalog_json = '{}'\n", catalog.display()),
        )
        .unwrap();
        let output = std::process::Command::new(codex)
            .args(["debug", "models"])
            .env("CODEX_HOME", home.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "codex 拒绝生成的目录：{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        let slugs: Vec<&str> = value["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|model| model["slug"].as_str())
            .collect();
        for id in ids {
            assert!(slugs.contains(&id.as_str()), "缺少 {id}，实际：{slugs:?}");
            let expected = catalog_entry(&result, id);
            let actual = value["models"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["slug"].as_str() == Some(id.as_str()))
                .unwrap();
            for key in [
                "slug",
                "priority",
                "context_window",
                "max_context_window",
                "input_modalities",
                "supported_reasoning_levels",
                "default_reasoning_level",
            ] {
                assert_eq!(actual[key], expected[key], "{id} 的 {key} 加载后发生变化");
            }
        }
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
            // default_reasoning_summary 由复用策略覆盖为 auto（推理条目打开摘要）
            if matches!(
                key.as_str(),
                "slug" | "priority" | "prefer_websockets" | "default_reasoning_summary"
            ) {
                continue;
            }
            assert_eq!(entry.get(key), Some(value), "键 {key} 应原样复用官方条目");
        }
        assert_eq!(entry["default_reasoning_summary"], json!("auto"));
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
        // 全局索引里该 ID 有多份资料，描述按字段可信度合并。
        assert!(entry["description"]
            .as_str()
            .unwrap()
            .contains("DeepSeek V4 Flash"));
        assert!(!has_source(&result.models[0], "official"));
        assert!(has_source(&result.models[0], "models_dev"));
        assert!(has_source(&result.models[0], "openrouter"));
    }

    #[test]
    fn empty_reasoning_options_do_not_clear_levels_from_openrouter() {
        // 机制回归：精确命中 models.dev 的 provider 级记录（声明会推理、档位却是空数组），
        // 修复前该空数组会被当成"明确无档位"，以更高的来源质量清掉 OpenRouter 给出的
        // supported_efforts，目录里的 `supported_reasoning_levels` 变成 `[]`，
        // codex 于是从不请求推理。用夹具复刻这条链路（真实数据见下条注释）。
        let dir = tempdir().unwrap();
        write_source_fixture(
            dir.path(),
            json!({ "data": [{
                "id": "relay/mimo-demo",
                "canonical_slug": "relay/mimo-demo",
                "name": "MiMo Demo",
                "context_length": 200000,
                "reasoning": {
                    "mandatory": false,
                    "supported_efforts": ["high", "low"],
                    "default_effort": "high"
                }
            }] }),
            json!({ "providers": { "relay": { "models": { "mimo-demo": {
                "reasoning": true,
                "reasoning_options": []
            }}}}}),
        );
        let result = build_catalog_result(
            &["mimo-demo".to_string()],
            dir.path(),
            "https://relay.example.com/v1",
        )
        .unwrap();
        let entry = catalog_entry(&result, "mimo-demo");
        let levels: Vec<&str> = entry["supported_reasoning_levels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|level| level["effort"].as_str())
            .collect();
        assert!(
            !levels.is_empty(),
            "空档位不得清掉 OpenRouter 给出的档位：{entry}"
        );
        assert!(levels.contains(&"high"), "档位应含 high：{levels:?}");
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
                "base_instructions": "You are Codex (base)",
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
        // 复用条目逐字段原样：提示词（两个字段）都不被模板改写
        assert_eq!(entry["base_instructions"], json!("You are Codex (base)"));
        assert!(has_source(&result.models[0], "official"));
        assert!(!has_source(&result.models[0], "models_dev"));
        assert!(!has_source(&result.models[0], "openrouter"));
    }

    #[test]
    fn entries_outside_full_entry_sources_use_template_prompt() {
        let dir = tempdir().unwrap();
        // 复刻真实故障：big-pickle 不在 codex 导出池（也不在 official-models.json），
        // 走多源合并 + 模板渲染，因此提示词必须换成模板里的精简版
        let long_prompt = "You are Codex, an agent based on GPT-5. ".repeat(400);
        codex_models::write_export_for_test(
            dir.path(),
            vec![json!({
                "slug": "gpt-5.6-sol",
                "base_instructions": long_prompt,
                "model_messages": { "instructions_template": long_prompt }
            })],
        );
        let expected = template_prompt(dir.path());
        assert!(
            expected.chars().count() < 2_000,
            "模板提示词应是精简版：{expected}"
        );

        for base_url in [
            "http://127.0.0.1:18080/v1",
            "https://relay.example.com/v1",
            "https://api.openai.com/v1",
        ] {
            let result =
                build_catalog_result(&["big-pickle".to_string()], dir.path(), base_url).unwrap();
            let entry = catalog_entry(&result, "big-pickle");
            // 两个提示词字段同写，不依赖 codex 对二者的优先级
            assert_eq!(entry["base_instructions"], json!(expected), "{base_url}");
            assert_eq!(
                entry["model_messages"]["instructions_template"],
                json!(expected),
                "{base_url}"
            );
            // 与提供方 host 无关：规则只看是否命中完整条目源
            assert_eq!(result.models[0].status, ModelCatalogModelStatus::Ready);
            assert!(result.models[0].selectable);
        }
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
    fn explicit_tool_incompatibility_is_rejected() {
        let no_tools = ModelFacts {
            supports_tool_calls: Some(false),
            ..ModelFacts::default()
        };
        assert!(incompatible_reason(&no_tools)
            .unwrap()
            .contains("tool_call=false"));
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
