//! 字段源的提取结果：所有字段可选，合并时后注册的源逐字段覆盖。
//!
//! 数据源只负责「提取 → 记录」，覆盖逻辑集中在 [`ModelFacts::merge_from`]，
//! 因此新增数据源不需要改动合并与渲染。

use std::collections::BTreeMap;

/// 字段名 → 写入该字段的源 id。
pub type Provenance = BTreeMap<&'static str, &'static str>;

/// 一个数据源对单个模型给出的规范化能力事实；`None` 表示该源没提供这个字段。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelFacts {
    pub context_window: Option<i64>,
    pub max_context_window: Option<i64>,
    /// 已归一化为 codex 支持的取值：`text` / `image` / `audio`
    pub input_modalities: Option<Vec<String>>,
    /// 已过滤为 codex 已知的推理档位
    pub reasoning_levels: Option<Vec<String>>,
    pub default_reasoning_level: Option<String>,
    pub description: Option<String>,
    pub supports_reasoning: Option<bool>,
    pub support_verbosity: Option<bool>,
    pub supports_search_tool: Option<bool>,
    pub provenance: Provenance,
}

impl ModelFacts {
    /// 逐字段覆盖：仅当 `other` 提供了该字段时才覆盖，并记录来源。
    pub fn merge_from(&mut self, source: &'static str, other: ModelFacts) {
        macro_rules! merge_fields {
            ($($field:ident),+ $(,)?) => {
                $(
                    if let Some(value) = other.$field {
                        self.$field = Some(value);
                        self.provenance.insert(stringify!($field), source);
                    }
                )+
            };
        }
        merge_fields!(
            context_window,
            max_context_window,
            input_modalities,
            reasoning_levels,
            default_reasoning_level,
            description,
            supports_reasoning,
            support_verbosity,
            supports_search_tool,
        );
    }

    /// 该源是否为这个字段提供了值（供测试断言合并与覆盖顺序）。
    #[cfg(test)]
    pub fn source_of(&self, field: &str) -> Option<&'static str> {
        self.provenance.get(field).copied()
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
    fn merge_overrides_only_provided_fields() {
        let mut merged = ModelFacts::default();
        merged.merge_from("openrouter", facts(1_048_576, "openrouter 描述"));
        merged.merge_from("models.dev", facts(1_000_000, "官方描述"));
        assert_eq!(merged.context_window, Some(1_000_000));
        assert_eq!(merged.description.as_deref(), Some("官方描述"));
        assert_eq!(merged.source_of("context_window"), Some("models.dev"));
    }

    #[test]
    fn merge_keeps_earlier_value_when_later_source_is_silent() {
        let mut merged = ModelFacts::default();
        merged.merge_from("openrouter", facts(1_048_576, "openrouter 描述"));
        let silent = ModelFacts {
            supports_search_tool: Some(true),
            ..ModelFacts::default()
        };
        merged.merge_from("models.dev", silent);
        assert_eq!(merged.context_window, Some(1_048_576));
        assert_eq!(merged.source_of("context_window"), Some("openrouter"));
        assert_eq!(merged.source_of("supports_search_tool"), Some("models.dev"));
    }

    #[test]
    fn provenance_tracks_latest_writer_per_field() {
        let mut merged = ModelFacts::default();
        merged.merge_from("openrouter", facts(1_000, "a"));
        assert_eq!(merged.source_of("context_window"), Some("openrouter"));
        assert_eq!(merged.source_of("description"), Some("openrouter"));
        assert!(merged.source_of("support_verbosity").is_none());
        assert!(!merged.provenance.is_empty());
    }
}
