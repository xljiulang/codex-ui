//! 模型提供方目录生成：读取提供方 `/models`，用 OpenRouter 元数据补全模型能力，
//! 并基于 codex 0.149.0 的 gpt-5.2 模板生成 `model_catalog_json` 内容。

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Map, Value};

const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";
const TEMPLATE_JSON: &str = include_str!("../../resources/model_catalog_template.json");
const BUNDLED_OPENROUTER_MODELS: &str = include_str!("../../resources/openrouter-models.json");
const FUZZY_MATCH_THRESHOLD: f64 = 0.72;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ModelCatalogModelOption {
    pub id: String,
    pub display_name: String,
    pub matched: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ModelCatalogGenerateResult {
    pub catalog: String,
    pub total: usize,
    pub matched: usize,
    pub skipped: usize,
    pub models: Vec<ModelCatalogModelOption>,
}

#[derive(Debug, Clone)]
struct SourceModel {
    id: String,
    canonical_slug: String,
    value: Value,
}

/// 从模型提供方的 `/models` 获取模型 ID，并用 OpenRouter 元数据生成目录。
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
    let sources = load_openrouter_models(app_dir)?;
    let template = template()?;
    build_catalog_result(&ids, &sources, template)
}

fn build_catalog_result(
    ids: &[String],
    sources: &[SourceModel],
    template: &Map<String, Value>,
) -> Result<ModelCatalogGenerateResult, String> {
    let (entries, models) = build_catalog_entries(ids, sources, template)?;
    if entries.is_empty() {
        return Err(format!(
            "模型提供者返回的 {} 个模型均未匹配到 OpenRouter 资料，未生成模型目录",
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

/// 启动时后台刷新 OpenRouter 模型缓存；失败时静默保留旧缓存/内置资源。
pub async fn refresh_openrouter_models_cache(app_dir: &Path) {
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
    let _ = store_openrouter_models_response(app_dir, &text);
}

fn template() -> Result<&'static Map<String, Value>, String> {
    static TEMPLATE: OnceLock<Map<String, Value>> = OnceLock::new();
    if let Some(value) = TEMPLATE.get() {
        return Ok(value);
    }
    let parsed: Value = serde_json::from_str(TEMPLATE_JSON)
        .map_err(|e| format!("内置模型目录模板不是合法 JSON: {e}"))?;
    let object = parsed
        .as_object()
        .cloned()
        .ok_or_else(|| "内置模型目录模板不是 JSON 对象".to_string())?;
    let _ = TEMPLATE.set(object);
    TEMPLATE
        .get()
        .ok_or_else(|| "内置模型目录模板初始化失败".to_string())
}

fn cache_path(app_dir: &Path) -> PathBuf {
    app_dir.join("cache").join("openrouter-models.json")
}

fn load_openrouter_models(app_dir: &Path) -> Result<Vec<SourceModel>, String> {
    if let Ok(text) = std::fs::read_to_string(cache_path(app_dir)) {
        if let Ok(models) = parse_openrouter_models(&text) {
            return Ok(models);
        }
    }
    parse_openrouter_models(BUNDLED_OPENROUTER_MODELS)
}

fn parse_openrouter_models(text: &str) -> Result<Vec<SourceModel>, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| format!("OpenRouter 模型数据不是合法 JSON: {e}"))?;
    let mut models = Vec::new();
    for key in ["data", "models"] {
        let Some(items) = value.get(key).and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            let Some(id) = item
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
            else {
                continue;
            };
            let canonical_slug = item
                .get("canonical_slug")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            models.push(SourceModel {
                id: id.to_string(),
                canonical_slug,
                value: item.clone(),
            });
        }
    }
    if models.is_empty() {
        return Err("OpenRouter 模型数据为空或缺少 id".to_string());
    }
    Ok(models)
}

fn store_openrouter_models_response(app_dir: &Path, text: &str) -> Result<(), String> {
    parse_openrouter_models(text)?;
    write_atomic(&cache_path(app_dir), text.as_bytes())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建 OpenRouter 缓存目录失败: {e}"))?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut file =
            std::fs::File::create(&tmp).map_err(|e| format!("写入 OpenRouter 缓存失败: {e}"))?;
        file.write_all(bytes)
            .map_err(|e| format!("写入 OpenRouter 缓存失败: {e}"))?;
        file.sync_all()
            .map_err(|e| format!("同步 OpenRouter 缓存失败: {e}"))?;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("替换 OpenRouter 缓存失败: {e}"));
    }
    Ok(())
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

fn build_catalog_entries(
    ids: &[String],
    sources: &[SourceModel],
    template: &Map<String, Value>,
) -> Result<(Vec<Value>, Vec<ModelCatalogModelOption>), String> {
    let mut entries = Vec::new();
    let mut models = Vec::with_capacity(ids.len());
    for id in ids {
        let display_name = display_name_from_slug(id);
        let Some(source) = match_source_model(id, sources) else {
            models.push(ModelCatalogModelOption {
                id: id.clone(),
                display_name,
                matched: false,
            });
            continue;
        };
        entries.push(build_catalog_entry(
            id,
            source,
            template,
            entries.len() + 1,
        )?);
        models.push(ModelCatalogModelOption {
            id: id.clone(),
            display_name,
            matched: true,
        });
    }
    Ok((entries, models))
}

fn match_source_model<'a>(model_id: &str, sources: &'a [SourceModel]) -> Option<&'a SourceModel> {
    if let Some(source) = sources.iter().find(|source| source.id == model_id) {
        return Some(source);
    }
    if let Some(source) = sources
        .iter()
        .find(|source| !source.canonical_slug.is_empty() && source.canonical_slug == model_id)
    {
        return Some(source);
    }

    let normalized = normalize_model_key(model_id);
    if normalized.is_empty() {
        return None;
    }
    if let Some(source) = sources
        .iter()
        .find(|source| normalize_model_key(&source.id) == normalized)
    {
        return Some(source);
    }
    if let Some(source) = sources.iter().find(|source| {
        !source.canonical_slug.is_empty()
            && normalize_model_key(&source.canonical_slug) == normalized
    }) {
        return Some(source);
    }

    let mut best: Option<(&SourceModel, f64)> = None;
    for source in sources {
        let source_key = normalize_model_key(&source.id);
        let canonical_key = normalize_model_key(&source.canonical_slug);
        let score =
            similarity(&normalized, &source_key).max(similarity(&normalized, &canonical_key));
        if score >= FUZZY_MATCH_THRESHOLD
            && best
                .map(|(_, best_score)| score > best_score)
                .unwrap_or(true)
        {
            best = Some((source, score));
        }
    }
    best.map(|(source, _)| source)
}

fn normalize_model_key(input: &str) -> String {
    let mut key = input
        .trim()
        .to_ascii_lowercase()
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string();
    loop {
        let before = key.clone();
        for suffix in [
            ":free",
            ":batch",
            ":extended",
            "-free",
            "-latest",
            "-preview",
            "-beta",
            "-alpha",
        ] {
            if let Some(stripped) = key.strip_suffix(suffix) {
                key = stripped.to_string();
            }
        }
        if key == before {
            break;
        }
    }
    key = strip_trailing_date(&key).to_string();
    key.replace('_', "-").trim_matches('-').to_string()
}

fn strip_trailing_date(key: &str) -> &str {
    if let Some((prefix, last)) = key.rsplit_once('-') {
        if last.len() == 8 && last.bytes().all(|b| b.is_ascii_digit()) {
            return prefix;
        }
    }
    if key.len() >= 11 {
        let start = key.len() - 11;
        let candidate = &key[start..];
        if candidate.starts_with('-')
            && candidate[1..]
                .bytes()
                .filter(|b| *b != b'-')
                .all(|b| b.is_ascii_digit())
            && candidate[1..].bytes().filter(|b| *b == b'-').count() == 2
        {
            return &key[..start];
        }
    }
    key
}

fn similarity(left: &str, right: &str) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    if left == right {
        return 1.0;
    }
    let left_chars: Vec<char> = left.chars().collect();
    let right_chars: Vec<char> = right.chars().collect();
    let distance = levenshtein(&left_chars, &right_chars) as f64;
    let max_len = left_chars.len().max(right_chars.len()) as f64;
    let edit_score = 1.0 - distance / max_len;
    edit_score.max(token_overlap(left, right))
}

fn levenshtein(left: &[char], right: &[char]) -> usize {
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0usize; right.len() + 1];
    for (i, left_char) in left.iter().enumerate() {
        current[0] = i + 1;
        for (j, right_char) in right.iter().enumerate() {
            let cost = usize::from(left_char != right_char);
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

fn token_overlap(left: &str, right: &str) -> f64 {
    let left_tokens: HashSet<&str> = left
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect();
    let right_tokens: HashSet<&str> = right
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect();
    let union = left_tokens.union(&right_tokens).count();
    if union == 0 {
        return 0.0;
    }
    left_tokens.intersection(&right_tokens).count() as f64 / union as f64
}

fn build_catalog_entry(
    model_id: &str,
    source: &SourceModel,
    template: &Map<String, Value>,
    priority: usize,
) -> Result<Value, String> {
    let mut entry = template.clone();
    let source_value = &source.value;

    entry.insert("slug".to_string(), json!(model_id));
    entry.insert(
        "display_name".to_string(),
        json!(display_name_from_slug(model_id)),
    );
    let description = source_value
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .or_else(|| {
            template
                .get("description")
                .and_then(Value::as_str)
                .filter(|description| !description.is_empty())
        })
        .unwrap_or("由模型提供者目录生成");
    entry.insert("description".to_string(), json!(description));

    if let Some(context_window) = source_value
        .get("context_length")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
    {
        entry.insert("context_window".to_string(), json!(context_window));
        entry.insert("max_context_window".to_string(), json!(context_window));
    }
    entry.insert(
        "input_modalities".to_string(),
        Value::Array(input_modalities(source_value)),
    );

    let (reasoning_levels, default_reasoning) = reasoning_levels(source_value, template);
    entry.insert(
        "supported_reasoning_levels".to_string(),
        Value::Array(reasoning_levels),
    );
    if let Some(default_reasoning) = default_reasoning {
        entry.insert(
            "default_reasoning_level".to_string(),
            json!(default_reasoning),
        );
    } else {
        entry.remove("default_reasoning_level");
    }
    entry.insert(
        "supports_reasoning_summary_parameter".to_string(),
        json!(reasoning_present(source_value)),
    );

    let supported_parameters = source_value
        .get("supported_parameters")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_ascii_lowercase)
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    entry.insert(
        "support_verbosity".to_string(),
        json!(supported_parameters.contains("verbosity")),
    );
    entry.remove("default_verbosity");
    entry.insert(
        "supports_search_tool".to_string(),
        json!(
            supported_parameters.contains("web_search_options")
                || supported_parameters.contains("web_search")
        ),
    );

    entry.insert("visibility".to_string(), json!("list"));
    entry.insert("supported_in_api".to_string(), json!(true));
    entry.insert("priority".to_string(), json!(priority as i64));
    entry.insert("additional_speed_tiers".to_string(), json!([]));
    entry.insert("service_tiers".to_string(), json!([]));
    entry.insert("default_service_tier".to_string(), Value::Null);
    entry.insert("availability_nux".to_string(), Value::Null);
    entry.insert("upgrade".to_string(), Value::Null);
    Ok(Value::Object(entry))
}

fn display_name_from_slug(slug: &str) -> String {
    let base = slug.rsplit('/').next().unwrap_or(slug).trim();
    let mut display_name = String::with_capacity(base.len());
    let mut uppercase_next = true;
    for ch in base.chars() {
        if ch.is_ascii_alphabetic() {
            if uppercase_next {
                display_name.push(ch.to_ascii_uppercase());
            } else {
                display_name.push(ch);
            }
            uppercase_next = false;
        } else {
            display_name.push(ch);
            uppercase_next = matches!(ch, '-' | '_' | ':');
        }
    }
    display_name
}

fn input_modalities(source_value: &Value) -> Vec<Value> {
    let mut modalities: Vec<Value> = Vec::new();
    if let Some(items) = source_value
        .get("architecture")
        .and_then(|architecture| architecture.get("input_modalities"))
        .and_then(Value::as_array)
    {
        for item in items {
            let Some(modality) = item.as_str().map(str::to_ascii_lowercase) else {
                continue;
            };
            if matches!(modality.as_str(), "text" | "image" | "audio")
                && !modalities
                    .iter()
                    .any(|existing| existing.as_str() == Some(modality.as_str()))
            {
                modalities.push(json!(modality));
            }
        }
    }
    if modalities.is_empty() {
        modalities.push(json!("text"));
    }
    modalities
}

fn reasoning_levels(
    source_value: &Value,
    template: &Map<String, Value>,
) -> (Vec<Value>, Option<String>) {
    let template_levels = template
        .get("supported_reasoning_levels")
        .and_then(Value::as_array);
    let mut levels = Vec::new();
    let mut seen = HashSet::new();
    if let Some(items) = source_value
        .get("reasoning")
        .and_then(|reasoning| reasoning.get("supported_efforts"))
        .and_then(Value::as_array)
    {
        for item in items {
            let Some(effort) = item.as_str().map(str::to_ascii_lowercase) else {
                continue;
            };
            if !is_known_reasoning_effort(&effort) || !seen.insert(effort.clone()) {
                continue;
            }
            let description = template_levels
                .and_then(|levels| {
                    levels.iter().find(|level| {
                        level.get("effort").and_then(Value::as_str) == Some(effort.as_str())
                    })
                })
                .and_then(|level| level.get("description"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("{effort} reasoning effort"));
            levels.push(json!({
                "effort": effort,
                "description": description,
            }));
        }
    }
    let default_reasoning = source_value
        .get("reasoning")
        .and_then(|reasoning| reasoning.get("default_effort"))
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
        .filter(|effort| {
            levels
                .iter()
                .any(|level| level.get("effort").and_then(Value::as_str) == Some(effort.as_str()))
        });
    (levels, default_reasoning)
}

fn reasoning_present(source_value: &Value) -> bool {
    let Some(reasoning) = source_value.get("reasoning") else {
        return false;
    };
    reasoning
        .get("mandatory")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || reasoning
            .get("supported_efforts")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty())
        || reasoning
            .get("default_effort")
            .and_then(Value::as_str)
            .is_some_and(|effort| !effort.is_empty())
}

fn is_known_reasoning_effort(effort: &str) -> bool {
    matches!(
        effort,
        "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
    )
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

    fn source(id: &str, canonical_slug: &str, value: Value) -> SourceModel {
        SourceModel {
            id: id.to_string(),
            canonical_slug: canonical_slug.to_string(),
            value,
        }
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
    fn normalize_model_key_removes_prefix_suffix_and_date() {
        assert_eq!(normalize_model_key("xiaomi/mimo-v2.5-free"), "mimo-v2.5");
        assert_eq!(
            normalize_model_key("xiaomi/mimo-v2.5-20260422"),
            "mimo-v2.5"
        );
        assert_eq!(
            normalize_model_key("google/gemini-3.8-flash:batch"),
            "gemini-3.8-flash"
        );
    }

    #[test]
    fn display_name_from_slug_capitalizes_segments() {
        assert_eq!(display_name_from_slug("mimo-v2.5-free"), "Mimo-V2.5-Free");
        assert_eq!(
            display_name_from_slug("xiaomi/mimo-v2.5-free"),
            "Mimo-V2.5-Free"
        );
        assert_eq!(display_name_from_slug("gpt-5.2-codex"), "Gpt-5.2-Codex");
        assert_eq!(display_name_from_slug("foo_bar:baz"), "Foo_Bar:Baz");
        assert_eq!(display_name_from_slug(""), "");
    }

    #[test]
    fn template_available_in_plans_is_empty() {
        let template = template().unwrap();
        assert_eq!(template.get("available_in_plans"), Some(&json!([])));
    }

    #[test]
    fn match_source_model_prefers_exact_then_normalized() {
        let sources = vec![
            source("xiaomi/mimo-v2.5-pro", "", json!({})),
            source("xiaomi/mimo-v2.5", "xiaomi/mimo-v2.5-20260422", json!({})),
        ];
        assert_eq!(
            match_source_model("xiaomi/mimo-v2.5", &sources).unwrap().id,
            "xiaomi/mimo-v2.5"
        );
        assert_eq!(
            match_source_model("mimo-v2.5-free", &sources).unwrap().id,
            "xiaomi/mimo-v2.5"
        );
    }

    #[test]
    fn build_catalog_entry_maps_source_metadata() {
        let template = template().unwrap();
        let source = source(
            "xiaomi/mimo-v2.5",
            "",
            json!({
                "name": "Xiaomi: MiMo-V2.5",
                "description": "Omnimodal model",
                "context_length": 1050000,
                "architecture": { "input_modalities": ["text", "audio", "image", "video"] },
                "reasoning": {
                    "mandatory": false,
                    "supported_efforts": ["high", "medium"],
                    "default_effort": "high"
                },
                "supported_parameters": ["verbosity", "web_search_options"]
            }),
        );
        let entry = build_catalog_entry("mimo-v2.5-free", &source, template, 1).unwrap();
        assert_eq!(entry["slug"], "mimo-v2.5-free");
        assert_eq!(entry["display_name"], "Mimo-V2.5-Free");
        assert_eq!(entry["available_in_plans"], json!([]));
        assert_eq!(entry["context_window"], 1050000);
        assert_eq!(entry["input_modalities"], json!(["text", "audio", "image"]));
        assert_eq!(entry["supported_reasoning_levels"][0]["effort"], "high");
        assert_eq!(entry["default_reasoning_level"], "high");
        assert_eq!(entry["support_verbosity"], true);
        assert_eq!(entry["supports_search_tool"], true);
        assert_eq!(entry["visibility"], "list");
        assert_eq!(entry["priority"], 1);
        assert!(entry["base_instructions"]
            .as_str()
            .unwrap()
            .starts_with("You are a coding agent running in the Codex CLI"));
        assert!(!entry["base_instructions"]
            .as_str()
            .unwrap()
            .contains("GPT-5.2"));
    }

    #[test]
    fn build_catalog_entries_skips_unmatched_models() {
        let template = template().unwrap();
        let sources = vec![source(
            "xiaomi/mimo-v2.5",
            "",
            json!({ "name": "MiMo", "context_length": 1000 }),
        )];
        let ids = vec!["mimo-v2.5-free".to_string(), "unknown-model".to_string()];
        let (entries, models) = build_catalog_entries(&ids, &sources, template).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["slug"], "mimo-v2.5-free");
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "mimo-v2.5-free");
        assert!(models[0].matched);
        assert_eq!(models[1].id, "unknown-model");
        assert_eq!(models[1].display_name, "Unknown-Model");
        assert!(!models[1].matched);
    }

    #[test]
    fn build_catalog_result_returns_candidates_and_matched_catalog() {
        let template = template().unwrap();
        let sources = vec![
            source(
                "deepseek/deepseek-chat",
                "",
                json!({ "context_length": 64000 }),
            ),
            source("xiaomi/mimo-v2.5", "", json!({ "context_length": 1000 })),
        ];
        let ids = vec![
            "deepseek-chat".to_string(),
            "unknown-model".to_string(),
            "mimo-v2.5-free".to_string(),
        ];
        let result = build_catalog_result(&ids, &sources, template).unwrap();
        assert_eq!(result.total, 3);
        assert_eq!(result.matched, 2);
        assert_eq!(result.skipped, 1);
        assert_eq!(result.models[0].id, "deepseek-chat");
        assert_eq!(result.models[0].display_name, "Deepseek-Chat");
        assert!(result.models[0].matched);
        assert_eq!(result.models[1].id, "unknown-model");
        assert!(!result.models[1].matched);
        assert_eq!(result.models[2].id, "mimo-v2.5-free");
        assert!(result.models[2].matched);

        let catalog: Value = serde_json::from_str(&result.catalog).unwrap();
        let entries = catalog["models"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["slug"], "deepseek-chat");
        assert_eq!(entries[1]["slug"], "mimo-v2.5-free");
    }

    #[test]
    fn build_catalog_result_errors_when_all_models_are_unmatched() {
        let template = template().unwrap();
        let sources = vec![source("xiaomi/mimo-v2.5", "", json!({}))];
        let ids = vec!["unknown-model".to_string()];
        let error = build_catalog_result(&ids, &sources, template).unwrap_err();
        assert!(error.contains("均未匹配到 OpenRouter 资料"));
    }

    #[test]
    fn cache_store_replaces_valid_json_and_rejects_invalid_json() {
        let dir = tempdir().unwrap();
        let valid = r#"{"data":[{"id":"a"}]}"#;
        store_openrouter_models_response(dir.path(), valid).unwrap();
        assert_eq!(
            std::fs::read_to_string(cache_path(dir.path())).unwrap(),
            valid
        );
        assert!(store_openrouter_models_response(dir.path(), r#"{"data":[]}"#).is_err());
        assert_eq!(
            std::fs::read_to_string(cache_path(dir.path())).unwrap(),
            valid
        );
    }
}
