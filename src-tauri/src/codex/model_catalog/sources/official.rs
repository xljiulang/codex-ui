//! 官方条目源：随包发布的权威模型条目（「完整条目源」）。
//!
//! `model_catalog_json` 是整份替换 codex 自带的模型目录（bundled catalog），因此
//! 写进自定义目录的条目必须与厂商给出的权威配置一致：命中 slug 时整条复用（含专属提示词、上下文、
//! `tool_mode`、`use_responses_lite`、`include_*` 等），只按提供方性质收敛
//! `prefer_websockets`（第三方中转不发起 WebSocket 连接）。
//!
//! `resources/official-models.json` 是多厂商共用的官方条目池：目前由
//! `scripts/update-model-catalog-sources.mjs --official` 刷新 codex 基线条目，
//! 后续可直接手工追加其它厂商（如 deepseek）提供的条目——脚本刷新时会保留这些条目。

use std::sync::OnceLock;

use serde_json::{json, Value};

use super::{FullEntryMatch, FullEntrySource};
use crate::codex::model_catalog::matching::{MatchKind, normalize_model_key};

const OFFICIAL_MODELS_JSON: &str = include_str!("../../../../resources/official-models.json");

pub struct OfficialModelSource {
    official_openai: bool,
}

impl OfficialModelSource {
    pub fn new(base_url: &str) -> Self {
        Self {
            official_openai: is_official_openai_host(base_url),
        }
    }
}

impl FullEntrySource for OfficialModelSource {
    fn id(&self) -> &'static str {
        "official"
    }

    /// 只做精确匹配（原始 slug 或规范化后相同），不做模糊匹配：
    /// 避免把提供方返回的 `gpt-5.6` 之类乱映射到 `gpt-5.6-sol/terra/luna`。
    fn full_entry(&self, model_id: &str) -> Option<FullEntryMatch> {
        let query = model_id.trim();
        if query.is_empty() {
            return None;
        }
        let query_key = normalize_model_key(query);
        let (entry, kind) = official_entries().iter().find_map(|entry| {
            let slug = entry.get("slug").and_then(Value::as_str).unwrap_or("");
            if slug == query {
                Some((entry, MatchKind::Exact))
            } else if !query_key.is_empty() && normalize_model_key(slug) == query_key {
                Some((entry, MatchKind::Normalized))
            } else {
                None
            }
        })?;

        let mut value = entry.clone();
        if !self.official_openai {
            if let Some(object) = value.as_object_mut() {
                object.insert("prefer_websockets".to_string(), json!(false));
            }
        }
        Some(FullEntryMatch {
            entry: value,
            matched_id: entry
                .get("slug")
                .and_then(Value::as_str)
                .unwrap_or(query)
                .to_string(),
            kind,
        })
    }
}

/// 官方条目（进程内解析一次）。
fn official_entries() -> &'static [Value] {
    static ENTRIES: OnceLock<Vec<Value>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        let parsed: Value = serde_json::from_str(OFFICIAL_MODELS_JSON).unwrap_or(Value::Null);
        parsed
            .get("models")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    })
}

/// 提供方是否走 OpenAI 官方后端（只有官方才保留条目自带的 WebSocket 传输偏好）。
fn is_official_openai_host(base_url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url.trim()) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    host == "openai.com"
        || host.ends_with(".openai.com")
        || host == "chatgpt.com"
        || host.ends_with(".chatgpt.com")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn official_entry(slug: &str) -> &'static Value {
        official_entries()
            .iter()
            .find(|entry| entry.get("slug").and_then(Value::as_str) == Some(slug))
            .unwrap_or_else(|| panic!("官方条目缺少 {slug}"))
    }

    #[test]
    fn bundled_official_entries_are_well_formed() {
        let entries = official_entries();
        assert!(entries.len() >= 8, "官方条目数变化需同步更新文档");
        for entry in entries {
            assert!(
                entry.get("slug").and_then(Value::as_str).is_some(),
                "官方条目缺少 slug（注意别把整个 models.json 当成单个条目粘进来）"
            );
            assert!(entry
                .pointer("/model_messages/instructions_template")
                .and_then(Value::as_str)
                .is_some());
        }
    }

    #[test]
    fn full_entry_reuses_official_entry_verbatim_for_third_party_provider() {
        let source = OfficialModelSource::new("https://api.deepseek.com/v1");
        let entry = source.full_entry("gpt-5.6-sol").unwrap().entry;
        let official_entry = official_entry("gpt-5.6-sol");

        for (key, value) in official_entry.as_object().unwrap() {
            if key == "prefer_websockets" {
                continue;
            }
            assert_eq!(entry.get(key), Some(value), "键 {key} 应原样复用官方条目");
        }
        assert_eq!(entry["prefer_websockets"], json!(false));
        assert_eq!(entry["max_context_window"], json!(872000));
        assert_eq!(entry["tool_mode"], json!("code_mode_only"));
        assert_eq!(entry["use_responses_lite"], json!(true));
        assert_eq!(entry["web_search_tool_type"], json!("text_and_image"));
    }

    #[test]
    fn full_entry_keeps_websockets_for_official_openai_host() {
        let source = OfficialModelSource::new("https://api.openai.com/v1");
        let entry = source.full_entry("gpt-5.6-sol").unwrap().entry;
        assert_eq!(entry["prefer_websockets"], json!(true));
    }

    #[test]
    fn full_entry_matches_normalized_slug_only() {
        let source = OfficialModelSource::new("https://relay.example.com/v1");
        assert!(source.full_entry("GPT-5.6-Sol").is_some());
        // 规范化会剥掉 `-preview` 这类别名后缀：仍命中对应官方条目
        let preview = source.full_entry("gpt-5.6-terra-preview").unwrap();
        assert_eq!(preview.entry["slug"], json!("gpt-5.6-terra"));
        assert_eq!(preview.kind, MatchKind::Normalized);
        // 没有任何别名键能对应上时不猜（不做模糊匹配）
        assert!(source.full_entry("gpt-5.6").is_none());
        assert!(source.full_entry("gpt-5.9-unknown").is_none());
        assert!(source.full_entry("unknown-model").is_none());
    }

    #[test]
    fn official_host_detection_ignores_lookalike_domains() {
        assert!(is_official_openai_host("https://api.openai.com/v1"));
        assert!(is_official_openai_host("https://chatgpt.com/backend-api"));
        assert!(!is_official_openai_host("https://openai.com.evil.example/v1"));
        assert!(!is_official_openai_host("https://relay.example.com/v1"));
        assert!(!is_official_openai_host(""));
    }
}
