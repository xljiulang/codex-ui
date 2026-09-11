//! 字段源的提取结果：所有字段可选，合并时同时考虑来源可信度。
//!
//! 数据源只负责「提取 → 记录」，覆盖逻辑集中在 [`ModelFacts::merge_from`]，
//! 因此新增数据源不需要改动渲染器。

use std::collections::BTreeMap;

/// 单个字段的来源与可信优先级。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldProvenance {
    pub source: &'static str,
    pub quality: u8,
}

/// 字段名 → 当前采用值的来源。
pub type Provenance = BTreeMap<&'static str, FieldProvenance>;

/// 一个数据源对单个模型给出的规范化能力事实；`None` 表示该源没提供。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
    /// 上源模型状态，目前主要处理 `beta` / `deprecated`。
    pub status: Option<String>,
    pub provenance: Provenance,
}

impl ModelFacts {
    /// 按来源质量逐字段合并：低质量源只补空缺，不覆盖高质量值。
    pub fn merge_from(&mut self, source: &'static str, quality: u8, other: ModelFacts) {
        macro_rules! merge_fields {
            ($($field:ident),+ $(,)?) => {
                $(
                    if let Some(value) = other.$field {
                        let field = stringify!($field);
                        let should_replace = self.$field.is_none()
                            || self
                                .provenance
                                .get(field)
                                .is_none_or(|current| quality >= current.quality);
                        if should_replace {
                            self.$field = Some(value);
                            self.provenance.insert(
                                field,
                                FieldProvenance { source, quality },
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
            status,
        );
    }

    /// 该源是否为这个字段提供了当前值（供测试断言）。
    #[cfg(test)]
    pub fn source_of(&self, field: &str) -> Option<&'static str> {
        self.provenance.get(field).map(|origin| origin.source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
