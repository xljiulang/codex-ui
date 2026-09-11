//! 目录条目的渲染基底。
//!
//! **渲染模板 = 本机 codex 导出基底 + `resources/model_catalog_template.json` 覆盖**：
//! 基底取 `codex debug models --bundled` 的第一条（随用户安装的 codex 版本变化），
//! 保证生成条目的键集与该版本一致——实测缺少必需字段会让 codex 直接拒绝整份
//! `model_catalog_json`（`missing field display_name` / `missing field visibility`）。
//! 模板资源只声明固定值与占位值：模板里有同名键就用模板的值。
//! 代码只声明哪些键由字段源覆盖（[`DATA_DRIVEN_KEYS`]）以及四条派生规则。

use std::collections::HashMap;
#[cfg(test)]
use std::path::Path;
use std::sync::OnceLock;

use serde_json::{json, Map, Value};

#[cfg(test)]
use super::codex_models;
use super::facts::ModelFacts;

const TEMPLATE_JSON: &str = include_str!("../../../resources/model_catalog_template.json");
/// 无导出（老版本 codex 没有 `debug models`、或导出失败且无缓存）时的基底骨架：
/// 只含必需键与一份提示词，其余仍由模板资源覆盖。
const FALLBACK_BASE_JSON: &str = include_str!("../../../resources/model_catalog_fallback.json");
/// 基底里不并入模板的键：剥掉**基底**（本机 codex 导出 / 兜底骨架）的长
/// `base_instructions`，随后由模板资源用精简版覆盖。
const BASE_EXCLUDED_KEYS: &[&str] = &["base_instructions"];
/// 复用条目补键时跳过的键：`base_instructions` 必须保持条目自带的那份，否则命中
/// 完整条目源（codex 导出池 / `official-models.json`）的模型会被注入模板的精简基底。
const ENSURE_EXCLUDED_KEYS: &[&str] = &["base_instructions"];

/// 由字段源覆盖的键；模板里其余键一律原样继承。
///
/// `default_reasoning_summary` 按推理能力写入（推理模型 auto、其余 none）；
/// 0.149.x 的 `supports_reasoning_summary_parameter` 一律不写——官方条目没有该键，
/// 显式写 false 会让 codex 完全不请求推理摘要（GPT 系因此只回加密推理、思考过程空白）。
/// 该清单作为渲染契约由 `render_only_touches_data_driven_keys` 断言，仅测试期使用。
#[cfg(test)]
pub const DATA_DRIVEN_KEYS: &[&str] = &[
    "slug",
    "display_name",
    "description",
    "priority",
    "input_modalities",
    "supported_reasoning_levels",
    "default_reasoning_level",
    "default_reasoning_summary",
    "context_window",
    "max_context_window",
    "effective_context_window_percent",
    "auto_compact_token_limit",
    "support_verbosity",
    "default_verbosity",
    "supports_search_tool",
    "status",
    "supports_image_detail_original",
];

/// 源没有给出描述时的兜底文案。
const DESCRIPTION_FALLBACK: &str = "由模型提供者目录生成";

/// 覆盖清单（模板资源，已做 OnceLock 缓存）；非法时返回错误，避免静默产出残缺条目。
fn overrides() -> Result<&'static Map<String, Value>, String> {
    static TEMPLATE: OnceLock<Result<Map<String, Value>, String>> = OnceLock::new();
    let cached = TEMPLATE.get_or_init(|| {
        let parsed: Value = serde_json::from_str(TEMPLATE_JSON)
            .map_err(|e| format!("内置模型目录模板不是合法 JSON: {e}"))?;
        parsed
            .as_object()
            .cloned()
            .ok_or_else(|| "内置模型目录模板不是 JSON 对象".to_string())
    });
    cached.as_ref().map_err(Clone::clone)
}

/// 兜底基底骨架（已做 OnceLock 缓存）。
fn fallback_base() -> Result<&'static Map<String, Value>, String> {
    static FALLBACK: OnceLock<Result<Map<String, Value>, String>> = OnceLock::new();
    let cached = FALLBACK.get_or_init(|| {
        let parsed: Value = serde_json::from_str(FALLBACK_BASE_JSON)
            .map_err(|e| format!("内置模型目录兜底基底不是合法 JSON: {e}"))?;
        parsed
            .as_object()
            .cloned()
            .ok_or_else(|| "内置模型目录兜底基底不是 JSON 对象".to_string())
    });
    cached.as_ref().map_err(Clone::clone)
}

/// 渲染模板：条目基底（覆盖后的固定值）+ 档位描述表（来自基底，随本机 codex 版本）。
pub struct RenderTemplate {
    /// 合并后的条目映射：本机 codex 导出基底（无导出则兜底骨架）被模板资源覆盖同名键。
    pub entry: Map<String, Value>,
    /// `effort → description`：取自**覆盖前**的基底条目（模板把该键最小化成 `[]`，
    /// 但界面上档位选项的悬停提示仍应显示本机 codex 的官方文案）。
    level_descriptions: HashMap<String, String>,
}

impl RenderTemplate {
    /// 档位描述：基底表里有就用，没有则给一句兜底文案。
    fn describe_level(&self, effort: &str) -> String {
        self.level_descriptions
            .get(effort)
            .cloned()
            .unwrap_or_else(|| format!("{effort} reasoning effort"))
    }
}

/// 渲染模板：本机 codex 导出基底（无导出则兜底骨架）→ 模板资源覆盖同名键。
///
/// 生成期只读 [`codex_models::codex_snapshot`] 的进程内快照，不会再起 codex 子进程。
#[cfg(test)]
pub fn effective_template(app_dir: &Path) -> Result<RenderTemplate, String> {
    from_snapshot(&codex_models::codex_snapshot(app_dir))
}

pub fn from_snapshot(snapshot: &[Value]) -> Result<RenderTemplate, String> {
    let base = match snapshot.first() {
        Some(entry) => entry
            .as_object()
            .cloned()
            .ok_or_else(|| "codex 导出的模型条目不是 JSON 对象".to_string())?,
        None => fallback_base()?.clone(),
    };
    let level_descriptions = level_descriptions_of(&base);
    let mut merged = base;
    for key in BASE_EXCLUDED_KEYS {
        merged.remove(*key);
    }
    let overrides = overrides()?;
    // 提示词：模板必须同时给出两个字段（codex 对二者互为备选，都写才不依赖其优先级）
    let prompt = overrides
        .get("model_messages")
        .and_then(|messages| messages.get("instructions_template"))
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| "内置模型目录模板缺少 model_messages.instructions_template".to_string())?
        .to_string();
    if !overrides
        .get("base_instructions")
        .and_then(Value::as_str)
        .is_some_and(|text| !text.trim().is_empty())
    {
        return Err("内置模型目录模板缺少 base_instructions".to_string());
    }
    for (key, value) in overrides {
        // model_messages 单独合并：整对象替换会丢掉基底的 approvals /
        // collaboration_modes / instructions_variables / multi_agent / permissions
        if key.as_str() == "model_messages" {
            continue;
        }
        merged.insert(key.clone(), value.clone());
    }
    match merged.get_mut("model_messages") {
        Some(Value::Object(messages)) => {
            messages.insert("instructions_template".to_string(), json!(prompt));
        }
        _ => {
            merged.insert(
                "model_messages".to_string(),
                json!({ "instructions_template": prompt }),
            );
        }
    }
    Ok(RenderTemplate {
        entry: merged,
        level_descriptions,
    })
}

/// 从条目里抽取 `supported_reasoning_levels` 的「档位 → 描述」表。
fn level_descriptions_of(entry: &Map<String, Value>) -> HashMap<String, String> {
    let mut descriptions = HashMap::new();
    let Some(levels) = entry
        .get("supported_reasoning_levels")
        .and_then(Value::as_array)
    else {
        return descriptions;
    };
    for level in levels {
        let Some(effort) = level.get("effort").and_then(Value::as_str) else {
            continue;
        };
        let Some(description) = level.get("description").and_then(Value::as_str) else {
            continue;
        };
        descriptions.insert(effort.trim().to_string(), description.to_string());
    }
    descriptions
}

/// 用模板值补齐条目缺失的键（只补不覆盖）。
///
/// 官方条目池里的条目（运行期导出或手写的第三方条目）按 slug 整条复用，
/// 这里保证它们也带上当前 codex 版本要求的全部键。
pub fn ensure_keys(entry: &mut Value, template: &Map<String, Value>) {
    let Some(object) = entry.as_object_mut() else {
        return;
    };
    for (key, value) in template {
        if ENSURE_EXCLUDED_KEYS.contains(&key.as_str()) {
            continue;
        }
        if !object.contains_key(key) {
            object.insert(key.clone(), value.clone());
        }
    }
}

/// 用字段源的合并结果渲染一个目录条目。
pub fn render(
    template: &RenderTemplate,
    model_id: &str,
    facts: &ModelFacts,
    priority: usize,
) -> Result<Value, String> {
    let mut entry = template.entry.clone();

    entry.insert("slug".to_string(), json!(model_id));
    entry.insert(
        "display_name".to_string(),
        json!(display_name_from_slug(model_id)),
    );
    entry.insert("priority".to_string(), json!(priority as i64));
    entry.insert(
        "description".to_string(),
        json!(facts
            .description
            .as_deref()
            .map(str::trim)
            .filter(|description| !description.is_empty())
            .unwrap_or(DESCRIPTION_FALLBACK)),
    );

    // 上下文：源有值才覆盖，否则保留模板占位（codex 对未知模型的 fallback 值）。
    if let Some(context_window) = facts.context_window.filter(|value| *value > 0) {
        entry.insert("context_window".to_string(), json!(context_window));
        entry.insert(
            "max_context_window".to_string(),
            json!(facts.max_context_window.unwrap_or(context_window)),
        );
        if let Some(input_limit) = facts
            .input_token_limit
            .filter(|input_limit| *input_limit > 0 && *input_limit < context_window)
        {
            let percent = ((i128::from(input_limit) * 100) / i128::from(context_window)).clamp(1, 95) as i64;
            entry.insert(
                "effective_context_window_percent".to_string(),
                json!(percent),
            );
            entry.insert(
                "auto_compact_token_limit".to_string(),
                json!((i128::from(input_limit) * 9 / 10) as i64),
            );
        }
    }

    // 输入模态：同时决定图片细节能力（纯文本模型不应声称支持 image detail original）。
    let modalities = facts.input_modalities.as_deref().map(normalize_modalities)
        .unwrap_or_else(|| vec!["text".to_string()]);
    entry.insert(
        "supports_image_detail_original".to_string(),
        json!(modalities.iter().any(|modality| modality == "image")),
    );
    entry.insert("input_modalities".to_string(), json!(modalities));

    // 推理档位与默认档位。
    let levels = normalize_reasoning_efforts(facts.reasoning_levels.as_deref().unwrap_or(&[]));
    let presets: Vec<Value> = levels
        .iter()
        .map(|effort| {
            json!({
                "effort": effort,
                "description": template.describe_level(effort),
            })
        })
        .collect();
    match resolve_default_reasoning_level(facts, &levels) {
        Some(default) => {
            entry.insert("default_reasoning_level".to_string(), json!(default));
        }
        None => {
            // 不删键：某些 codex 版本把该键当作必需字段，缺键会让整份目录解析失败
            entry.insert("default_reasoning_level".to_string(), Value::Null);
        }
    }
    entry.insert(
        "supported_reasoning_levels".to_string(),
        Value::Array(presets),
    );
    // 推理模型请求推理摘要：GPT 系只回加密推理 + 摘要，不请求就等于思考过程空白；
    // 非推理模型显式关闭。不写 `supports_reasoning_summary_parameter`（见 DATA_DRIVEN_KEYS 注释）。
    let supports_reasoning = facts.supports_reasoning == Some(true) || !levels.is_empty();
    entry.insert(
        "default_reasoning_summary".to_string(),
        json!(if supports_reasoning { "auto" } else { "none" }),
    );

    // 源明确给了 verbosity / 搜索能力才覆盖；源无信息时保留模板取值。
    match facts.support_verbosity {
        Some(true) => {
            entry.insert("support_verbosity".to_string(), json!(true));
            entry.insert("default_verbosity".to_string(), Value::Null);
        }
        Some(false) => {
            entry.insert("support_verbosity".to_string(), json!(false));
            entry.insert("default_verbosity".to_string(), Value::Null);
        }
        None => {}
    }
    if let Some(supports_search_tool) = facts.supports_search_tool {
        entry.insert(
            "supports_search_tool".to_string(),
            json!(supports_search_tool),
        );
    }
    if let Some(status) = facts
        .status
        .as_deref()
        .map(str::trim)
        .filter(|status| !status.is_empty())
    {
        entry.insert("status".to_string(), json!(status));
    }

    Ok(Value::Object(entry))
}

/// 按提供方返回的 slug 生成显示名（如 `deepseek-flash` → `DeepSeek-Flash`）。
pub fn display_name_from_slug(slug: &str) -> String {
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

fn normalize_modalities(modalities: &[String]) -> Vec<String> {
    let mut normalized: Vec<String> = Vec::new();
    for modality in modalities {
        let modality = modality.trim().to_ascii_lowercase();
        if !matches!(modality.as_str(), "text" | "image" | "audio") {
            continue;
        }
        if !normalized.iter().any(|existing| existing == &modality) {
            normalized.push(modality);
        }
    }
    normalized
}

fn normalize_reasoning_efforts(efforts: &[String]) -> Vec<String> {
    let mut filtered: Vec<String> = Vec::new();
    for effort in efforts {
        let effort = effort.trim().to_ascii_lowercase();
        if effort.is_empty() || matches!(effort.as_str(), "default" | "null") {
            continue;
        }
        if !filtered.iter().any(|existing| existing == &effort) {
            filtered.push(effort);
        }
    }
    filtered
}

fn resolve_default_reasoning_level(facts: &ModelFacts, levels: &[String]) -> Option<String> {
    if levels.is_empty() {
        return None;
    }
    if let Some(default) = facts.default_reasoning_level.as_deref() {
        let default = default.trim().to_ascii_lowercase();
        if levels.iter().any(|level| level == &default) {
            return Some(default);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// 无导出时的渲染模板：兜底基底 + 模板资源覆盖（与真实流程一致）
    fn effective_without_export() -> RenderTemplate {
        let dir = tempdir().unwrap();
        effective_template(dir.path()).unwrap()
    }

    fn rendered(facts: ModelFacts) -> Value {
        render(&effective_without_export(), "mimo-v2.5-free", &facts, 1).unwrap()
    }

    /// 模板资源持有的精简提示词（渲染路径条目的提示词来源）。
    fn third_party_prompt() -> String {
        overrides().unwrap()["model_messages"]["instructions_template"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn overrides_are_the_single_source_of_truth_for_fixed_values() {
        let overrides = overrides().unwrap();
        assert_eq!(overrides.len(), 44, "覆盖清单键数变化需同步更新文档与测试");
        assert_eq!(overrides["prefer_websockets"], json!(false));
        assert_eq!(overrides["web_search_tool_type"], json!("text"));
        assert_eq!(overrides["use_responses_lite"], json!(false));
        assert_eq!(overrides["tool_mode"], Value::Null);
        assert_eq!(overrides["truncation_policy"], json!({"mode":"tokens","limit":10000}));
        assert_eq!(overrides["multi_agent_version"], json!("v2"));
        assert_eq!(overrides["comp_hash"], json!("3000"));
        assert_eq!(overrides["minimal_client_version"], json!("0.144.0"));
        assert_eq!(overrides["reasoning_summary_format"], json!("experimental"));
        assert_eq!(overrides["default_reasoning_summary"], json!("auto"));
        assert_eq!(overrides["include_skills_usage_instructions"], json!(false));
        assert_eq!(overrides["include_plugin_usage_instructions"], json!(true));
        assert_eq!(overrides["include_apps_usage_instructions"], json!(true));
        assert_eq!(overrides["effective_context_window_percent"], json!(95));
        assert_eq!(overrides["supports_parallel_tool_calls"], json!(true));
        assert_eq!(overrides["shell_type"], json!("shell_command"));
        assert_eq!(overrides["apply_patch_tool_type"], json!("freeform"));
        assert_eq!(overrides["visibility"], json!("list"));
        assert_eq!(overrides["supported_in_api"], json!(true));
        assert_eq!(overrides["available_in_plans"], json!([]));
        assert_eq!(overrides["service_tiers"], json!([]));
        assert_eq!(overrides["additional_speed_tiers"], json!([]));
        assert_eq!(overrides["default_service_tier"], Value::Null);
        // 提示词由覆盖清单持有：两个字段同写，且是精简版（只服务未命中完整条目源的模型）
        assert_eq!(
            overrides["base_instructions"],
            overrides["model_messages"]["instructions_template"]
        );
        assert_eq!(
            overrides["model_messages"].as_object().unwrap().len(),
            1,
            "model_messages 只覆盖 instructions_template，其余子键跟随运行期基底"
        );
        let prompt = third_party_prompt();
        assert!(!prompt.trim().is_empty());
        assert!(
            prompt.chars().count() < 2_000,
            "精简提示词应远小于官方长提示词：{} 字符",
            prompt.chars().count()
        );
        // 不删键：缺值时写 null，避免某些版本把该键当必需字段
        assert_eq!(overrides["default_verbosity"], Value::Null);
        // 档位最小化：保留键但不带任何档位（描述表改由基底提供）
        assert_eq!(overrides["supported_reasoning_levels"], json!([]));
    }

    #[test]
    fn effective_template_falls_back_to_bundled_skeleton_without_export() {
        let template = effective_without_export();
        let prompt = third_party_prompt();
        // 基底（兜底骨架）的长提示词被模板值替换
        let instructions = template.entry["model_messages"]["instructions_template"]
            .as_str()
            .unwrap();
        assert_eq!(instructions, prompt);
        assert_eq!(template.entry["base_instructions"], json!(prompt));
        assert!(
            !instructions.starts_with("You are Codex"),
            "模板提示词应覆盖兜底骨架自带的官方长提示词"
        );
    }

    #[test]
    fn rendered_entries_carry_both_prompt_fields() {
        let entry = rendered(ModelFacts::default());
        let prompt = third_party_prompt();
        assert_eq!(entry["base_instructions"], json!(prompt));
        assert_eq!(
            entry["model_messages"]["instructions_template"],
            json!(prompt)
        );
    }

    #[test]
    fn effective_template_follows_codex_export_and_template_overrides() {
        let dir = tempdir().unwrap();
        codex_models::write_export_for_test(
            dir.path(),
            vec![json!({
                "slug": "gpt-5.6-sol",
                // 覆盖清单没有的键：必须跟随本机 codex（版本对齐的关键）
                "node_repl_disabled": true,
                "context_window": 400_000,
                // 提示词：模板值覆盖基底；但 model_messages 的其它子键跟随本机版本
                "model_messages": {
                    "instructions_template": "You are Codex (0.150)",
                    "approvals": { "from": "codex-export" }
                },
                // 覆盖清单里有的键：必须用模板值
                "prefer_websockets": true,
                "shell_type": "shell_command",
                "base_instructions": "should be dropped"
            })],
        );
        let template = effective_template(dir.path()).unwrap();
        let entry = &template.entry;

        assert_eq!(entry["node_repl_disabled"], json!(true));
        // 提示词不再跟随基底：渲染路径统一用模板里的精简提示词（两个字段同写）
        assert_eq!(
            entry["model_messages"]["instructions_template"],
            json!(third_party_prompt())
        );
        assert_eq!(entry["base_instructions"], json!(third_party_prompt()));
        // 基底的其余 model_messages 子键必须保留（整对象替换会丢）
        assert_eq!(
            entry["model_messages"]["approvals"],
            json!({ "from": "codex-export" })
        );
        assert_eq!(entry["prefer_websockets"], json!(false));
        assert_eq!(entry["context_window"], json!(272_000));
    }

    #[test]
    fn ensure_keys_fills_only_missing_keys() {
        let template = effective_without_export();
        let mut entry = serde_json::json!({
            "slug": "mimo-v2.5-free",
            "prefer_websockets": true,
            "supported_reasoning_levels": [{ "effort": "low", "description": "自有值" }]
        });
        ensure_keys(&mut entry, &template.entry);

        let object = entry.as_object().unwrap();
        assert_eq!(object["prefer_websockets"], json!(true), "已有值不得被覆盖");
        assert_eq!(object["slug"], json!("mimo-v2.5-free"));
        assert_eq!(
            object["supported_reasoning_levels"],
            json!([{ "effort": "low", "description": "自有值" }]),
            "已有档位不得被模板覆盖"
        );
        for key in template.entry.keys() {
            if ENSURE_EXCLUDED_KEYS.contains(&key.as_str()) {
                continue;
            }
            assert!(object.contains_key(key), "缺失键 {key} 应被补齐");
        }
        // 提示词例外：复用条目不得被注入模板的精简基底
        assert!(
            !object.contains_key("base_instructions"),
            "复用条目不应被补上模板的 base_instructions"
        );

        // 缺失该键时补进来的是最小化空数组，而不是基底的六档
        let mut bare = json!({ "slug": "x" });
        ensure_keys(&mut bare, &template.entry);
        assert_eq!(bare["supported_reasoning_levels"], json!([]));
    }

    #[test]
    fn merged_template_minimizes_levels_but_keeps_level_descriptions() {
        let template = effective_without_export();
        assert_eq!(template.entry["supported_reasoning_levels"], json!([]));
        // 描述表来自基底（无导出时是兜底资源里的 codex 基线表）
        for effort in ["low", "medium", "high", "xhigh", "max", "ultra"] {
            assert!(
                template.level_descriptions.contains_key(effort),
                "描述表缺少档位 {effort}"
            );
        }
        assert_eq!(
            template.level_descriptions.get("low").map(String::as_str),
            Some("Fast responses with lighter reasoning")
        );
    }

    #[test]
    fn render_writes_only_source_levels_with_base_descriptions() {
        // 字段源不给档位：生成条目写空数组（不继承模板/基底的档位）
        assert_eq!(rendered(ModelFacts::default())["supported_reasoning_levels"], json!([]));

        let facts = ModelFacts {
            reasoning_levels: Some(vec!["low".to_string(), "turbo".to_string()]),
            ..ModelFacts::default()
        };
        assert_eq!(
            rendered(facts)["supported_reasoning_levels"],
            json!([
                { "effort": "low", "description": "Fast responses with lighter reasoning" },
                { "effort": "turbo", "description": "turbo reasoning effort" }
            ])
        );
    }

    #[test]
    fn render_sets_identity_and_keeps_fixed_values_from_template() {
        let facts = ModelFacts {
            context_window: Some(1_000_000),
            description: Some("官方描述".to_string()),
            ..ModelFacts::default()
        };
        let entry = rendered(facts);
        assert_eq!(entry["slug"], json!("mimo-v2.5-free"));
        assert_eq!(entry["display_name"], json!("Mimo-V2.5-Free"));
        assert_eq!(entry["priority"], json!(1));
        assert_eq!(entry["description"], json!("官方描述"));
        assert_eq!(entry["context_window"], json!(1_000_000));
        assert_eq!(entry["max_context_window"], json!(1_000_000));
        assert_eq!(entry["prefer_websockets"], json!(false));
        assert_eq!(entry["truncation_policy"], json!({"mode":"tokens","limit":10000}));
        assert_eq!(entry["multi_agent_version"], json!("v2"));
        assert_eq!(entry["comp_hash"], json!("3000"));
    }

    #[test]
    fn render_keeps_template_placeholder_context_when_source_is_silent() {
        let entry = rendered(ModelFacts::default());
        assert_eq!(entry["context_window"], json!(272000));
        assert_eq!(entry["max_context_window"], json!(272000));
        assert_eq!(entry["input_modalities"], json!(["text"]));
        assert_eq!(entry["supports_image_detail_original"], json!(false));
        assert_eq!(entry["description"], json!(DESCRIPTION_FALLBACK));
    }

    #[test]
    fn render_links_image_detail_original_to_modalities() {
        let facts = ModelFacts {
            input_modalities: Some(vec!["image".to_string(), "text".to_string(), "video".to_string()]),
            ..ModelFacts::default()
        };
        let entry = rendered(facts);
        assert_eq!(entry["input_modalities"], json!(["image", "text"]));
        assert_eq!(entry["supports_image_detail_original"], json!(true));
    }

    #[test]
    fn explicit_empty_modalities_do_not_inherit_template_text() {
        let entry = rendered(ModelFacts { input_modalities: Some(vec![]), ..ModelFacts::default() });
        assert_eq!(entry["input_modalities"], json!([]));
        assert_eq!(entry["supports_image_detail_original"], json!(false));
    }

    #[test]
    fn render_keeps_custom_efforts_without_guessing_default_level() {
        let facts = ModelFacts {
            reasoning_levels: Some(vec![
                "low".to_string(),
                "turbo".to_string(),
                "high".to_string(),
            ]),
            ..ModelFacts::default()
        };
        let entry = rendered(facts);
        let levels = entry["supported_reasoning_levels"].as_array().unwrap();
        assert_eq!(levels.len(), 3);
        assert_eq!(levels[0]["effort"], json!("low"));
        // 描述取模板内置的档位描述表（写入前后者必被覆盖，不会泄漏到其它条目）
        assert_eq!(
            levels[0]["description"],
            json!("Fast responses with lighter reasoning")
        );
        assert_eq!(levels[1]["effort"], json!("turbo"));
        assert_eq!(
            levels[1]["description"],
            json!("turbo reasoning effort")
        );
        // 不删键：无默认档位时写 null
        assert_eq!(entry["default_reasoning_level"], Value::Null);
        // 有推理档位即请求摘要；且不写会挡掉摘要请求的旧字段
        assert_eq!(entry["default_reasoning_summary"], json!("auto"));
        assert!(entry.get("supports_reasoning_summary_parameter").is_none());
    }

    #[test]
    fn render_uses_default_effort_only_when_it_is_a_known_level() {
        let facts = ModelFacts {
            reasoning_levels: Some(vec!["low".to_string(), "high".to_string()]),
            default_reasoning_level: Some("high".to_string()),
            ..ModelFacts::default()
        };
        assert_eq!(rendered(facts)["default_reasoning_level"], json!("high"));

        let facts = ModelFacts {
            reasoning_levels: Some(vec!["low".to_string()]),
            default_reasoning_level: Some("max".to_string()),
            ..ModelFacts::default()
        };
        assert_eq!(rendered(facts)["default_reasoning_level"], Value::Null);
    }

    #[test]
    fn render_filters_only_reasoning_sentinel_values() {
        let facts = ModelFacts {
            reasoning_levels: Some(vec![
                "".to_string(),
                "default".to_string(),
                "null".to_string(),
                "turbo".to_string(),
            ]),
            ..ModelFacts::default()
        };
        let entry = rendered(facts);
        assert_eq!(
            entry["supported_reasoning_levels"],
            json!([{"effort":"turbo","description":"turbo reasoning effort"}])
        );
        assert_eq!(entry["default_reasoning_level"], Value::Null);
        assert_eq!(entry["default_reasoning_summary"], json!("auto"));
    }

    #[test]
    fn render_disables_reasoning_summary_for_non_reasoning_models() {
        let entry = rendered(ModelFacts::default());
        assert_eq!(entry["default_reasoning_summary"], json!("none"));
        assert!(entry.get("supports_reasoning_summary_parameter").is_none());
    }

    #[test]
    fn render_uses_conservative_verbosity_and_search_defaults() {
        let entry = rendered(ModelFacts::default());
        assert_eq!(entry["support_verbosity"], json!(false));
        assert_eq!(entry["default_verbosity"], Value::Null);
        assert_eq!(entry["supports_search_tool"], json!(false));
    }

    #[test]
    fn render_derives_effective_context_and_compaction_from_input_limit() {
        let facts = ModelFacts {
            context_window: Some(200_000),
            max_context_window: Some(200_000),
            input_token_limit: Some(128_000),
            ..ModelFacts::default()
        };
        let entry = rendered(facts);
        assert_eq!(entry["context_window"], json!(200_000));
        assert_eq!(entry["max_context_window"], json!(200_000));
        assert_eq!(entry["effective_context_window_percent"], json!(64));
        assert_eq!(entry["auto_compact_token_limit"], json!(115_200));
    }

    #[test]
    fn render_preserves_beta_status_for_generated_entries() {
        let facts = ModelFacts {
            status: Some("beta".to_string()),
            ..ModelFacts::default()
        };
        assert_eq!(rendered(facts)["status"], json!("beta"));
    }

    #[test]
    fn render_clears_verbosity_details_when_source_reports_false() {
        let facts = ModelFacts {
            support_verbosity: Some(false),
            supports_search_tool: Some(false),
            ..ModelFacts::default()
        };
        let entry = rendered(facts);
        assert_eq!(entry["support_verbosity"], json!(false));
        assert_eq!(entry["default_verbosity"], Value::Null);
        assert_eq!(entry["supports_search_tool"], json!(false));
    }

    #[test]
    fn render_only_touches_data_driven_keys() {
        let template = effective_without_export();
        let entry = rendered(ModelFacts::default());
        let entry = entry.as_object().unwrap();
        for (key, value) in &template.entry {
            if DATA_DRIVEN_KEYS.contains(&key.as_str()) {
                continue;
            }
            assert_eq!(entry.get(key), Some(value), "键 {key} 不应被渲染改动");
        }
        // 渲染不新增键（不再写 supports_reasoning_summary_parameter）；其余键
        // （含无档位时的 default_reasoning_level=null）一律保留，不再删键
        assert_eq!(entry.len(), template.entry.len());
        assert!(!entry.contains_key("supports_reasoning_summary_parameter"));
        assert!(entry.contains_key("default_reasoning_level"));
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
}
