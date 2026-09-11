//! 官方条目源：运行期导出的本机 codex 条目 + 随包发布的第三方官方条目（「完整条目源」）。
//!
//! `model_catalog_json` 是整份替换 codex 自带的模型目录（bundled catalog），因此
//! 写进自定义目录的条目必须与厂商给出的权威配置一致：命中 slug 时整条复用（含专属提示词、上下文、
//! `tool_mode`、`use_responses_lite`、`include_*` 等），只按提供方性质收敛
//! `prefer_websockets`（第三方中转不发起 WebSocket 连接）。
//!
//! 条目池由两部分合并（同 slug 时运行期条目优先）：
//! - 启动时从用户安装的 codex 导出的自带目录（见 [`super::super::codex_models`]），
//!   GPT/codex 基线条目因此始终与该 codex 版本匹配；
//! - `resources/official-models.json` 里手写的第三方官方条目（如 deepseek），
//!   由 `scripts/update-model-catalog-sources.mjs --official` 校验并规范化。

use std::collections::HashSet;
#[cfg(test)]
use std::path::Path;
use std::sync::OnceLock;

use serde_json::{json, Value};

use super::{FullEntryMatch, FullEntrySource};
#[cfg(test)]
use crate::codex::model_catalog::codex_models;
use crate::codex::model_catalog::matching::{MatchKind, normalize_model_key, namespaces_conflict};

const OFFICIAL_MODELS_JSON: &str = include_str!("../../../../resources/official-models.json");

pub struct OfficialModelSource {
    entries: Vec<Value>,
    official_openai: bool,
}

impl OfficialModelSource {
    #[cfg(test)]
    pub fn new(app_dir: &Path, base_url: &str) -> Self {
        Self::from_snapshot(&codex_models::codex_snapshot(app_dir), base_url)
    }

    pub fn from_snapshot(snapshot: &[Value], base_url: &str) -> Self {
        Self {
            entries: official_pool(snapshot),
            official_openai: is_official_openai_host(base_url),
        }
    }

    fn normalized_entries(&self, query: &str) -> Vec<&Value> {
        let key = normalize_model_key(query);
        self.entries.iter().filter(|entry| {
            let slug = entry["slug"].as_str().unwrap_or("");
            !key.is_empty() && !namespaces_conflict(query, slug) && normalize_model_key(slug) == key
        }).collect()
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
        let (entry, kind) = if let Some(entry) = self.entries.iter()
            .find(|entry| entry["slug"].as_str() == Some(query)) {
            (entry, MatchKind::Exact)
        } else {
            let normalized = self.normalized_entries(query);
            if normalized.len() != 1 { return None; }
            (normalized[0], MatchKind::Normalized)
        };

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

    fn warnings(&self, model_id: &str) -> Vec<String> {
        let exact = self.entries.iter().any(|entry| entry["slug"].as_str() == Some(model_id.trim()));
        if !exact && self.normalized_entries(model_id).len() > 1 {
            vec!["官方条目规范化匹配存在歧义，已自动改用字段来源".to_string()]
        } else { Vec::new() }
    }
}

/// 官方条目池：运行期导出的本机 codex 条目优先，内置资源只放第三方官方条目。
fn official_pool(snapshot: &[Value]) -> Vec<Value> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut entries = Vec::new();
    // 与模板基底共用同一份进程内快照：不重复解析、也不再起子进程
    for entry in snapshot
        .iter().cloned()
        .chain(bundled_entries().iter().cloned())
    {
        let slug = entry
            .get("slug")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if slug.is_empty() || !seen.insert(slug.to_string()) {
            continue;
        }
        entries.push(entry);
    }
    entries
}

/// 随包发布的第三方官方条目（进程内解析一次）。
fn bundled_entries() -> &'static [Value] {
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
    use tempfile::tempdir;

    #[test]
    fn exact_match_beats_earlier_normalized_entry_and_ambiguity_falls_back() {
        for entries in [
            vec![gpt_entry("gpt-test-preview"), gpt_entry("gpt-test")],
            vec![gpt_entry("gpt-test"), gpt_entry("gpt-test-preview")],
        ] {
            let source = OfficialModelSource::from_snapshot(&entries, "https://relay.example.com");
            let found = source.full_entry("gpt-test").unwrap();
            assert_eq!(found.kind, MatchKind::Exact);
            assert_eq!(found.matched_id, "gpt-test");
            assert!(source.full_entry("GPT-TEST").is_none());
            assert_eq!(source.warnings("GPT-TEST").len(), 1);
        }
        let source = OfficialModelSource::from_snapshot(
            &[gpt_entry("vendor/model-v5")], "https://relay.example.com");
        assert!(source.full_entry("other/model-v5").is_none());
    }

    /// 运行期导出的 GPT 基线条目（本机 codex 经 `codex debug models --bundled` 导出）。
    fn gpt_entry(slug: &str) -> Value {
        json!({
            "slug": slug,
            "display_name": "GPT",
            "prefer_websockets": true,
            "tool_mode": "code_mode_only",
            "use_responses_lite": true,
            "web_search_tool_type": "text_and_image",
            "context_window": 272_000,
            "max_context_window": 872_000,
            "model_messages": { "instructions_template": "You are Codex" }
        })
    }

    #[test]
    fn bundled_pool_has_only_third_party_entries() {
        let entries = bundled_entries();
        assert!(entries.len() >= 2, "第三方官方条目数变化需同步更新文档");
        for entry in entries {
            let slug = entry
                .get("slug")
                .and_then(Value::as_str)
                .expect("官方条目缺少 slug（注意别把整个 models.json 当成单个条目粘进来）");
            assert!(
                !slug.starts_with("gpt-") && slug != "codex-auto-review",
                "GPT/codex 基线条目应由本机 codex 导出，不再内置：{slug}"
            );
            assert!(entry
                .pointer("/model_messages/instructions_template")
                .and_then(Value::as_str)
                .is_some());
        }
        assert!(entries.iter().any(|entry| entry["slug"] == json!("deepseek-flash")));
        assert!(entries.iter().any(|entry| entry["slug"] == json!("deepseek-v4-pro")));
    }

    #[test]
    fn runtime_export_entries_are_reused_and_override_bundled() {
        let dir = tempdir().unwrap();
        codex_models::write_export_for_test(
            dir.path(),
            vec![
                gpt_entry("gpt-5.6-sol"),
                json!({
                    "slug": "deepseek-flash",
                    "description": "runtime override",
                    "model_messages": { "instructions_template": "You are Codex" }
                }),
            ],
        );
        let source = OfficialModelSource::new(dir.path(), "https://api.deepseek.com/v1");

        // 运行期导出的 GPT 条目整条复用，第三方中转收敛 prefer_websockets
        let entry = source.full_entry("gpt-5.6-sol").unwrap().entry;
        assert_eq!(entry["prefer_websockets"], json!(false));
        assert_eq!(entry["max_context_window"], json!(872000));
        assert_eq!(entry["tool_mode"], json!("code_mode_only"));
        assert_eq!(entry["use_responses_lite"], json!(true));
        assert_eq!(entry["web_search_tool_type"], json!("text_and_image"));
        assert_eq!(
            entry["model_messages"]["instructions_template"],
            json!("You are Codex")
        );

        // 同 slug 时运行期条目覆盖内置条目
        let overridden = source.full_entry("deepseek-flash").unwrap().entry;
        assert_eq!(overridden["description"], json!("runtime override"));
    }

    #[test]
    fn full_entry_keeps_websockets_for_official_openai_host() {
        let dir = tempdir().unwrap();
        codex_models::write_export_for_test(dir.path(), vec![gpt_entry("gpt-5.6-sol")]);
        let source = OfficialModelSource::new(dir.path(), "https://api.openai.com/v1");
        let entry = source.full_entry("gpt-5.6-sol").unwrap().entry;
        assert_eq!(entry["prefer_websockets"], json!(true));
    }

    #[test]
    fn full_entry_matches_normalized_slug_only() {
        let dir = tempdir().unwrap();
        codex_models::write_export_for_test(
            dir.path(),
            vec![gpt_entry("gpt-5.6-sol"), gpt_entry("gpt-5.6-terra")],
        );
        let source = OfficialModelSource::new(dir.path(), "https://relay.example.com/v1");
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
    fn pool_without_runtime_export_keeps_only_bundled_entries() {
        let dir = tempdir().unwrap();
        let source = OfficialModelSource::new(dir.path(), "https://relay.example.com/v1");
        assert!(source.full_entry("deepseek-flash").is_some());
        assert!(
            source.full_entry("gpt-5.6-sol").is_none(),
            "没有运行期导出时不再内置 GPT 条目"
        );
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
