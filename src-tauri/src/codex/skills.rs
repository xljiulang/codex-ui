//! 技能管理：从 codex app-server 的 `skills/list` 聚合列表中过滤出本地技能。
//!
//! 本地用户技能判定：`scope == "user"`（0.149 协议中 scope 为必填枚举），
//! 并排除插件缓存路径（plugins/cache）。展示包含禁用技能（不做 enabled 过滤）。

use serde::{Deserialize, Serialize};
use std::fs;
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

/// 安装本地技能：把选中的 `SKILL.md` 所在文件夹复制/覆盖到 `{home}/skills/<文件夹名>`。
///
/// 校验规则（与 codex 技能目录约定一致）：
/// - 文件名必须为 `SKILL.md`（大小写不敏感）；
/// - SKILL.md frontmatter 的 `name` 与所在文件夹名一致（trim + 大小写不敏感）。
///
/// 目标目录已存在时按“覆盖更新”处理：先整体删除再复制，保证与源一致；
/// 源目录与目标目录为同一路径时视为已是最新，直接成功、不删除源。
pub fn install_in(source_md: &Path, home: &Path) -> Result<String, String> {
    let file_name = source_md
        .file_name()
        .ok_or_else(|| "无法确定所选文件的文件名".to_string())?
        .to_string_lossy()
        .into_owned();
    if !file_name.eq_ignore_ascii_case("SKILL.md") {
        return Err("请选择名为 SKILL.md 的文件".to_string());
    }
    let source_dir = source_md
        .parent()
        .ok_or_else(|| "无法确定 SKILL.md 所在文件夹".to_string())?
        .to_path_buf();
    let folder_name = source_dir
        .file_name()
        .ok_or_else(|| "无法确定 SKILL.md 所在文件夹名".to_string())?
        .to_string_lossy()
        .into_owned();
    if folder_name.is_empty() {
        return Err("无法确定 SKILL.md 所在文件夹名".to_string());
    }
    let name = parse_skill_name(source_md)?;
    if !name.trim().eq_ignore_ascii_case(&folder_name) {
        return Err(format!(
            "frontmatter name（{name}）与所在文件夹名（{folder_name}）不一致"
        ));
    }

    let skills_dir = home.join("skills");
    let target = skills_dir.join(&folder_name);
    // 同路径选择（从 CODEX_HOME/skills 内重新选择）不删不改，视为已是最新
    if paths_equivalent(&source_dir, &target) {
        return Ok(name);
    }
    if target.exists() {
        if target.is_dir() {
            fs::remove_dir_all(&target)
                .map_err(|e| format!("删除旧技能目录失败 {}: {e}", target.display()))?;
        } else {
            return Err(format!(
                "目标路径存在同名文件，无法安装技能: {}",
                target.display()
            ));
        }
    }
    fs::create_dir_all(&skills_dir)
        .map_err(|e| format!("创建技能目录失败 {}: {e}", skills_dir.display()))?;
    copy_dir_recursive(&source_dir, &target).map_err(|e| {
        format!(
            "复制技能目录失败 {} → {}: {e}",
            source_dir.display(),
            target.display()
        )
    })?;
    Ok(name)
}

/// 解析 SKILL.md frontmatter 的 `name` 字段。
fn parse_skill_name(skill_md: &Path) -> Result<String, String> {
    let text = fs::read_to_string(skill_md)
        .map_err(|e| format!("读取 SKILL.md 失败 {}: {e}", skill_md.display()))?;
    let yaml_text = frontmatter_yaml(&text)
        .ok_or_else(|| format!("SKILL.md 缺少 YAML frontmatter: {}", skill_md.display()))?;
    let doc: serde_yaml::Value = serde_yaml::from_str(yaml_text)
        .map_err(|e| format!("SKILL.md frontmatter 解析失败: {e}"))?;
    let name = doc
        .get("name")
        .and_then(serde_yaml::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            format!(
                "SKILL.md frontmatter 缺少非空 name 字段: {}",
                skill_md.display()
            )
        })?;
    Ok(name.to_string())
}

/// 截取 frontmatter YAML 文本（支持 CRLF 与 UTF-8 BOM）。
fn frontmatter_yaml(text: &str) -> Option<&str> {
    let text = text.trim_start_matches('\u{feff}');
    let rest = text
        .strip_prefix("---\r\n")
        .or_else(|| text.strip_prefix("---\n"))?;
    let end = rest.find("\n---")?;
    Some(&rest[..end])
}

/// 目标已存在时以 canonical 路径比较源/目标是否为同一目录。
fn paths_equivalent(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// 递归复制目录（含子目录与隐藏文件）。
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// 删除本地技能：删除 SKILL.md 所在目录（不限 CODEX_HOME/skills）。
pub fn remove_in(skill_md: &Path) -> Result<String, String> {
    let file_name = skill_md
        .file_name()
        .ok_or_else(|| "无法确定所选文件的文件名".to_string())?
        .to_string_lossy()
        .into_owned();
    if !file_name.eq_ignore_ascii_case("SKILL.md") {
        return Err("只能删除名为 SKILL.md 的技能文件".to_string());
    }
    let skill_dir = skill_md
        .parent()
        .ok_or_else(|| "无法确定技能所在目录".to_string())?;
    if skill_dir.file_name().is_none() || skill_dir.parent().is_none() {
        return Err("技能目录位于文件系统根层级，拒绝删除".to_string());
    }
    if !skill_dir.is_dir() {
        return Err(format!("技能目录不存在: {}", skill_dir.display()));
    }
    let folder_name = skill_dir
        .file_name()
        .ok_or_else(|| "无法确定技能目录名".to_string())?
        .to_string_lossy()
        .into_owned();
    fs::remove_dir_all(skill_dir)
        .map_err(|e| format!("删除技能目录失败 {}: {e}", skill_dir.display()))?;
    Ok(folder_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_skill(dir: &Path, name: &str, extra: Option<&str>) {
        let content = format!("---\nname: {name}\ndescription: 测试技能\n---\n\n正文\n");
        fs::write(dir.join("SKILL.md"), content).unwrap();
        if let Some(file) = extra {
            fs::write(dir.join(file), "payload").unwrap();
        }
    }

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
        assert_eq!(
            errors[0].path,
            "C:/apps/codex-ui/.codex/skills/bad/SKILL.md"
        );
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
            list_item("on", "C:/apps/codex-ui/.codex/skills/on/SKILL.md", true, ""),
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
        assert_eq!(
            out.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["Alpha", "beta"]
        );
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

    #[test]
    fn rejects_file_that_is_not_skill_md() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("foo");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("readme.md"), "# hi").unwrap();
        let err = install_in(&src.join("readme.md"), tmp.path()).unwrap_err();
        assert!(err.contains("SKILL.md"), "{err}");
    }

    #[test]
    fn rejects_missing_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("bare");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("SKILL.md"), "# 无 frontmatter").unwrap();
        let err = install_in(&src.join("SKILL.md"), tmp.path()).unwrap_err();
        assert!(err.contains("frontmatter"), "{err}");
    }

    #[test]
    fn rejects_frontmatter_without_name() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("noname");
        fs::create_dir_all(&src).unwrap();
        fs::write(
            src.join("SKILL.md"),
            "---\ndescription: 缺 name\n---\n\n正文\n",
        )
        .unwrap();
        let err = install_in(&src.join("SKILL.md"), tmp.path()).unwrap_err();
        assert!(err.contains("name"), "{err}");
    }

    #[test]
    fn rejects_name_folder_mismatch() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("wanted");
        fs::create_dir_all(&src).unwrap();
        fs::write(
            src.join("SKILL.md"),
            "---\nname: other\ndescription: 测试\n---\n",
        )
        .unwrap();
        let err = install_in(&src.join("SKILL.md"), tmp.path()).unwrap_err();
        assert!(err.contains("不一致"), "{err}");
    }

    #[test]
    fn installs_skill_folder_with_extra_files() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join(".codex");
        let src = tmp.path().join("demo-skill");
        fs::create_dir_all(src.join("scripts")).unwrap();
        write_skill(&src, "demo-skill", Some("scripts/run.ps1"));
        let name = install_in(&src.join("SKILL.md"), &home).unwrap();
        assert_eq!(name, "demo-skill");
        let target = home.join("skills").join("demo-skill");
        assert!(target.join("SKILL.md").is_file());
        assert!(target.join("scripts").join("run.ps1").is_file());
    }

    #[test]
    fn overwrite_replaces_existing_folder_and_removes_stale_files() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join(".codex");
        let src = tmp.path().join("skill-x");
        fs::create_dir_all(&src).unwrap();
        write_skill(&src, "skill-x", Some("new.txt"));
        let target = home.join("skills").join("skill-x");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("stale.txt"), "old").unwrap();
        install_in(&src.join("SKILL.md"), &home).unwrap();
        assert!(target.join("new.txt").is_file());
        assert!(!target.join("stale.txt").exists());
    }

    #[test]
    fn same_source_and_target_is_noop() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join(".codex");
        let target = home.join("skills").join("same");
        fs::create_dir_all(&target).unwrap();
        write_skill(&target, "same", Some("keep.txt"));
        install_in(&target.join("SKILL.md"), &home).unwrap();
        assert!(target.join("keep.txt").is_file());
    }

    #[test]
    fn accepts_case_insensitive_name_and_filename() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join(".codex");
        let src = tmp.path().join("MySkill");
        fs::create_dir_all(&src).unwrap();
        fs::write(
            src.join("skill.md"),
            "---\nname: \"myskill\"\ndescription: 测试\n---\n",
        )
        .unwrap();
        let name = install_in(&src.join("skill.md"), &home).unwrap();
        assert_eq!(name, "myskill");
        assert!(home
            .join("skills")
            .join("MySkill")
            .join("skill.md")
            .is_file());
    }

    #[test]
    fn remove_deletes_skill_dir_with_nested_files() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path().join("del-skill");
        fs::create_dir_all(skill_dir.join("sub")).unwrap();
        write_skill(&skill_dir, "del-skill", Some("sub/data.txt"));
        let skill_md = skill_dir.join("SKILL.md");
        let name = remove_in(&skill_md).unwrap();
        assert_eq!(name, "del-skill");
        assert!(!skill_dir.exists());
        assert!(!tmp.path().join("del-skill").join("SKILL.md").exists());
    }

    #[test]
    fn remove_rejects_non_skill_md() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("keep");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("readme.md"), "# hi").unwrap();
        let err = remove_in(&dir.join("readme.md")).unwrap_err();
        assert!(err.contains("SKILL.md"), "{err}");
        assert!(dir.exists());
    }

    #[test]
    fn remove_rejects_missing_directory() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("missing-skill").join("SKILL.md");
        let err = remove_in(&missing).unwrap_err();
        assert!(err.contains("不存在"), "{err}");
    }

    #[test]
    fn remove_rejects_path_without_parent_folder() {
        let err = remove_in(Path::new("SKILL.md")).unwrap_err();
        assert!(err.contains("目录") || err.contains("根"), "{err}");
    }
}
