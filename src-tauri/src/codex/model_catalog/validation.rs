//! 生成期的字段约束与最终条目校验，不改变手工目录保存规则。

use serde_json::{json, Value};

use super::facts::{FieldProvenance, FieldQuality, ModelFacts, Provenance};
use super::matching::MatchKind;

/// 输出字段的只读来源说明；未知的新版本字段仍保留在条目中。
pub const OUTPUT_FIELDS: &[&str] = &[
    "slug",
    "display_name",
    "description",
    "priority",
    "context_window",
    "max_context_window",
    "effective_context_window_percent",
    "auto_compact_token_limit",
    "input_modalities",
    "supported_reasoning_levels",
    "default_reasoning_level",
    "support_verbosity",
    "default_verbosity",
    "supports_search_tool",
    "supports_image_detail_original",
    "supports_reasoning_summary_parameter",
    "model_messages",
    "shell_type",
    "truncation_policy",
    "visibility",
    "supported_in_api",
    "experimental_supported_tools",
    "prefer_websockets",
    "tool_mode",
    "use_responses_lite",
    "web_search_tool_type",
    "apply_patch_tool_type",
    "multi_agent_version",
    "comp_hash",
    "minimal_client_version",
    "reasoning_summary_format",
    "default_reasoning_summary",
    "supports_reasoning_summaries",
    "include_skills_usage_instructions",
    "include_plugin_usage_instructions",
    "include_apps_usage_instructions",
    "supports_parallel_tool_calls",
    "available_in_plans",
    "service_tiers",
    "additional_speed_tiers",
    "default_service_tier",
    "auto_review_model_override",
    "availability_nux",
    "upgrade",
];

pub fn reconcile_facts(facts: &mut ModelFacts, warnings: &mut Vec<String>) {
    // 已知输入上限但上下文缺失时，不能直接套用可能更大的模板窗口。
    if facts.context_window.is_none() {
        if let Some(input) = facts.input_token_limit.filter(|input| *input > 0) {
            facts.context_window = Some(input);
            facts.max_context_window = Some(input);
            if let Some(mut origin) = facts.provenance.get("input_token_limit").cloned() {
                origin.reason = "上下文缺失，自动以已知输入上限作为保守基准".to_string();
                facts.provenance.insert("context_window", origin.clone());
                facts.provenance.insert("max_context_window", origin);
            }
            warnings.push("上下文资料缺失，已按已知输入上限自动派生".to_string());
        }
    }
    if let (Some(context), Some(input)) = (facts.context_window, facts.input_token_limit) {
        if input > context {
            let context_quality = facts
                .provenance
                .get("context_window")
                .map(|origin| origin.quality);
            let input_origin = facts.provenance.get("input_token_limit").cloned();
            if input_origin.as_ref().map(|origin| origin.quality) > context_quality {
                // 输入上限更可信时，用其作为保守上下文基准，不继承较低可信的旧限制。
                facts.context_window = Some(input);
                facts.max_context_window = Some(input);
                if let Some(mut origin) = input_origin {
                    origin.reason = "输入上限更可信，自动作为上下文基准".to_string();
                    origin.value = json!(input);
                    facts.provenance.insert("context_window", origin.clone());
                    facts.provenance.insert("max_context_window", origin);
                }
                warnings
                    .push("输入上限与上下文冲突，已按更可信的输入上限重新派生上下文".to_string());
            } else {
                facts.input_token_limit = None;
                facts.provenance.remove("input_token_limit");
                warnings.push("输入上限与上下文冲突，已放弃较低或同等可信的输入限制".to_string());
            }
        }
    }
    if let Some(default) = &facts.default_reasoning_level {
        if !facts.reasoning_levels.as_ref().is_some_and(|levels| {
            levels
                .iter()
                .any(|level| level.trim().eq_ignore_ascii_case(default.trim()))
        }) {
            facts.default_reasoning_level = None;
            facts.provenance.remove("default_reasoning_level");
            warnings.push("默认推理档位不在支持列表中，已自动清空".to_string());
        }
    }
    if let Some(origin) = facts.provenance.get("supports_tool_calls") {
        if origin.match_kind == MatchKind::Fuzzy && facts.supports_tool_calls == Some(false) {
            warnings.push(format!(
                "近似资料 {} 的 supports_tool_calls 存在限制，不作为目标模型禁用依据",
                origin.matched_id
            ));
        }
    }
}

pub fn validate_entry(entry: &mut Value, warnings: &mut Vec<String>) -> Result<(), String> {
    let object = entry.as_object().ok_or("生成条目不是对象")?;
    for key in ["slug", "display_name", "shell_type", "visibility"] {
        if !object
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(format!("生成条目的 {key} 必须是非空字符串"));
        }
    }
    if !entry["priority"].as_i64().is_some_and(|value| value > 0) {
        return Err("生成条目的 priority 必须为正整数".to_string());
    }
    if !entry
        .pointer("/model_messages/instructions_template")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
    {
        return Err("生成条目缺少有效的提示词模板".to_string());
    }
    for key in ["support_verbosity", "supported_in_api"] {
        if !entry[key].is_boolean() {
            return Err(format!("生成条目的 {key} 必须为布尔值"));
        }
    }
    for key in ["experimental_supported_tools", "supported_reasoning_levels"] {
        if !entry[key].is_array() {
            return Err(format!("生成条目的 {key} 必须为数组"));
        }
    }
    if !entry["experimental_supported_tools"]
        .as_array()
        .unwrap()
        .iter()
        .all(Value::is_string)
    {
        return Err("experimental_supported_tools 必须为字符串数组".to_string());
    }
    let policy = &entry["truncation_policy"];
    if !matches!(policy["mode"].as_str(), Some("tokens" | "bytes"))
        || !policy["limit"].as_i64().is_some_and(|limit| limit > 0)
    {
        return Err("生成条目的截断策略无效".to_string());
    }
    let levels = entry["supported_reasoning_levels"].as_array().unwrap();
    let mut seen = std::collections::HashSet::new();
    for level in levels {
        let effort = level["effort"]
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .ok_or("推理档位缺少有效 effort")?;
        if !level["description"].is_string() || !seen.insert(effort.to_string()) {
            return Err("推理档位描述无效或档位重复".to_string());
        }
    }
    if !entry["default_reasoning_level"].is_null() {
        let default = entry["default_reasoning_level"].as_str();
        if !default.is_some_and(|default| seen.contains(default)) {
            entry["default_reasoning_level"] = Value::Null;
            warnings.push("默认推理档位无效，已自动清空".to_string());
        }
    }
    for key in [
        "context_window",
        "max_context_window",
        "auto_compact_token_limit",
    ] {
        if !entry[key].is_null() && !entry[key].as_i64().is_some_and(|value| value > 0) {
            return Err(format!("生成条目的 {key} 必须为空或正整数"));
        }
    }
    if let (Some(context), Some(maximum)) = (
        entry["context_window"].as_i64(),
        entry["max_context_window"].as_i64(),
    ) {
        if maximum < context {
            return Err("最大上下文小于上下文窗口".to_string());
        }
    }
    if !entry["effective_context_window_percent"].is_null()
        && entry["effective_context_window_percent"].as_i64().is_none()
    {
        return Err("有效上下文百分比必须为整数".to_string());
    }
    let percent = entry["effective_context_window_percent"]
        .as_i64()
        .unwrap_or(95);
    if !(1..=100).contains(&percent) {
        return Err("有效上下文百分比必须在 1–100 之间".to_string());
    }
    if let (Some(context), Some(compact)) = (
        entry["context_window"].as_i64(),
        entry["auto_compact_token_limit"].as_i64(),
    ) {
        if i128::from(compact) * 100 > i128::from(context) * i128::from(percent) {
            return Err("自动压缩阈值超过可用上下文".to_string());
        }
    }
    if let Some(modalities) = entry.get("input_modalities") {
        if !modalities
            .as_array()
            .is_some_and(|values| values.iter().all(Value::is_string))
        {
            return Err("输入模态必须为字符串数组".to_string());
        }
    }
    Ok(())
}

/// 记录的是最终输出值；派生字段引用输入事实，未提供的字段注明模板默认。
pub fn complete_provenance(entry: &Value, provenance: &mut Provenance) {
    for key in OUTPUT_FIELDS {
        let Some(value) = entry.get(*key) else {
            continue;
        };
        let has_output_origin = provenance.contains_key(*key);
        let input = match *key {
            "supported_reasoning_levels" => "reasoning_levels",
            "supports_image_detail_original" => "input_modalities",
            "auto_compact_token_limit" | "effective_context_window_percent" => "input_token_limit",
            _ => key,
        };
        let mut origin = provenance
            .get(*key)
            .or_else(|| provenance.get(input))
            .cloned()
            .unwrap_or_else(|| {
                let mut origin =
                    FieldProvenance::new("template", "", MatchKind::Exact, FieldQuality::from(0));
                origin.reason = "来源未提供有效值，自动使用模板默认值".to_string();
                origin
            });
        if matches!(*key, "slug" | "priority")
            || (*key == "display_name" && origin.source != "official")
        {
            origin.source = "generator";
            origin.reason = "按提供方原始 ID 和候选顺序自动生成".to_string();
        } else if *key != input && !has_output_origin && provenance.contains_key(input) {
            origin.reason = format!("根据 {input} 自动派生");
        } else if origin.value != *value && origin.source != "template" {
            origin.reason = "根据字段约束自动修正为最终输出值".to_string();
        }
        origin.value = value.clone();
        provenance.insert(*key, origin);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_entry() -> Value {
        super::super::template::render(
            &super::super::template::from_snapshot(&[]).unwrap(),
            "review-model",
            &ModelFacts::default(),
            1,
        )
        .unwrap()
    }

    #[test]
    fn validates_required_shapes_context_and_compaction() {
        for (key, value) in [
            ("slug", json!("")),
            ("priority", json!(0)),
            ("model_messages", json!({})),
            ("support_verbosity", json!("false")),
            ("experimental_supported_tools", json!([1])),
            ("context_window", json!(-1)),
            ("max_context_window", json!(1)),
            ("auto_compact_token_limit", json!(272000)),
            ("effective_context_window_percent", json!("95")),
            ("input_modalities", json!("text")),
            ("supported_reasoning_levels", json!([{"effort":"low"}])),
        ] {
            let mut entry = valid_entry();
            entry[key] = value;
            assert!(
                validate_entry(&mut entry, &mut vec![]).is_err(),
                "应拒绝非法 {key}"
            );
        }
    }

    #[test]
    fn custom_efforts_and_audio_remain_valid_and_default_is_repaired() {
        let mut entry = valid_entry();
        entry["supported_reasoning_levels"] = json!([{"effort":"turbo","description":"自定义"}]);
        entry["input_modalities"] = json!(["text", "audio"]);
        entry["default_reasoning_level"] = json!("high");
        let mut warnings = vec![];
        validate_entry(&mut entry, &mut warnings).unwrap();
        assert!(entry["default_reasoning_level"].is_null());
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn conflicting_constraints_use_higher_quality_or_drop_equal_input_limit() {
        for input_authority in [20, 30] {
            let mut facts = ModelFacts {
                context_window: Some(1000),
                max_context_window: Some(1000),
                input_token_limit: Some(2000),
                ..ModelFacts::default()
            };
            facts.provenance.insert(
                "context_window",
                FieldProvenance::new("openrouter", "model", MatchKind::Exact, 20.into()),
            );
            facts.provenance.insert(
                "input_token_limit",
                FieldProvenance::new(
                    "models_dev",
                    "model",
                    MatchKind::Exact,
                    input_authority.into(),
                ),
            );
            let mut warnings = vec![];
            reconcile_facts(&mut facts, &mut warnings);
            if input_authority == 20 {
                assert_eq!(facts.context_window, Some(1000));
                assert_eq!(facts.input_token_limit, None);
            } else {
                assert_eq!(facts.context_window, Some(2000));
                assert_eq!(facts.max_context_window, Some(2000));
                assert_eq!(facts.input_token_limit, Some(2000));
            }
            assert_eq!(warnings.len(), 1);
        }
    }

    #[test]
    fn provenance_values_describe_the_final_output_not_raw_facts() {
        let mut facts = ModelFacts {
            reasoning_levels: Some(vec!["turbo".into()]),
            input_modalities: Some(vec!["audio".into()]),
            ..ModelFacts::default()
        };
        facts.attach_match("models_dev", "model", MatchKind::Exact, 30.into());
        let entry = super::super::template::render(
            &super::super::template::from_snapshot(&[]).unwrap(),
            "model",
            &facts,
            1,
        )
        .unwrap();
        complete_provenance(&entry, &mut facts.provenance);
        assert_eq!(
            facts.provenance["supported_reasoning_levels"].value,
            entry["supported_reasoning_levels"]
        );
        assert_eq!(
            facts.provenance["supported_reasoning_levels"].source,
            "models_dev"
        );
        assert_eq!(facts.provenance["model_messages"].source, "template");
        assert_eq!(facts.provenance["slug"].source, "generator");
        let mut official = Provenance::new();
        for key in ["display_name", "supported_reasoning_levels"] {
            let mut origin =
                FieldProvenance::new("official", "model", MatchKind::Exact, 100.into());
            origin.reason = "完整条目复用".to_string();
            origin.value = entry[key].clone();
            official.insert(key, origin);
        }
        complete_provenance(&entry, &mut official);
        assert_eq!(official["display_name"].source, "official");
        assert_eq!(
            official["supported_reasoning_levels"].reason,
            "完整条目复用"
        );
    }

    #[test]
    fn known_input_limit_is_not_lost_when_context_sources_are_missing() {
        let mut facts = ModelFacts {
            input_token_limit: Some(32000),
            ..ModelFacts::default()
        };
        facts.attach_match("models_dev", "model", MatchKind::Exact, 30.into());
        let mut warnings = vec![];
        reconcile_facts(&mut facts, &mut warnings);
        assert_eq!(facts.context_window, Some(32000));
        assert_eq!(facts.max_context_window, Some(32000));
        assert_eq!(facts.provenance["context_window"].source, "models_dev");
        let mut entry = super::super::template::render(
            &super::super::template::from_snapshot(&[]).unwrap(),
            "model",
            &facts,
            1,
        )
        .unwrap();
        assert_eq!(entry["context_window"], json!(32000));
        validate_entry(&mut entry, &mut warnings).unwrap();
    }
}
