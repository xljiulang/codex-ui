//! 技能管理：从 codex app-server 的 `skills/list` 聚合列表中过滤出本地技能。
//!
//! 本地用户技能判定：`scope == "user"`（旧版本缺失 scope 时回退路径判定），且
//! SKILL.md 路径位于 `{CODEX_HOME}/skills` 目录内、相对路径不以 `.` 开头
//! （跳过 `.system` 等系统目录）。展示包含禁用技能（不做 enabled 过滤）。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::app_server::CodexServer;
use super::model_config::codex_home;

/// 单个本地技能条目。
#[derive(Debug, Clone, Serialize)]
pub struct SkillInfo {
    pub name: String,
    /// SKILL.md 的绝对路径。
    pub path: String,
    pub description: String,
    pub enabled: bool,
    /// skills/list 返回的 scope（user/repo/system/admin；旧版本缺失时为空串）。
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

/// `skills/list` 返回的单个技能条目（只声明需要的字段，缺省字段给默认值）。
#[derive(Debug, Clone, Deserialize)]
struct SkillListItem {
    name: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    description: String,
    /// 防御旧形状：部分实现可能用 `desc` 提供描述。
    #[serde(default)]
    desc: String,
    #[serde(default, rename = "shortDescription")]
    short_description: Option<String>,
    #[serde(default)]
    interface: Option<SkillInterface>,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    scope: String,
}

/// `skills/list` 条目中的 interface 字段（取 shortDescription）。
#[derive(Debug, Clone, Deserialize)]
struct SkillInterface {
    #[serde(default, rename = "shortDescription")]
    short_description: Option<String>,
}

/// 读取本地技能列表：请求 `skills/list` 后按路径过滤出 `{CODEX_HOME}/skills` 下的技能。
pub async fn read_state(server: &CodexServer, force_reload: bool) -> Result<SkillsState, String> {
    let home = codex_home()?;
    let skills_dir = home.join("skills");
    let params = serde_json::json!({ "forceReload": force_reload });
    let res = server.request("skills/list", params, None).await?;
    let (items, errors) = parse_skill_items(&res)?;
    Ok(SkillsState {
        skills_dir: skills_dir.to_string_lossy().into_owned(),
        items: filter_local_skills(&items, &skills_dir),
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

/// 过滤出本地用户技能：`scope == "user"`（scope 缺失时回退路径判定），
/// path 非空且不属于插件缓存（plugins/cache）路径。不做 enabled 过滤。
fn filter_local_skills(items: &[SkillListItem], skills_dir: &Path) -> Vec<SkillInfo> {
    let mut out = Vec::new();
    for item in items {
        if !item.scope.is_empty() && item.scope != "user" {
            continue;
        }
        let path = PathBuf::from(&item.path);
        if path.as_os_str().is_empty() {
            continue;
        }
        if !item.scope.is_empty() {
            // 插件技能 scope 也是 user：按插件缓存路径排除
            if is_plugin_cache_path(&path) {
                continue;
            }
        } else {
            // 旧版本无 scope：回退路径判定（位于 skills_dir 内且不以 `.` 开头）
            let Some(rel) = relative_under(&path, skills_dir) else {
                continue;
            };
            if rel
                .components()
                .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
            {
                continue;
            }
        }
        let description = if !item.description.is_empty() {
            item.description.clone()
        } else if let Some(short) = item
            .interface
            .as_ref()
            .and_then(|i| i.short_description.as_ref())
            .or(item.short_description.as_ref())
        {
            short.clone()
        } else if !item.desc.is_empty() {
            item.desc.clone()
        } else {
            String::new()
        };
        out.push(SkillInfo {
            name: item.name.clone(),
            path: item.path.clone(),
            description,
            enabled: item.enabled,
            scope: item.scope.clone(),
        });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// 是否为插件缓存下的技能路径（大小写不敏感、统一分隔符、兼容 `\\?\` 前缀）。
fn is_plugin_cache_path(path: &Path) -> bool {
    let s = path.to_string_lossy().replace('/', "\\").to_lowercase();
    let s = if let Some(rest) = s.strip_prefix(r"\\?\unc\") {
        format!("\\\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s
    };
    s.contains(r"\plugins\cache\")
}

/// 返回 path 相对 dir 的相对路径；path 不在 dir 内（或等于 dir）时返回 None。
/// Windows 下路径比较大小写不敏感、统一分隔符。
fn relative_under(path: &Path, dir: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let dir_key = path_key(dir);
        let child_key = path_key(path);
        let prefix = format!("{}\\", dir_key);
        let rest = child_key.strip_prefix(&prefix)?;
        if rest.is_empty() {
            return None;
        }
        Some(PathBuf::from(rest.replace('\\', "/")))
    }
    #[cfg(not(windows))]
    {
        path.strip_prefix(dir)
            .ok()
            .filter(|r| !r.as_os_str().is_empty())
            .map(PathBuf::from)
    }
}

#[cfg(windows)]
fn path_key(p: &Path) -> String {
    p.components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect::<Vec<_>>()
        .join("\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list_item(name: &str, path: &str, enabled: bool, description: &str) -> SkillListItem {
        SkillListItem {
            name: name.to_string(),
            path: path.to_string(),
            description: description.to_string(),
            desc: String::new(),
            short_description: None,
            interface: None,
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
        let dir = Path::new("C:/apps/codex-ui/.codex/skills");
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
        let out = filter_local_skills(&items, dir);
        assert_eq!(out.len(), 2);
        assert!(out
            .iter()
            .any(|s| s.name == "pdf" && s.description == "读写 PDF 文件" && s.enabled));
        assert!(out.iter().any(|s| s.name == "outside"));
        assert!(!out.iter().any(|s| s.name == "plugin-skill"));
    }

    #[test]
    fn keeps_only_user_scope_skills() {
        let dir = Path::new("C:/apps/codex-ui/.codex/skills");
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
        let out = filter_local_skills(&items, dir);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "user-skill");
        assert_eq!(out[0].scope, "user");
    }

    #[test]
    fn falls_back_to_path_when_scope_missing() {
        let dir = Path::new("C:/apps/codex-ui/.codex/skills");
        let items = vec![
            with_scope(
                list_item(
                    "ok",
                    "C:/apps/codex-ui/.codex/skills/ok/SKILL.md",
                    true,
                    "",
                ),
                "",
            ),
            with_scope(
                list_item(
                    "sys",
                    "C:/apps/codex-ui/.codex/skills/.system/sys/SKILL.md",
                    true,
                    "",
                ),
                "",
            ),
            with_scope(
                list_item(
                    "outside",
                    "D:/elsewhere/SKILL.md",
                    true,
                    "",
                ),
                "",
            ),
        ];
        let out = filter_local_skills(&items, dir);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "ok");
    }

    #[test]
    fn keeps_user_scope_skills_outside_skills_dir() {
        let dir = Path::new("C:/apps/codex-ui/.codex/skills");
        let items = vec![list_item(
            "aspnet-core",
            "D:/elsewhere/aspnet-core/SKILL.md",
            true,
            "",
        )];
        let out = filter_local_skills(&items, dir);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "aspnet-core");
    }

    #[test]
    fn keeps_verbatim_prefixed_user_skill() {
        let dir = Path::new("C:/apps/codex-ui/.codex/skills");
        let items = vec![list_item(
            "playwright",
            r"\\?\C:\Users\colleague\.codex\skills\playwright\SKILL.md",
            true,
            "",
        )];
        let out = filter_local_skills(&items, dir);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "playwright");
    }

    #[test]
    fn excludes_plugin_skills_by_cache_path() {
        let dir = Path::new("C:/apps/codex-ui/.codex/skills");
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
        let out = filter_local_skills(&items, dir);
        assert!(out.is_empty());
    }

    #[test]
    fn excludes_empty_path_user_skill() {
        let dir = Path::new("C:/apps/codex-ui/.codex/skills");
        let items = vec![list_item("nopath", "", true, "")];
        let out = filter_local_skills(&items, dir);
        assert!(out.is_empty());
    }

    #[test]
    fn keeps_user_dir_named_plugins_without_cache() {
        let dir = Path::new("C:/apps/codex-ui/.codex/skills");
        let items = vec![list_item(
            "ok",
            "C:/apps/codex-ui/.codex/skills/plugins/foo/SKILL.md",
            true,
            "",
        )];
        let out = filter_local_skills(&items, dir);
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

    #[test]
    fn skips_dot_segments_in_fallback() {
        let dir = Path::new("C:/apps/codex-ui/.codex/skills");
        let items = vec![
            with_scope(
                list_item(
                    "sys",
                    "C:/apps/codex-ui/.codex/skills/.system/sys/SKILL.md",
                    true,
                    "系统技能",
                ),
                "",
            ),
            with_scope(
                list_item(
                    "ok",
                    "C:/apps/codex-ui/.codex/skills/ok/SKILL.md",
                    true,
                    "",
                ),
                "",
            ),
        ];
        let out = filter_local_skills(&items, dir);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "ok");
    }

    #[cfg(windows)]
    #[test]
    fn path_compare_is_case_insensitive_on_windows() {
        let dir = Path::new("C:/Apps/Codex-UI/.codex/Skills");
        let items = vec![list_item(
            "pdf",
            "c:/apps/codex-ui/.codex/skills/pdf/SKILL.md",
            false,
            "",
        )];
        let out = filter_local_skills(&items, dir);
        assert_eq!(out.len(), 1);
        assert!(!out[0].enabled);
    }

    #[test]
    fn keeps_disabled_skills() {
        let dir = Path::new("C:/apps/codex-ui/.codex/skills");
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
        let out = filter_local_skills(&items, dir);
        assert_eq!(out.len(), 2);
        assert!(out.iter().any(|s| s.name == "off" && !s.enabled));
    }

    #[test]
    fn description_precedence() {
        let dir = Path::new("C:/apps/codex-ui/.codex/skills");
        let mut from_interface = list_item(
            "a",
            "C:/apps/codex-ui/.codex/skills/a/SKILL.md",
            true,
            "",
        );
        from_interface.interface = Some(SkillInterface {
            short_description: Some("interface 短描述".to_string()),
        });
        let mut from_desc = list_item(
            "b",
            "C:/apps/codex-ui/.codex/skills/b/SKILL.md",
            true,
            "",
        );
        from_desc.desc = "desc 字段".to_string();
        let out = filter_local_skills(&[from_interface, from_desc], dir);
        assert_eq!(out[0].description, "interface 短描述");
        assert_eq!(out[1].description, "desc 字段");
    }

    #[test]
    fn sorts_by_name_case_insensitive() {
        let dir = Path::new("C:/apps/codex-ui/.codex/skills");
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
        let out = filter_local_skills(&items, dir);
        assert_eq!(out.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), [
            "Alpha",
            "beta"
        ]);
    }

    #[test]
    fn empty_list_returns_empty() {
        let dir = Path::new("C:/apps/codex-ui/.codex/skills");
        assert!(filter_local_skills(&[], dir).is_empty());
    }

    #[test]
    fn missing_path_is_excluded() {
        let dir = Path::new("C:/apps/codex-ui/.codex/skills");
        let items = vec![list_item("nopath", "", true, "")];
        assert!(filter_local_skills(&items, dir).is_empty());
    }
}
