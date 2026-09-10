//! models.dev 字段源（覆盖源）：官方厂商口径的上下文、输入模态、推理档位与描述。
//!
//! 形态与 <https://models.dev/api.json> 一致（顶层为 provider id）；内置快照与运行时缓存
//! 都是全量（含中转/聚合商，缓存 24 小时内不重复下载）。
//! 匹配时先按 `base_url` 定位 provider 分组（URL 前缀优先、主机名兜底），否则用全局索引；
//! 全局索引同 ID 冲突时优先官方厂商（清单见 `models-dev-official-providers.json`）。

use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde_json::{json, Value};

use super::{FactSource, write_atomic};
use crate::codex::model_catalog::facts::ModelFacts;
use crate::codex::model_catalog::matching::{Candidate, CandidateStore, release_from_date};

const MODELS_DEV_URL: &str = "https://models.dev/api.json";
const BUNDLED_MODELS_DEV: &str = include_str!("../../../../resources/models-dev.json");
const BUNDLED_OFFICIAL_PROVIDERS: &str =
    include_str!("../../../../resources/models-dev-official-providers.json");

struct ProviderEntry {
    id: String,
    api: Option<String>,
    models: Vec<Value>,
}

pub struct ModelsDevSource {
    scoped: CandidateStore,
    global: CandidateStore,
}

impl ModelsDevSource {
    pub fn load(app_dir: &Path, base_url: &str) -> Self {
        let cached = std::fs::read_to_string(cache_path(app_dir))
            .ok()
            .and_then(|text| parse_providers(&text).ok());
        let providers: &[ProviderEntry] = cached.as_deref().unwrap_or_else(|| bundled_providers());
        let scoped = scoped_provider(providers, base_url)
            .map(build_store)
            .unwrap_or_default();
        Self {
            scoped,
            global: build_global_store(providers),
        }
    }
}

impl FactSource for ModelsDevSource {
    fn id(&self) -> &'static str {
        "models_dev"
    }

    fn extract(&self, model_id: &str) -> Option<ModelFacts> {
        self.scoped
            .lookup(model_id)
            .or_else(|| self.global.lookup(model_id))
            .map(facts_from_model)
    }
}

/// 解析 provider 分组：只保留带模型的 provider。
fn parse_providers(text: &str) -> Result<Vec<ProviderEntry>, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| format!("models.dev 目录不是合法 JSON: {e}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "models.dev 目录不是 JSON 对象".to_string())?;

    let mut providers = Vec::new();
    for (provider_id, provider) in object {
        let Some(models) = provider.get("models").and_then(Value::as_object) else {
            continue;
        };
        let mut entries = Vec::new();
        for (model_id, model) in models {
            if !model.is_object() {
                continue;
            }
            let mut model = model.clone();
            if model.get("id").and_then(Value::as_str).is_none() {
                model["id"] = json!(model_id);
            }
            entries.push(model);
        }
        if entries.is_empty() {
            continue;
        }
        providers.push(ProviderEntry {
            id: provider_id.clone(),
            api: provider
                .get("api")
                .and_then(Value::as_str)
                .map(str::to_string),
            models: entries,
        });
    }

    if providers.is_empty() {
        return Err("models.dev 目录没有可用的 provider 分组".to_string());
    }
    Ok(providers)
}

/// 按 `base_url` 定位 provider 分组：
/// 1. 同源且路径前缀匹配（区分同主机不同产品线，如 `opencode` 与 `opencode-go`）；
/// 2. 退化为同主机匹配。
///
/// 命中多个时取路径最长者，再按 provider id 字典序，保证确定性。
fn scoped_provider<'a>(providers: &'a [ProviderEntry], base_url: &str) -> Option<&'a ProviderEntry> {
    let base = url_parts(base_url)?;
    let mut best: Option<(u8, usize, &'a ProviderEntry)> = None;
    for provider in providers {
        let Some(api) = provider.api.as_deref().and_then(url_parts) else {
            continue;
        };
        if api.host != base.host {
            continue;
        }
        let tier = u8::from(!(api.scheme == base.scheme && is_path_prefix(&api.path, &base.path)));
        let better = match best {
            None => true,
            Some((best_tier, best_path_len, best_provider)) => {
                tier.cmp(&best_tier)
                    .then_with(|| best_path_len.cmp(&api.path.len()))
                    .then_with(|| provider.id.cmp(&best_provider.id))
                    == Ordering::Less
            }
        };
        if better {
            best = Some((tier, api.path.len(), provider));
        }
    }
    best.map(|(_, _, provider)| provider)
}

fn build_store(provider: &ProviderEntry) -> CandidateStore {
    CandidateStore::new(build_pairs(provider))
}

fn build_pairs(provider: &ProviderEntry) -> Vec<(Candidate, Value)> {
    provider
        .models
        .iter()
        .filter_map(|model| {
            let id = model
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())?
                .to_string();
            let mut keys = vec![id.clone()];
            if let Some(family) = model
                .get("family")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|family| !family.is_empty())
            {
                keys.push(family.to_string());
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
            Some((Candidate::new(keys, release, id), model.clone()))
        })
        .collect()
}

/// 全局索引：同一模型 id 出现在多个 provider 时，官方厂商优先、其余按 provider id 字典序。
fn build_global_store(providers: &[ProviderEntry]) -> CandidateStore {
    let official = official_provider_ids();
    let mut order: Vec<&ProviderEntry> = providers.iter().collect();
    order.sort_by(|left, right| {
        official
            .contains(&right.id)
            .cmp(&official.contains(&left.id))
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut seen: HashSet<String> = HashSet::new();
    let mut items = Vec::new();
    for provider in order {
        for (candidate, payload) in build_pairs(provider) {
            let id = payload
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if !seen.insert(id) {
                continue;
            }
            items.push((candidate, payload));
        }
    }
    CandidateStore::new(items)
}

/// 官方厂商清单（由更新脚本产出，仅影响同 ID 冲突时的优先级）。
fn official_provider_ids() -> &'static HashSet<String> {
    static IDS: OnceLock<HashSet<String>> = OnceLock::new();
    IDS.get_or_init(|| {
        serde_json::from_str::<Vec<String>>(BUNDLED_OFFICIAL_PROVIDERS)
            .map(|ids| ids.into_iter().collect())
            .unwrap_or_default()
    })
}

/// 内联全量快照（约 2.8 MB）只解析一次进程复用。
fn bundled_providers() -> &'static [ProviderEntry] {
    static BUNDLED: OnceLock<Vec<ProviderEntry>> = OnceLock::new();
    BUNDLED
        .get_or_init(|| parse_providers(BUNDLED_MODELS_DEV).unwrap_or_default())
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
    facts.input_modalities = input_modalities(model);
    facts.reasoning_levels = reasoning_levels(model);
    facts.supports_reasoning = supports_reasoning(model);
    facts.description = model
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .map(str::to_string);
    facts
}

fn input_modalities(model: &Value) -> Option<Vec<String>> {
    let items = model
        .pointer("/modalities/input")
        .and_then(Value::as_array)?;
    let mut modalities = Vec::new();
    for item in items {
        let Some(modality) = item.as_str().map(str::to_ascii_lowercase) else {
            continue;
        };
        if matches!(modality.as_str(), "text" | "image" | "audio")
            && !modalities.iter().any(|existing| existing == &modality)
        {
            modalities.push(modality);
        }
    }
    (!modalities.is_empty()).then_some(modalities)
}

/// `reasoning_options` 里的 `{ type: "effort", values: [...] }` → 推理档位。
fn reasoning_levels(model: &Value) -> Option<Vec<String>> {
    let options = model.get("reasoning_options").and_then(Value::as_array)?;
    let mut levels = Vec::new();
    for option in options {
        if option.get("type").and_then(Value::as_str) != Some("effort") {
            continue;
        }
        let Some(values) = option.get("values").and_then(Value::as_array) else {
            continue;
        };
        for value in values {
            let Some(effort) = value.as_str().map(str::to_ascii_lowercase) else {
                continue;
            };
            if !levels.iter().any(|existing| existing == &effort) {
                levels.push(effort);
            }
        }
    }
    (!levels.is_empty()).then_some(levels)
}

fn supports_reasoning(model: &Value) -> Option<bool> {
    let reasoning = model.get("reasoning")?;
    let has_options = model
        .get("reasoning_options")
        .and_then(Value::as_array)
        .is_some_and(|options| !options.is_empty());
    Some(reasoning.as_bool().unwrap_or(false) || has_options)
}

struct UrlParts {
    scheme: String,
    host: String,
    path: String,
}

/// 归一化 URL：scheme/host 小写、路径去尾斜杠（query/fragment 不参与比较）。
fn url_parts(value: &str) -> Option<UrlParts> {
    let url = reqwest::Url::parse(value.trim()).ok()?;
    Some(UrlParts {
        scheme: url.scheme().to_ascii_lowercase(),
        host: url.host_str()?.to_ascii_lowercase(),
        path: url.path().trim_end_matches('/').to_string(),
    })
}

/// `prefix` 是否等于 `full` 或在路径边界（`/`）处结束的前缀。
fn is_path_prefix(prefix: &str, full: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    full == prefix
        || (full.starts_with(prefix) && full.as_bytes().get(prefix.len()) == Some(&b'/'))
}

fn cache_path(app_dir: &Path) -> PathBuf {
    app_dir.join("cache").join("models-dev.json")
}

/// 是否需要重新下载：缓存缺失、超过 24 小时或内容不可解析（损坏视为过期）。
fn needs_refresh(app_dir: &Path) -> bool {
    !super::cache_is_reusable(&cache_path(app_dir), |text| {
        parse_providers(text).is_ok()
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
    parse_providers(text)?;
    write_atomic(&cache_path(app_dir), text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn bundled_snapshot_is_full_and_usable() {
        let providers = bundled_providers();
        assert!(providers.len() >= 200, "内置快照应为全量（含中转商）");
        let models: usize = providers.iter().map(|provider| provider.models.len()).sum();
        assert!(models >= 7000, "内置快照模型数异常: {models}");
        for id in ["deepseek", "opencode", "opencode-go", "zenmux"] {
            assert!(
                providers.iter().any(|provider| provider.id == id),
                "内置快照缺少 provider {id}"
            );
        }
    }

    #[test]
    fn scoping_uses_url_prefix_before_host() {
        let providers = bundled_providers();
        assert_eq!(
            scoped_provider(providers, "https://opencode.ai/zen/v1")
                .unwrap()
                .id,
            "opencode"
        );
        assert_eq!(
            scoped_provider(providers, "https://opencode.ai/zen/go/v1")
                .unwrap()
                .id,
            "opencode-go"
        );
        assert_eq!(
            scoped_provider(providers, "https://api.deepseek.com/v1")
                .unwrap()
                .id,
            "deepseek"
        );
        // 路径边界：`/zen/go` 不以 `/zen/v1` 为前缀，两者只能落到同主机兜底，取路径最长者
        assert_eq!(
            scoped_provider(providers, "https://opencode.ai/zen/go")
                .unwrap()
                .id,
            "opencode-go"
        );
        assert_eq!(
            scoped_provider(providers, "https://opencode.ai").unwrap().id,
            "opencode-go"
        );
    }

    #[test]
    fn relay_groups_are_reachable_without_cache() {
        let dir = tempdir().unwrap();
        let zen = ModelsDevSource::load(dir.path(), "https://opencode.ai/zen/v1");
        assert!(zen.extract("hy3-preview-free").is_some());
        let go = ModelsDevSource::load(dir.path(), "https://opencode.ai/zen/go/v1");
        assert!(go.extract("longcat-2.0").is_some());
    }

    #[test]
    fn official_provider_list_excludes_relays() {
        let ids = official_provider_ids();
        assert!(ids.contains("deepseek"));
        assert!(ids.contains("openai"));
        assert!(!ids.contains("opencode"));
        assert!(!ids.contains("zenmux"));
    }

    #[test]
    fn extract_resolves_versionless_family_to_latest_base_release() {
        let dir = tempdir().unwrap();
        let source = ModelsDevSource::load(dir.path(), "https://api.deepseek.com/v1");
        let facts = source.extract("deepseek-flash").unwrap();
        assert_eq!(facts.context_window, Some(1_000_000));
        assert_eq!(facts.max_context_window, Some(1_000_000));
        assert_eq!(facts.input_modalities, Some(vec!["text".to_string()]));
        assert_eq!(
            facts.reasoning_levels,
            Some(vec![
                "low".to_string(),
                "high".to_string(),
                "max".to_string()
            ])
        );
        assert_eq!(facts.supports_reasoning, Some(true));
        assert!(facts
            .description
            .as_deref()
            .is_some_and(|description| description.contains("Official DeepSeek V4 Flash")));
        // models.dev 不提供 verbosity / 搜索能力，保持沉默交给其它源或模板
        assert_eq!(facts.support_verbosity, None);
        assert_eq!(facts.supports_search_tool, None);
    }

    #[test]
    fn provider_host_scopes_matching_to_that_provider() {
        let dir = tempdir().unwrap();
        let text = json!({
            "alpha": {
                "id": "alpha",
                "api": "https://api.alpha.example/v1",
                "models": { "shared-model": { "id": "shared-model", "limit": { "context": 100 } } }
            },
            "beta": {
                "id": "beta",
                "api": "https://api.beta.example/v1",
                "models": { "shared-model": { "id": "shared-model", "limit": { "context": 200 } } }
            }
        })
        .to_string();
        std::fs::create_dir_all(dir.path().join("cache")).unwrap();
        std::fs::write(cache_path(dir.path()), text).unwrap();

        let alpha = ModelsDevSource::load(dir.path(), "https://api.alpha.example/v1");
        assert_eq!(alpha.extract("shared-model").unwrap().context_window, Some(100));
        let beta = ModelsDevSource::load(dir.path(), "https://api.beta.example/v1");
        assert_eq!(beta.extract("shared-model").unwrap().context_window, Some(200));
    }

    #[test]
    fn global_index_is_deterministic_for_duplicate_model_ids() {
        let text = json!({
            "zeta": {
                "id": "zeta",
                "api": "https://api.zeta.example/v1",
                "models": { "shared-model": { "id": "shared-model", "limit": { "context": 999 } } }
            },
            "alpha": {
                "id": "alpha",
                "api": "https://api.alpha.example/v1",
                "models": { "shared-model": { "id": "shared-model", "limit": { "context": 100 } } }
            }
        })
        .to_string();
        let providers = parse_providers(&text).unwrap();
        let store = build_global_store(&providers);
        let facts = facts_from_model(store.lookup("shared-model").unwrap());
        assert_eq!(facts.context_window, Some(100));
    }

    #[test]
    fn global_index_prefers_official_vendor_over_lexicographic_relay() {
        // 中转商 id 排在官方厂商之前，验证「官方优先」不是靠字典序碰巧成立
        let text = json!({
            "aaa-relay": {
                "id": "aaa-relay",
                "api": "https://api.aaa-relay.example/v1",
                "models": { "shared-model": { "id": "shared-model", "limit": { "context": 999 } } }
            },
            "deepseek": {
                "id": "deepseek",
                "api": "https://api.deepseek.com/v1",
                "models": { "shared-model": { "id": "shared-model", "limit": { "context": 100 } } }
            }
        })
        .to_string();
        let providers = parse_providers(&text).unwrap();
        let store = build_global_store(&providers);
        let facts = facts_from_model(store.lookup("shared-model").unwrap());
        assert_eq!(facts.context_window, Some(100), "官方厂商应优先于中转商");
    }

    #[test]
    fn cache_is_used_when_valid_and_ignored_when_broken() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("cache")).unwrap();
        let valid = json!({
            "customrelay": {
                "id": "customrelay",
                "api": "https://api.customrelay.example/v1",
                "models": { "relay-model": { "id": "relay-model", "limit": { "context": 4242 } } }
            }
        })
        .to_string();
        std::fs::write(cache_path(dir.path()), &valid).unwrap();
        let source = ModelsDevSource::load(dir.path(), "https://api.customrelay.example/v1");
        assert_eq!(source.extract("relay-model").unwrap().context_window, Some(4242));
        assert!(!needs_refresh(dir.path()), "新鲜且合法的缓存不应重新下载");

        std::fs::write(cache_path(dir.path()), "{ broken").unwrap();
        let source = ModelsDevSource::load(dir.path(), "https://api.customrelay.example/v1");
        assert!(source.extract("relay-model").is_none());
        assert!(source.extract("deepseek-flash").is_some(), "损坏缓存应回退内置快照");
    }

    #[test]
    fn store_response_rejects_invalid_payload() {
        let dir = tempdir().unwrap();
        assert!(store_response(dir.path(), "{ broken").is_err());
        assert!(!cache_path(dir.path()).exists());
    }

    #[test]
    fn needs_refresh_follows_ttl_and_content_validity() {
        let dir = tempdir().unwrap();
        let path = cache_path(dir.path());
        let valid = json!({
            "customrelay": {
                "id": "customrelay",
                "api": "https://api.customrelay.example/v1",
                "models": { "relay-model": { "id": "relay-model", "limit": { "context": 4242 } } }
            }
        })
        .to_string();

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
