//! 目录条目的渲染基底。
//!
//! **模板 JSON 是固定值与占位值的唯一事实来源**：代码只声明哪些键由字段源覆盖
//! （[`DATA_DRIVEN_KEYS`]）以及四条派生规则，不再持有任何固定取值常量。
//! 调整生成条目的固定行为 = 只改 `resources/model_catalog_template.json`。

use std::sync::OnceLock;

use serde_json::{json, Map, Value};

use super::facts::ModelFacts;

const TEMPLATE_JSON: &str = include_str!("../../../resources/model_catalog_template.json");

/// 由字段源覆盖的键；模板里其余键一律原样继承。
///
/// `supports_reasoning_summary_parameter` 不在模板里（官方内置条目也没有该键），
/// 由渲染器按推理能力写入。
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
    "context_window",
    "max_context_window",
    "support_verbosity",
    "default_verbosity",
    "supports_search_tool",
    "supports_image_detail_original",
    "supports_reasoning_summary_parameter",
];

/// codex 已知的推理档位（未知档位会被过滤，避免写入非法值）。
pub const KNOWN_REASONING_EFFORTS: &[&str] = &[
    "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
];

/// 源没给出默认档位时的回退顺序。
const DEFAULT_REASONING_FALLBACK_ORDER: &[&str] =
    &["medium", "high", "low", "xhigh", "max", "minimal", "none"];

/// 源没有给出描述时的兜底文案。
const DESCRIPTION_FALLBACK: &str = "由模型提供者目录生成";

/// 模板（已做 OnceLock 缓存）。模板非法时返回错误，避免静默产出残缺条目。
pub fn template() -> Result<&'static Map<String, Value>, String> {
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

/// 用字段源的合并结果渲染一个目录条目。
pub fn render(model_id: &str, facts: &ModelFacts, priority: usize) -> Result<Value, String> {
    let template = template()?;
    let mut entry = template.clone();

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
    }

    // 输入模态：同时决定图片细节能力（纯文本模型不应声称支持 image detail original）。
    let modalities = normalize_modalities(facts.input_modalities.as_deref().unwrap_or(&[]));
    entry.insert(
        "supports_image_detail_original".to_string(),
        json!(modalities.iter().any(|modality| modality == "image")),
    );
    entry.insert("input_modalities".to_string(), json!(modalities));

    // 推理档位与默认档位。
    let levels = filter_known_efforts(facts.reasoning_levels.as_deref().unwrap_or(&[]));
    let presets: Vec<Value> = levels
        .iter()
        .map(|effort| {
            json!({
                "effort": effort,
                "description": reasoning_effort_description(template, effort),
            })
        })
        .collect();
    match resolve_default_reasoning_level(facts, &levels) {
        Some(default) => {
            entry.insert("default_reasoning_level".to_string(), json!(default));
        }
        None => {
            entry.remove("default_reasoning_level");
        }
    }
    entry.insert(
        "supported_reasoning_levels".to_string(),
        Value::Array(presets),
    );
    entry.insert(
        "supports_reasoning_summary_parameter".to_string(),
        json!(facts.supports_reasoning.unwrap_or(!levels.is_empty())),
    );

    // 源明确给了 verbosity / 搜索能力才覆盖；源无信息时保留模板取值。
    match facts.support_verbosity {
        Some(true) => {
            entry.insert("support_verbosity".to_string(), json!(true));
        }
        Some(false) => {
            entry.insert("support_verbosity".to_string(), json!(false));
            entry.remove("default_verbosity");
        }
        None => {}
    }
    if let Some(supports_search_tool) = facts.supports_search_tool {
        entry.insert(
            "supports_search_tool".to_string(),
            json!(supports_search_tool),
        );
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
    if normalized.is_empty() {
        normalized.push("text".to_string());
    }
    normalized
}

fn filter_known_efforts(efforts: &[String]) -> Vec<String> {
    let mut filtered: Vec<String> = Vec::new();
    for effort in efforts {
        let effort = effort.trim().to_ascii_lowercase();
        if !KNOWN_REASONING_EFFORTS.contains(&effort.as_str()) {
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
    for candidate in DEFAULT_REASONING_FALLBACK_ORDER {
        if levels.iter().any(|level| level == candidate) {
            return Some((*candidate).to_string());
        }
    }
    levels.first().cloned()
}

fn reasoning_effort_description(template: &Map<String, Value>, effort: &str) -> String {
    template
        .get("supported_reasoning_levels")
        .and_then(Value::as_array)
        .and_then(|levels| {
            levels.iter().find(|level| {
                level.get("effort").and_then(Value::as_str) == Some(effort)
            })
        })
        .and_then(|level| level.get("description"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("{effort} reasoning effort"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(facts: ModelFacts) -> Value {
        render("mimo-v2.5-free", &facts, 1).unwrap()
    }

    #[test]
    fn template_is_the_single_source_of_truth_for_fixed_values() {
        let template = template().unwrap();
        assert_eq!(template.len(), 43, "模板键数变化需同步更新文档与测试");
        assert_eq!(template["prefer_websockets"], json!(false));
        assert_eq!(template["web_search_tool_type"], json!("text"));
        assert_eq!(template["use_responses_lite"], json!(false));
        assert_eq!(template["tool_mode"], Value::Null);
        assert_eq!(template["truncation_policy"], json!({"mode":"tokens","limit":10000}));
        assert_eq!(template["multi_agent_version"], json!("v2"));
        assert_eq!(template["comp_hash"], json!("3000"));
        assert_eq!(template["minimal_client_version"], json!("0.144.0"));
        assert_eq!(template["reasoning_summary_format"], json!("experimental"));
        assert_eq!(template["default_reasoning_summary"], json!("none"));
        assert_eq!(template["include_skills_usage_instructions"], json!(false));
        assert_eq!(template["include_plugin_usage_instructions"], json!(true));
        assert_eq!(template["include_apps_usage_instructions"], json!(true));
        assert_eq!(template["effective_context_window_percent"], json!(95));
        assert_eq!(template["supports_parallel_tool_calls"], json!(true));
        assert_eq!(template["shell_type"], json!("shell_command"));
        assert_eq!(template["apply_patch_tool_type"], json!("freeform"));
        assert_eq!(template["visibility"], json!("list"));
        assert_eq!(template["supported_in_api"], json!(true));
        assert_eq!(template["available_in_plans"], json!([]));
        assert_eq!(template["service_tiers"], json!([]));
        assert_eq!(template["additional_speed_tiers"], json!([]));
        assert_eq!(template["default_service_tier"], Value::Null);
        // supported_reasoning_levels 是渲染期的“档位描述表”，写入前必被覆盖
        assert!(!template["supported_reasoning_levels"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn template_keeps_instructions_without_duplicate_base_instructions() {
        let template = template().unwrap();
        assert!(template.get("base_instructions").is_none());
        let instructions = template["model_messages"]["instructions_template"]
            .as_str()
            .unwrap();
        assert!(instructions.starts_with("You are Codex"));
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
    fn render_filters_unknown_efforts_and_resolves_default_level() {
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
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0]["effort"], json!("low"));
        // 描述取模板内置的档位描述表（写入前后者必被覆盖，不会泄漏到其它条目）
        assert_eq!(
            levels[0]["description"],
            json!("Fast responses with lighter reasoning")
        );
        assert_eq!(entry["default_reasoning_level"], json!("high"));
        assert_eq!(entry["supports_reasoning_summary_parameter"], json!(true));
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
        assert_eq!(rendered(facts)["default_reasoning_level"], json!("low"));
    }

    #[test]
    fn render_drops_default_level_when_there_are_no_levels() {
        let facts = ModelFacts {
            reasoning_levels: Some(vec!["turbo".to_string()]),
            ..ModelFacts::default()
        };
        let entry = rendered(facts);
        assert_eq!(entry["supported_reasoning_levels"], json!([]));
        assert!(entry.get("default_reasoning_level").is_none());
        assert_eq!(entry["supports_reasoning_summary_parameter"], json!(false));
    }

    #[test]
    fn render_inherits_template_verbosity_and_search_when_source_is_silent() {
        let entry = rendered(ModelFacts::default());
        assert_eq!(entry["support_verbosity"], json!(true));
        assert_eq!(entry["default_verbosity"], json!("low"));
        assert_eq!(entry["supports_search_tool"], json!(true));
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
        assert!(entry.get("default_verbosity").is_none());
        assert_eq!(entry["supports_search_tool"], json!(false));
    }

    #[test]
    fn render_only_touches_data_driven_keys() {
        let template = template().unwrap();
        let entry = rendered(ModelFacts::default());
        let entry = entry.as_object().unwrap();
        for (key, value) in template {
            if DATA_DRIVEN_KEYS.contains(&key.as_str()) {
                continue;
            }
            assert_eq!(entry.get(key), Some(value), "键 {key} 不应被渲染改动");
        }
        // 渲染新增 supports_reasoning_summary_parameter、并移除无档位时的 default_reasoning_level
        assert_eq!(entry.len(), template.len());
        assert!(entry.contains_key("supports_reasoning_summary_parameter"));
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
