//! models.dev 字段源（覆盖源）：模型级的上下文、输入模态、推理档位与描述。
//!
//! 数据来自 <https://models.dev/catalog.json>（形态 `{ models, providers }`）；内置快照与
//! 运行时缓存同形态、全量（含中转/聚合商），缓存 24 小时内不重复下载。
//! **匹配只看模型 ID**：保留 `providers` 全部记录与规范模型资料，逐字段处理可信度和冲突。
//! 不按 `base_url` 定位目标提供方，因此同一个模型 ID 在任何目标提供方下结果一致。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde_json::{json, Value};

use super::{FactMatch, FactSource, write_atomic};
use crate::codex::model_catalog::facts::{FieldProvenance, FieldQuality, ModelFacts};
use crate::codex::model_catalog::matching::{Candidate, CandidateStore, release_from_date};

const MODELS_DEV_URL: &str = "https://models.dev/catalog.json";
const BUNDLED_MODELS_DEV: &str = include_str!("../../../../resources/models-dev.json");

pub struct ModelsDevSource {
    store: CandidateStore,
}

impl ModelsDevSource {
    pub fn load(app_dir: &Path) -> Self {
        let cached = std::fs::read_to_string(cache_path(app_dir))
            .ok()
            .and_then(|text| parse_catalog(&text).ok());
        Self { store: CandidateStore::new(cached.unwrap_or_else(|| bundled_items().to_vec())) }
    }
}

impl FactSource for ModelsDevSource {
    fn id(&self) -> &'static str {
        "models_dev"
    }

    /// 同等匹配准确度时优先于 OpenRouter，近似资料不能压过精确资料。
    fn quality_base(&self) -> u8 {
        30
    }

    fn extract(&self, model_id: &str) -> Option<FactMatch> {
        let matched = self.store.lookup(model_id)?;
        Some(FactMatch {
            facts: facts_from_records(matched.value),
            matched_id: matched.matched_id.to_string(),
            kind: matched.kind,
            score: matched.score,
        })
    }
}

/// 解析 catalog.json（`{ models, providers }`）并按模型 ID 展平成候选表。
///
/// 保留每个 ID 的规范、原厂及其他提供方记录，字段冲突不按字典序决定。
/// 旧版 api.json 形态（顶层直接是 provider 分组）
/// 没有 `providers` 键，这里判为非法，使旧缓存自动失效并重下。
fn parse_catalog(text: &str) -> Result<Vec<(Candidate, Value)>, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| format!("models.dev 目录不是合法 JSON: {e}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "models.dev 目录不是 JSON 对象".to_string())?;
    let providers = object
        .get("providers")
        .and_then(Value::as_object)
        .ok_or_else(|| "models.dev 目录缺少 providers 分组".to_string())?;

    // 保留所有原始记录；同名资料只有在字段解析阶段才比较可信度。
    let mut grouped: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut canonical_tails: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if let Some(canonical) = object.get("models").and_then(Value::as_object) {
        for (id, model) in canonical {
            if id.trim().is_empty() || !model.is_object() { continue; }
            canonical_tails.entry(id.rsplit('/').next().unwrap_or(id).to_string())
                .or_default().push(id.clone());
            grouped.entry(id.clone()).or_default().push(json!({
                "model": model, "matched_id": id, "provider_id": null, "tier": 3
            }));
        }
    }
    for (provider_id, provider) in providers {
        let Some(models) = provider.get("models").and_then(Value::as_object) else { continue; };
        for (id, model) in models {
            if id.trim().is_empty() || !model.is_object() { continue; }
            grouped.entry(id.clone()).or_default().push(json!({
                "model": model, "matched_id": id, "provider_id": provider_id, "tier": 1
            }));
        }
    }
    let mut items = Vec::new();
    for id in grouped.keys() {
        let tail = id.rsplit('/').next().unwrap_or(id);
        let canonical = canonical_tails.get(tail).and_then(|ids| {
            if id.contains('/') {
                ids.iter().find(|candidate| *candidate == id)
            } else if ids.len() == 1 { ids.first() } else { None }
        });
        let mut related = BTreeSet::from([id.as_str()]);
        if let Some(canonical) = canonical {
            related.insert(canonical);
            // 多命名空间共享尾段时，不把无前缀记录附到任一命名空间。
            if canonical_tails.get(tail).is_some_and(|ids| ids.len() == 1) {
                related.insert(tail);
            }
        }
        let namespace = canonical.and_then(|id| id.split_once('/').map(|(vendor, _)| vendor));
        let mut records = Vec::new();
        for related_id in related {
            if let Some(values) = grouped.get(related_id) {
                for value in values {
                    let mut value = value.clone();
                    if namespace.is_some() && value["provider_id"].as_str() == namespace {
                        value["tier"] = json!(2);
                    }
                    records.push(value);
                }
            }
        }
        records.sort_by(|left, right| {
            right["tier"].as_u64().cmp(&left["tier"].as_u64())
                .then_with(|| left["provider_id"].as_str().cmp(&right["provider_id"].as_str()))
                .then_with(|| left["matched_id"].as_str().cmp(&right["matched_id"].as_str()))
        });
        let mut representative = records[0]["model"].clone();
        representative["id"] = json!(id);
        representative["_records"] = json!(records);
        // 匹配排序也使用字段级归并后的状态，不能仅看恰好排在前面的记录。
        let resolved = facts_from_records(&representative);
        if let Some(status) = resolved.status {
            representative["status"] = json!(status);
        } else if let Some(object) = representative.as_object_mut() {
            object.remove("status");
        }
        let (candidate, payload) = build_item(id, &representative);
        items.push((candidate, payload));
    }

    if items.is_empty() {
        return Err("models.dev 目录没有可用模型".to_string());
    }
    Ok(items)
}

/// 单条模型记录 → 候选（family 只供模糊查找，发布时间取 release_date / last_updated）。
fn build_item(model_id: &str, model: &Value) -> (Candidate, Value) {
    let mut payload = model.clone();
    if payload.get("id").and_then(Value::as_str).is_none() {
        payload["id"] = json!(model_id);
    }
    let release = model
        .get("release_date")
        .and_then(Value::as_str)
        .and_then(release_from_date)
        .or_else(|| {
            model
                .get("last_updated")
                .and_then(Value::as_str)
                .and_then(release_from_date)
        });
    let status = model.get("status").and_then(Value::as_str);
    (
        Candidate::new(model_id.to_string(), Vec::new(), release, model_id)
            .with_family(model.get("family").and_then(Value::as_str)).with_status(status),
        payload,
    )
}

fn facts_from_records(model: &Value) -> ModelFacts {
    let Some(records) = model.get("_records").and_then(Value::as_array) else {
        return facts_from_model(model);
    };
    let records: Vec<_> = records.iter().map(|record| {
        let mut origin = FieldProvenance::new(
            "models_dev", record["matched_id"].as_str().unwrap_or(""),
            crate::codex::model_catalog::matching::MatchKind::Exact,
            FieldQuality::from(record["tier"].as_u64().unwrap_or(1) as u8),
        );
        origin.provider_id = record["provider_id"].as_str().map(str::to_string);
        (facts_from_model(&record["model"]), origin)
    }).collect();
    ModelFacts::resolve_records(&records)
}

/// 内联全量快照（约 3 MB）只解析一次进程复用。
fn bundled_items() -> &'static [(Candidate, Value)] {
    static BUNDLED: OnceLock<Vec<(Candidate, Value)>> = OnceLock::new();
    BUNDLED
        .get_or_init(|| parse_catalog(BUNDLED_MODELS_DEV).unwrap_or_default())
        .as_slice()
}

fn facts_from_model(model: &Value) -> ModelFacts {
    let mut facts = ModelFacts::default();
    let context_window = model
        .pointer("/limit/context")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0);
    facts.context_window = context_window;
    facts.max_context_window = context_window;
    facts.input_token_limit = model
        .pointer("/limit/input")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0);
    facts.input_modalities = input_modalities(model);
    facts.reasoning_levels = reasoning_levels(model);
    facts.supports_reasoning = supports_reasoning(model);
    facts.description = model
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .map(str::to_string);
    facts.supports_tool_calls = model.get("tool_call").and_then(Value::as_bool);
    facts.status = model
        .get("status")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|status| !status.is_empty())
        .map(str::to_ascii_lowercase);
    facts
}

fn input_modalities(model: &Value) -> Option<Vec<String>> {
    let items = model
        .pointer("/modalities/input")
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
    Some(modalities)
}

/// `reasoning_options` 里的 `{ type: "effort", values: [...] }` → 推理档位。
fn reasoning_levels(model: &Value) -> Option<Vec<String>> {
    let options = model.get("reasoning_options").and_then(Value::as_array)?;
    let mut levels = Vec::new();
    let mut explicit = options.is_empty();
    for option in options {
        if option.get("type").and_then(Value::as_str) != Some("effort") {
            continue;
        }
        let values = option.get("values").and_then(Value::as_array)?;
        if !values.iter().all(Value::is_string) { return None; }
        explicit = true;
        for value in values {
            let Some(effort) = value.as_str().map(|value| value.trim().to_ascii_lowercase()) else {
                continue;
            };
            if !levels.iter().any(|existing| existing == &effort) {
                levels.push(effort);
            }
        }
    }
    explicit.then_some(levels)
}

fn supports_reasoning(model: &Value) -> Option<bool> {
    let reasoning = model.get("reasoning").and_then(Value::as_bool);
    let has_options = model
        .get("reasoning_options")
        .and_then(Value::as_array)
        .is_some_and(|options| !options.is_empty());
    reasoning.map(|value| value || has_options).or_else(|| has_options.then_some(true))
}

fn cache_path(app_dir: &Path) -> PathBuf {
    app_dir.join("cache").join("models-dev.json")
}

/// 是否需要重新下载：缓存缺失、超过 24 小时或内容不可解析（损坏视为过期）。
fn needs_refresh(app_dir: &Path) -> bool {
    !super::cache_is_reusable(&cache_path(app_dir), |text| {
        parse_catalog(text).is_ok()
    })
}

/// 启动时后台刷新缓存（24 小时内跳过）；失败静默保留旧缓存/内置资源。
pub async fn refresh_cache(app_dir: &Path) {
    if !needs_refresh(app_dir) {
        return;
    }
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
    {
        Ok(client) => client,
        Err(_) => return,
    };
    let response = match client
        .get(MODELS_DEV_URL)
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
    parse_catalog(text)?;
    write_atomic(&cache_path(app_dir), text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn canonical_and_manufacturer_fields_are_combined_without_relay_extensions() {
        let text = json!({
            "models": { "vendor/model-v5": { "limit": { "context": 1000 }, "tool_call": true } },
            "providers": {
                "aaa-relay": { "models": { "model-v5": {
                    "limit": { "context": 9000 }, "reasoning_options": [{"type":"effort","values":["turbo"]}]
                }}},
                "vendor": { "models": { "model-v5": {
                    "limit": { "context": 2000, "input": 800 },
                    "reasoning_options": [], "status":"beta"
                }}}
            }
        }).to_string();
        let source = ModelsDevSource { store: CandidateStore::new(parse_catalog(&text).unwrap()) };
        for query in ["model-v5", "vendor/model-v5"] {
            let facts = source.extract(query).unwrap().facts;
            assert_eq!(facts.context_window, Some(1000));
            assert_eq!(facts.input_token_limit, Some(800));
            assert_eq!(facts.reasoning_levels, Some(vec![]));
            assert_eq!(facts.status.as_deref(), Some("beta"));
            assert_eq!(facts.provenance["context_window"].provider_id, None);
            assert_eq!(facts.provenance["reasoning_levels"].provider_id.as_deref(), Some("vendor"));
        }
    }

    #[test]
    fn equal_relay_conflicts_are_not_resolved_by_provider_name() {
        let text = json!({ "providers": {
            "alpha": { "models": { "private-model": { "limit":{"context":100}, "tool_call":true } } },
            "beta": { "models": { "private-model": { "limit":{"context":200}, "tool_call":true } } }
        }}).to_string();
        let source = ModelsDevSource { store: CandidateStore::new(parse_catalog(&text).unwrap()) };
        let facts = source.extract("private-model").unwrap().facts;
        assert_eq!(facts.context_window, None);
        assert_eq!(facts.supports_tool_calls, Some(true));
        assert!(!facts.warnings.is_empty());
    }

    #[test]
    fn ambiguous_unqualified_names_do_not_inherit_canonical_namespaces() {
        let text = json!({
            "models": {
                "vendor-a/model-v5": {"limit":{"context":100}},
                "vendor-b/model-v5": {"limit":{"context":200}}
            },
            "providers": {"relay":{"models":{"model-v5":{"tool_call":true}}}}
        }).to_string();
        let source = ModelsDevSource { store: CandidateStore::new(parse_catalog(&text).unwrap()) };
        assert_eq!(source.extract("model-v5").unwrap().facts.context_window, None);
        assert_eq!(source.extract("vendor-a/model-v5").unwrap().facts.context_window, Some(100));
    }

    /// 合成快照：`alpha-relay` / `beta-relay` 都定义 `shared-model`（值不同），
    /// `models` 里另有一个只属于模型清单的 ID，用来验证「分组优先、清单补缺」的顺序。
    fn synthetic_catalog() -> String {
        json!({
            "models": {
                "canonical-only-model": { "limit": { "context": 4096 }, "tool_call": true },
                "shared-model": { "limit": { "context": 999 }, "tool_call": true }
            },
            "providers": {
                "alpha-relay": {
                    "models": { "shared-model": { "limit": { "context": 100 }, "tool_call": true } }
                },
                "beta-relay": {
                    "models": { "shared-model": { "limit": { "context": 200 }, "tool_call": true } }
                }
            }
        })
        .to_string()
    }

    fn write_cache_file(dir: &std::path::Path, text: &str) {
        std::fs::create_dir_all(dir.join("cache")).unwrap();
        std::fs::write(cache_path(dir), text).unwrap();
    }

    fn has_id(items: &[(Candidate, Value)], id: &str) -> bool {
        items
            .iter()
            .any(|(_, payload)| payload.get("id").and_then(Value::as_str) == Some(id))
    }

    #[test]
    fn bundled_snapshot_is_full_and_usable() {
        // 快照本体仍是「全量 provider 分组 + 模型清单」两段
        let raw: Value = serde_json::from_str(BUNDLED_MODELS_DEV).unwrap();
        let providers = raw["providers"].as_object().unwrap();
        let provider_models: usize = providers
            .values()
            .map(|provider| {
                provider["models"]
                    .as_object()
                    .map_or(0, |models| models.len())
            })
            .sum();
        assert!(providers.len() >= 200, "内置快照应为全量（含中转商）");
        assert!(provider_models >= 7000, "内置快照模型数异常: {provider_models}");
        assert!(
            raw["models"].as_object().is_some_and(|models| models.len() >= 380),
            "内置快照缺少模型清单"
        );

        // 展平后的索引：分组唯一 ID + 模型清单补充的 ID
        let items = bundled_items();
        // 全量分组展开 = 3624 个唯一 ID，加上模型清单补充的 ID，约 3740 条
        assert!(items.len() >= 3700, "内置快照条目数异常: {}", items.len());
        for id in ["big-pickle", "hy3-preview-free", "longcat-2.0"] {
            assert!(has_id(items, id), "内置快照缺少分组模型 {id}");
        }
        assert!(
            has_id(items, "swiss-ai/apertus-8b"),
            "内置快照缺少只在模型清单里的条目"
        );
    }

    #[test]
    fn matching_only_depends_on_model_id() {
        let dir = tempdir().unwrap();
        // 本机代理无法按 base_url 命中任何分组，加载不再接受 base_url
        let source = ModelsDevSource::load(dir.path());

        let facts = source.extract("big-pickle").unwrap().facts;
        assert_eq!(facts.context_window, Some(200_000));
        assert_eq!(facts.max_context_window, Some(200_000));
        assert_eq!(facts.input_token_limit, Some(160_000));
        assert_eq!(facts.supports_tool_calls, Some(true));

        for id in ["hy3-preview-free", "longcat-2.0", "mimo-v2.5-free"] {
            assert!(source.extract(id).is_some(), "分组专有 ID {id} 应命中");
        }
    }

    #[test]
    fn duplicate_ids_prefer_canonical_fields() {
        let dir = tempdir().unwrap();
        write_cache_file(dir.path(), &synthetic_catalog());
        let source = ModelsDevSource::load(dir.path());

        let shared = source.extract("shared-model").unwrap();
        assert_eq!(shared.matched_id, "shared-model");
        assert_eq!(
            shared.facts.context_window,
            Some(999),
            "规范资料优先于提供方字典序"
        );
    }

    #[test]
    fn canonical_models_fill_ids_missing_from_provider_groups() {
        let dir = tempdir().unwrap();
        write_cache_file(dir.path(), &synthetic_catalog());
        let source = ModelsDevSource::load(dir.path());

        // 规范模型资料既补充独有 ID，也覆盖同模型的低可信字段。
        assert_eq!(
            source
                .extract("canonical-only-model")
                .unwrap()
                .facts
                .context_window,
            Some(4096)
        );
        assert_eq!(
            source.extract("shared-model").unwrap().facts.context_window,
            Some(999)
        );
    }

    #[test]
    fn extract_resolves_versionless_family_to_latest_base_release() {
        let dir = tempdir().unwrap();
        let source = ModelsDevSource::load(dir.path());
        let extracted = source.extract("deepseek-flash").unwrap();
        let facts = extracted.facts;
        assert!(
            extracted.matched_id.contains("deepseek") && extracted.matched_id.contains("flash"),
            "实际命中 {}",
            extracted.matched_id
        );
        assert!(facts.context_window.is_some());
        assert!(facts
            .reasoning_levels
            .as_ref()
            .is_some_and(|levels| levels.iter().any(|level| level == "low")));
        assert_eq!(facts.supports_reasoning, Some(true));
        assert!(facts
            .description
            .as_deref()
            .is_some_and(|description| description.contains("DeepSeek")));
        // models.dev 不提供 verbosity / 搜索能力，保持沉默交给其它源或模板
        assert_eq!(facts.support_verbosity, None);
        assert_eq!(facts.supports_search_tool, None);
    }

    #[test]
    fn bundled_matching_inherits_latest_compatible_model_family() {
        let dir = tempdir().unwrap();

        let openai = ModelsDevSource::load(dir.path());
        let gpt = openai.extract("gpt-5.7").unwrap();
        assert_eq!(gpt.matched_id, "gpt-5.6");
        assert_eq!(gpt.kind, crate::codex::model_catalog::matching::MatchKind::Fuzzy);
        assert_eq!(gpt.facts.input_token_limit, Some(922_000));
        assert_eq!(gpt.facts.supports_tool_calls, Some(true));

        let qwen = openai.extract("qwen4-coder").unwrap();
        // 全局索引里的 Qwen 条目可能带供应商前缀（如 qwen/qwen3-coder），只要求命中 Coder 规格
        assert!(qwen.matched_id.contains("qwen3-coder"));

        let deepseek = openai.extract("deepseek-v5").unwrap();
        assert!(
            deepseek.matched_id.to_ascii_lowercase().starts_with("deepseek-v4"),
            "实际命中 {}",
            deepseek.matched_id
        );
        assert!(!deepseek.matched_id.contains("r1"));
    }

    #[test]
    fn extracts_tool_capability_and_model_status() {
        let model = json!({
            "limit": { "context": 200000, "input": 128000 },
            "tool_call": false,
            "status": "beta"
        });
        let facts = facts_from_model(&model);
        assert_eq!(facts.context_window, Some(200_000));
        assert_eq!(facts.input_token_limit, Some(128_000));
        assert_eq!(facts.supports_tool_calls, Some(false));
        assert_eq!(facts.status.as_deref(), Some("beta"));
    }

    #[test]
    fn cache_is_used_when_valid_and_legacy_api_json_is_expired() {
        let dir = tempdir().unwrap();
        write_cache_file(dir.path(), &synthetic_catalog());
        let cached = ModelsDevSource::load(dir.path());
        assert_eq!(
            cached.extract("shared-model").unwrap().facts.context_window,
            Some(999)
        );
        assert!(!needs_refresh(dir.path()), "新鲜且合法的缓存不应重新下载");

        // 旧版 api.json（顶层直接是 provider 分组）缺少 providers 键 → 视为过期
        let legacy = json!({
            "customrelay": {
                "id": "customrelay",
                "api": "https://api.customrelay.example/v1",
                "models": { "relay-model": { "id": "relay-model", "limit": { "context": 4242 } } }
            }
        })
        .to_string();
        write_cache_file(dir.path(), &legacy);
        assert!(needs_refresh(dir.path()), "旧版 api.json 缓存应视为过期");
        let fallback = ModelsDevSource::load(dir.path());
        assert!(fallback.extract("relay-model").is_none());
        assert!(
            fallback.extract("big-pickle").is_some(),
            "旧缓存应回退内置快照"
        );

        write_cache_file(dir.path(), "{ broken");
        let fallback = ModelsDevSource::load(dir.path());
        assert!(fallback.extract("big-pickle").is_some(), "损坏缓存应回退内置快照");
    }

    #[test]
    fn store_response_rejects_invalid_payload() {
        let dir = tempdir().unwrap();
        assert!(store_response(dir.path(), "{ broken").is_err());
        assert!(!cache_path(dir.path()).exists());

        // 旧形态（没有 providers 键）同样拒绝写入
        let legacy = json!({
            "customrelay": { "models": { "relay-model": { "id": "relay-model" } } }
        })
        .to_string();
        assert!(store_response(dir.path(), &legacy).is_err());
        assert!(!cache_path(dir.path()).exists());

        assert!(store_response(dir.path(), &synthetic_catalog()).is_ok());
        assert!(cache_path(dir.path()).exists());
    }

    #[test]
    fn needs_refresh_follows_ttl_and_content_validity() {
        let dir = tempdir().unwrap();
        let path = cache_path(dir.path());
        let valid = synthetic_catalog();

        // 没有缓存文件 → 需要下载
        assert!(needs_refresh(dir.path()));

        // 新鲜且合法 → 24 小时内不重复下载
        super::super::test_support::write_cache(&path, &valid, Duration::from_secs(60 * 60));
        assert!(!needs_refresh(dir.path()));

        // 新鲜但损坏 → 视为过期，需要重新下载
        super::super::test_support::write_cache(&path, "{ broken", Duration::from_secs(60 * 60));
        assert!(needs_refresh(dir.path()));

        // 超过 24 小时 → 需要重新下载
        super::super::test_support::write_cache(&path, &valid, Duration::from_secs(25 * 60 * 60));
        assert!(needs_refresh(dir.path()));
    }
}
