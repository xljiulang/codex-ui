//! 技能管理：从 codex app-server 的 `skills/list` 聚合列表中过滤出本地技能。
//!
//! 本地用户技能判定：`scope == "user"`（0.149 协议中 scope 为必填枚举），
//! 并排除插件缓存路径（plugins/cache）。展示包含禁用技能（不做 enabled 过滤）。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::app_server::CodexServer;
use super::model_config::codex_home;
use crate::codex::path_util::{clean_path, norm_path_key};

/// 单个本地技能条目。
#[derive(Debug, Clone, Serialize)]
pub struct SkillInfo {
    pub name: String,
    /// SKILL.md 的绝对路径。
    pub path: String,
    pub description: String,
    pub enabled: bool,
    /// skills/list 返回的 scope（user/repo/system/admin）。
    pub scope: String,
}

/// `skills_read` 的返回结构。
#[derive(Debug, Clone, Serialize)]
pub struct SkillsState {
    /// 技能根目录（CODEX_HOME/skills）。
    pub skills_dir: String,
    pub items: Vec<SkillInfo>,
    /// 加载失败的技能（如 frontmatter 缺少必填字段），供界面提示而非静默消失。
    pub errors: Vec<SkillErrorInfo>,
}

/// `skills/list` 返回的单个技能加载错误。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillErrorInfo {
    pub message: String,
    pub path: String,
}

/// `skills/list` 返回的单个技能条目（只声明需要的字段；0.149 中
/// name/description/path/scope/enabled 均为必填）。
#[derive(Debug, Clone, Deserialize)]
struct SkillListItem {
    name: String,
    path: String,
    description: String,
    enabled: bool,
    scope: String,
}

/// 读取本地技能列表：请求 `skills/list` 后按路径过滤出 `{CODEX_HOME}/skills` 下的技能。
pub async fn read_state(server: &CodexServer, force_reload: bool) -> Result<SkillsState, String> {
    let home = codex_home()?;
    let skills_dir = home.join("skills");
    let params = serde_json::json!({ "forceReload": force_reload });
    let res = server.request("skills/list", params, None).await?;
    let (items, errors) = parse_skill_items(&res)?;
    Ok(SkillsState {
        skills_dir: clean_path(&skills_dir),
        items: filter_local_skills(&items),
        errors,
    })
}

/// 展平 `skills/list` 响应：`data[].skills[]` 与 `data[].errors[]`；
/// 单个条目解析失败时跳过而不是整体失败。
fn parse_skill_items(
    res: &serde_json::Value,
) -> Result<(Vec<SkillListItem>, Vec<SkillErrorInfo>), String> {
    let entries = res
        .get("data")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "skills/list 响应缺少 data 数组".to_string())?;
    let mut items = Vec::new();
    let mut errors = Vec::new();
    for entry in entries {
        let skills = entry
            .get("skills")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| "skills/list 条目缺少 skills 数组".to_string())?;
        for skill in skills {
            if let Ok(item) = serde_json::from_value::<SkillListItem>(skill.clone()) {
                items.push(item);
            }
        }
        if let Some(error_list) = entry.get("errors").and_then(serde_json::Value::as_array) {
            for error in error_list {
                if let Ok(err) = serde_json::from_value::<SkillErrorInfo>(error.clone()) {
                    errors.push(err);
                }
            }
        }
    }
    Ok((items, errors))
}

/// 过滤出本地用户技能：`scope == "user"`、path 非空且不属于插件缓存
/// （plugins/cache）路径。不做 enabled 过滤。
fn filter_local_skills(items: &[SkillListItem]) -> Vec<SkillInfo> {
    let mut out = Vec::new();
    for item in items {
        if item.scope != "user" {
            continue;
        }
        let path = PathBuf::from(&item.path);
        if path.as_os_str().is_empty() {
            continue;
        }
        // 插件技能 scope 也是 user：按插件缓存路径排除
        if is_plugin_cache_path(&path) {
            continue;
        }
        out.push(SkillInfo {
            name: item.name.clone(),
            path: clean_path(&path),
            description: item.description.clone(),
            enabled: item.enabled,
            scope: item.scope.clone(),
        });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// 是否为插件缓存下的技能路径（大小写不敏感、统一分隔符、兼容 `\\?\` 前缀）。
fn is_plugin_cache_path(path: &Path) -> bool {
    norm_path_key(path).contains(r"\plugins\cache\")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list_item(name: &str, path: &str, enabled: bool, description: &str) -> SkillListItem {
        SkillListItem {
            name: name.to_string(),
            path: path.to_string(),
            description: description.to_string(),
            enabled,
            scope: "user".to_string(),
        }
    }

    fn with_scope(mut item: SkillListItem, scope: &str) -> SkillListItem {
        item.scope = scope.to_string();
        item
    }

    #[test]
    fn keeps_user_scope_skills_excluding_plugin_cache() {
        let items = vec![
            list_item(
                "pdf",
                "C:/apps/codex-ui/.codex/skills/pdf/SKILL.md",
                true,
                "读写 PDF 文件",
            ),
            list_item(
                "plugin-skill",
                "C:/apps/codex-ui/.codex/plugins/cache/x/skills/y/SKILL.md",
                true,
                "插件技能",
            ),
            list_item("outside", "D:/elsewhere/SKILL.md", true, "外部"),
        ];
        let out = filter_local_skills(&items);
        assert_eq!(out.len(), 2);
        assert!(out
            .iter()
            .any(|s| s.name == "pdf" && s.description == "读写 PDF 文件" && s.enabled));
        assert!(out.iter().any(|s| s.name == "outside"));
        assert!(!out.iter().any(|s| s.name == "plugin-skill"));
    }

    #[test]
    fn keeps_only_user_scope_skills() {
        let items = vec![
            list_item(
                "user-skill",
                "C:/apps/codex-ui/.codex/skills/user-skill/SKILL.md",
                true,
                "",
            ),
            with_scope(
                list_item(
                    "sys",
                    "C:/apps/codex-ui/.codex/skills/.system/sys/SKILL.md",
                    true,
                    "",
                ),
                "system",
            ),
            with_scope(
                list_item(
                    "repo-skill",
                    "C:/apps/codex-ui/.codex/skills/repo-skill/SKILL.md",
                    true,
                    "",
                ),
                "repo",
            ),
            with_scope(
                list_item(
                    "admin-skill",
                    "C:/apps/codex-ui/.codex/skills/admin-skill/SKILL.md",
                    true,
                    "",
                ),
                "admin",
            ),
        ];
        let out = filter_local_skills(&items);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "user-skill");
        assert_eq!(out[0].scope, "user");
    }

    #[test]
    fn keeps_user_scope_skills_outside_skills_dir() {
        let items = vec![list_item(
            "aspnet-core",
            "D:/elsewhere/aspnet-core/SKILL.md",
            true,
            "",
        )];
        let out = filter_local_skills(&items);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "aspnet-core");
    }

    #[test]
    fn keeps_verbatim_prefixed_user_skill() {
        let items = vec![list_item(
            "playwright",
            r"\\?\C:\Users\colleague\.codex\skills\playwright\SKILL.md",
            true,
            "",
        )];
        let out = filter_local_skills(&items);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "playwright");
    }

    #[test]
    fn excludes_plugin_skills_by_cache_path() {
        let items = vec![
            list_item(
                "p1",
                "C:/apps/codex-ui/.codex/plugins/cache/mp/p/1.0.0/skills/s/SKILL.md",
                true,
                "",
            ),
            list_item(
                "p2",
                r"\\?\C:\Users\x\.codex\plugins\cache\mp\p\1.0.0\skills\s\SKILL.md",
                true,
                "",
            ),
        ];
        let out = filter_local_skills(&items);
        assert!(out.is_empty());
    }

    #[test]
    fn excludes_empty_path_user_skill() {
        let items = vec![list_item("nopath", "", true, "")];
        let out = filter_local_skills(&items);
        assert!(out.is_empty());
    }

    #[test]
    fn keeps_user_dir_named_plugins_without_cache() {
        let items = vec![list_item(
            "ok",
            "C:/apps/codex-ui/.codex/skills/plugins/foo/SKILL.md",
            true,
            "",
        )];
        let out = filter_local_skills(&items);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "ok");
    }

    #[test]
    fn parses_errors_from_skills_list() {
        let res = serde_json::json!({
            "data": [{
                "cwd": "C:/apps/codex-ui",
                "skills": [],
                "errors": [{
                    "path": "C:/apps/codex-ui/.codex/skills/bad/SKILL.md",
                    "message": "missing field `description`"
                }]
            }]
        });
        let (items, errors) = parse_skill_items(&res).unwrap();
        assert!(items.is_empty());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].path, "C:/apps/codex-ui/.codex/skills/bad/SKILL.md");
        assert_eq!(errors[0].message, "missing field `description`");
    }

    #[cfg(windows)]
    #[test]
    fn path_compare_is_case_insensitive_on_windows() {
        let items = vec![list_item(
            "pdf",
            "c:/apps/codex-ui/.codex/skills/pdf/SKILL.md",
            false,
            "",
        )];
        let out = filter_local_skills(&items);
        assert_eq!(out.len(), 1);
        assert!(!out[0].enabled);
    }

    #[test]
    fn keeps_disabled_skills() {
        let items = vec![
            list_item(
                "on",
                "C:/apps/codex-ui/.codex/skills/on/SKILL.md",
                true,
                "",
            ),
            list_item(
                "off",
                "C:/apps/codex-ui/.codex/skills/off/SKILL.md",
                false,
                "",
            ),
        ];
        let out = filter_local_skills(&items);
        assert_eq!(out.len(), 2);
        assert!(out.iter().any(|s| s.name == "off" && !s.enabled));
    }

    #[test]
    fn uses_description_field_directly() {
        let items = vec![list_item(
            "a",
            "C:/apps/codex-ui/.codex/skills/a/SKILL.md",
            true,
            "SKILL.md 必填 description",
        )];
        let out = filter_local_skills(&items);
        assert_eq!(out[0].description, "SKILL.md 必填 description");
    }

    #[test]
    fn sorts_by_name_case_insensitive() {
        let items = vec![
            list_item(
                "beta",
                "C:/apps/codex-ui/.codex/skills/beta/SKILL.md",
                true,
                "",
            ),
            list_item(
                "Alpha",
                "C:/apps/codex-ui/.codex/skills/Alpha/SKILL.md",
                true,
                "",
            ),
        ];
        let out = filter_local_skills(&items);
        assert_eq!(out.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), [
            "Alpha",
            "beta"
        ]);
    }

    #[test]
    fn empty_list_returns_empty() {
        assert!(filter_local_skills(&[]).is_empty());
    }

    #[test]
    fn missing_path_is_excluded() {
        let items = vec![list_item("nopath", "", true, "")];
        assert!(filter_local_skills(&items).is_empty());
    }
}
