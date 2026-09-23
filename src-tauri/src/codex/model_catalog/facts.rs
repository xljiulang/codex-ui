//! 字段源的提取结果：所有字段可选，合并时同时考虑来源可信度。
//!
//! 数据源只负责「提取 → 记录」，覆盖逻辑集中在 [`ModelFacts::merge_from`]，
//! 因此新增数据源不需要改动渲染器。

use super::matching::MatchKind;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// 匹配准确度优先于来源权威性，相似度仅用于模糊命中。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FieldQuality {
    pub reliability: u8,
    pub authority: u8,
    pub similarity: u16,
}

impl From<u8> for FieldQuality {
    fn from(authority: u8) -> Self {
        Self {
            reliability: 4,
            authority,
            similarity: 1000,
        }
    }
}

/// 单个最终字段的资料与选择依据（只读，不要求用户补参数）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldProvenance {
    pub source: &'static str,
    pub matched_id: String,
    pub match_kind: MatchKind,
    pub provider_id: Option<String>,
    pub value: Value,
    pub reason: String,
    #[serde(skip)]
    pub quality: FieldQuality,
}

impl FieldProvenance {
    pub fn new(
        source: &'static str,
        matched_id: &str,
        kind: MatchKind,
        quality: FieldQuality,
    ) -> Self {
        Self {
            source,
            matched_id: matched_id.to_string(),
            match_kind: kind,
            provider_id: None,
            value: Value::Null,
            quality,
            reason: "按匹配准确度、来源权威性和相似度自动选取".to_string(),
        }
    }

    fn stable_key(&self) -> (&str, &str, &str) {
        (
            self.source,
            &self.matched_id,
            self.provider_id.as_deref().unwrap_or(""),
        )
    }
}

/// 字段名 → 当前采用值的来源。
pub type Provenance = BTreeMap<&'static str, FieldProvenance>;

/// 一个数据源对单个模型给出的规范化能力事实；`None` 表示该源没提供。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ModelFacts {
    pub context_window: Option<i64>,
    pub max_context_window: Option<i64>,
    /// 提供方明确声明的最大输入 token。
    pub input_token_limit: Option<i64>,
    /// 已归一化为 codex 支持的取值：`text` / `image` / `audio`。
    pub input_modalities: Option<Vec<String>>,
    /// 提供方声明的推理档位；允许 Codex 的自定义档位。
    pub reasoning_levels: Option<Vec<String>>,
    pub default_reasoning_level: Option<String>,
    pub description: Option<String>,
    pub supports_reasoning: Option<bool>,
    pub support_verbosity: Option<bool>,
    pub supports_search_tool: Option<bool>,
    /// 明确 false 时，模型不适合作为 Codex 工具调用模型。
    pub supports_tool_calls: Option<bool>,
    pub provenance: Provenance,
    #[serde(skip)]
    pub warnings: Vec<String>,
}

impl ModelFacts {
    /// 按来源质量逐字段合并：低质量源只补空缺，不覆盖高质量值。
    pub fn merge_from(
        &mut self,
        source: &'static str,
        quality: impl Into<FieldQuality>,
        other: ModelFacts,
    ) {
        let quality = quality.into();
        self.warnings.extend(other.warnings.iter().cloned());
        macro_rules! merge_fields {
            ($($field:ident),+ $(,)?) => {
                $(
                    if let Some(value) = other.$field {
                        let field = stringify!($field);
                        let mut origin = other.provenance.get(field).cloned().unwrap_or_else(|| {
                            FieldProvenance::new(source, "", MatchKind::Exact, quality)
                        });
                        origin.quality = quality;
                        origin.value = serde_json::to_value(&value).expect("事实字段可序列化");
                        let should_replace = self.$field.is_none()
                            || self
                                .provenance
                                .get(field)
                                .is_none_or(|current| quality > current.quality
                                    || (quality == current.quality && origin.stable_key() < current.stable_key()));
                        if should_replace {
                            self.$field = Some(value);
                            self.provenance.insert(
                                field,
                                origin,
                            );
                        }
                    }
                )+
            };
        }
        merge_fields!(
            context_window,
            max_context_window,
            input_token_limit,
            input_modalities,
            reasoning_levels,
            default_reasoning_level,
            description,
            supports_reasoning,
            support_verbosity,
            supports_search_tool,
            supports_tool_calls,
        );
    }

    /// 来源内部的同模型多记录逐字段归并；同级冲突不取任意第一条。
    pub fn resolve_records(records: &[(Self, FieldProvenance)]) -> Self {
        let mut result = Self::default();
        macro_rules! resolve {
            ($($field:ident),+ $(,)?) => {$(
                let available: Vec<_> = records.iter()
                    .filter_map(|(facts, origin)| facts.$field.as_ref().map(|value| (value, origin)))
                    .collect();
                if let Some(tier) = available.iter().map(|(_, origin)| origin.quality.authority).max() {
                    let mut top: Vec<_> = available.into_iter()
                        .filter(|(_, origin)| origin.quality.authority == tier).collect();
                    top.sort_by(|(_, left), (_, right)| left.stable_key().cmp(&right.stable_key()));
                    let (value, origin) = top[0];
                    if top.iter().all(|(other, _)| equivalent_values(*other, value)) {
                        result.$field = Some(value.clone());
                        let mut origin = origin.clone();
                        origin.value = serde_json::to_value(value).expect("事实字段可序列化");
                        origin.reason = "规范模型资料优先，其次原厂资料；同级记录须一致".to_string();
                        result.provenance.insert(stringify!($field), origin);
                    } else {
                        result.warnings.push(format!("{} 的同级资料冲突，自动交由其他来源或模板补充", stringify!($field)));
                    }
                }
            )+};
        }
        resolve!(
            context_window,
            max_context_window,
            input_token_limit,
            input_modalities,
            reasoning_levels,
            default_reasoning_level,
            description,
            supports_reasoning,
            support_verbosity,
            supports_search_tool,
            supports_tool_calls
        );
        result
    }

    /// 给每个有值字段附上此次模型匹配身份，保留来源内部的 provider 证据。
    pub fn attach_match(
        &mut self,
        source: &'static str,
        id: &str,
        kind: MatchKind,
        quality: FieldQuality,
    ) {
        macro_rules! attach {
            ($($field:ident),+ $(,)?) => {$(
                if let Some(value) = &self.$field {
                    let origin = self.provenance.entry(stringify!($field))
                        .or_insert_with(|| FieldProvenance::new(source, id, kind, quality));
                    origin.match_kind = kind;
                    origin.quality = quality;
                    origin.value = serde_json::to_value(value).expect("事实字段可序列化");
                }
            )+};
        }
        attach!(
            context_window,
            max_context_window,
            input_token_limit,
            input_modalities,
            reasoning_levels,
            default_reasoning_level,
            description,
            supports_reasoning,
            support_verbosity,
            supports_search_tool,
            supports_tool_calls
        );
    }

    /// 该源是否为这个字段提供了当前值（供测试断言）。
    #[cfg(test)]
    pub fn source_of(&self, field: &str) -> Option<&'static str> {
        self.provenance.get(field).map(|origin| origin.source)
    }
}

/// 档位和模态是集合，来源的展示顺序不同不构成能力冲突。
fn equivalent_values<T: Serialize>(left: &T, right: &T) -> bool {
    let normalize = |value: &T| {
        let mut value = serde_json::to_value(value).expect("事实字段可序列化");
        if let Some(items) = value.as_array_mut() {
            items.sort_by_key(Value::to_string);
        }
        value
    };
    normalize(left) == normalize(right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accuracy_beats_source_and_absent_values_do_not_clear() {
        let exact = FieldQuality {
            reliability: 4,
            authority: 20,
            similarity: 1000,
        };
        let fuzzy = FieldQuality {
            reliability: 1,
            authority: 30,
            similarity: 900,
        };
        let accurate = ModelFacts {
            context_window: Some(128_000),
            supports_tool_calls: Some(false),
            reasoning_levels: Some(vec![]),
            ..ModelFacts::default()
        };
        let approximate = ModelFacts {
            context_window: Some(1_000_000),
            supports_tool_calls: Some(true),
            reasoning_levels: Some(vec!["high".into()]),
            description: Some("补缺".into()),
            ..ModelFacts::default()
        };
        let mut forward = ModelFacts::default();
        forward.merge_from("openrouter", exact, accurate.clone());
        forward.merge_from("models_dev", fuzzy, approximate.clone());
        let mut reverse = ModelFacts::default();
        reverse.merge_from("models_dev", fuzzy, approximate);
        reverse.merge_from("openrouter", exact, accurate);
        assert_eq!(forward, reverse);
        assert_eq!(forward.context_window, Some(128_000));
        assert_eq!(forward.supports_tool_calls, Some(false));
        assert_eq!(forward.reasoning_levels, Some(vec![]));
        assert_eq!(forward.description.as_deref(), Some("补缺"));
        forward.merge_from("models_dev", FieldQuality::from(30), ModelFacts::default());
        assert_eq!(forward.context_window, Some(128_000));
    }

    #[test]
    fn same_tier_conflicts_are_unknown_and_array_order_is_not_a_conflict() {
        let origin = |provider: &str| {
            let mut origin =
                FieldProvenance::new("models_dev", "model", MatchKind::Exact, 1.into());
            origin.provider_id = Some(provider.to_string());
            origin
        };
        let records = vec![
            (
                ModelFacts {
                    context_window: Some(100),
                    reasoning_levels: Some(vec!["low".into(), "high".into()]),
                    supports_tool_calls: Some(true),
                    ..ModelFacts::default()
                },
                origin("a"),
            ),
            (
                ModelFacts {
                    context_window: Some(200),
                    reasoning_levels: Some(vec!["high".into(), "low".into()]),
                    supports_tool_calls: Some(true),
                    ..ModelFacts::default()
                },
                origin("b"),
            ),
        ];
        let facts = ModelFacts::resolve_records(&records);
        assert_eq!(facts.context_window, None);
        assert_eq!(facts.supports_tool_calls, Some(true));
        assert_eq!(
            facts.reasoning_levels,
            Some(vec!["low".into(), "high".into()])
        );
        assert!(facts
            .warnings
            .iter()
            .any(|warning| warning.contains("context_window")));
        assert_eq!(
            facts,
            ModelFacts::resolve_records(&records.into_iter().rev().collect::<Vec<_>>())
        );
    }

    fn facts(context_window: i64, description: &str) -> ModelFacts {
        ModelFacts {
            context_window: Some(context_window),
            description: Some(description.to_string()),
            ..ModelFacts::default()
        }
    }

    #[test]
    fn merge_overrides_only_with_equal_or_higher_quality() {
        let mut merged = ModelFacts::default();
        merged.merge_from("openrouter", 10, facts(1_048_576, "openrouter 描述"));
        merged.merge_from("models.dev", 20, facts(1_000_000, "官方描述"));
        assert_eq!(merged.context_window, Some(1_000_000));
        assert_eq!(merged.description.as_deref(), Some("官方描述"));
        assert_eq!(merged.source_of("context_window"), Some("models.dev"));
    }

    #[test]
    fn merge_keeps_earlier_value_when_later_source_is_silent() {
        let mut merged = ModelFacts::default();
        merged.merge_from("openrouter", 10, facts(1_048_576, "openrouter 描述"));
        let silent = ModelFacts {
            supports_search_tool: Some(true),
            ..ModelFacts::default()
        };
        merged.merge_from("models.dev", 20, silent);
        assert_eq!(merged.context_window, Some(1_048_576));
        assert_eq!(merged.source_of("context_window"), Some("openrouter"));
        assert_eq!(merged.source_of("supports_search_tool"), Some("models.dev"));
    }

    #[test]
    fn lower_quality_source_only_fills_missing_fields() {
        let mut merged = ModelFacts::default();
        merged.merge_from("scoped", 30, facts(100, "scoped"));
        merged.merge_from("global", 10, facts(200, "global"));
        assert_eq!(merged.context_window, Some(100));
        assert_eq!(merged.description.as_deref(), Some("scoped"));
        assert_eq!(merged.source_of("context_window"), Some("scoped"));
    }
}
