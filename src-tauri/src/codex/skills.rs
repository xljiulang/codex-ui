//! 技能管理：扫描 `{CODEX_HOME}/skills` 文件夹下的本地技能。
//!
//! 每个子文件夹对应一个技能，读取其中的 `SKILL.md` YAML frontmatter
//! （name / description，缺失时回退文件夹名/空描述）；`.` 开头的系统文件夹跳过。

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

use super::model_config::codex_home;

/// 单个本地技能条目。
#[derive(Debug, Clone, Serialize)]
pub struct SkillInfo {
    pub name: String,
    /// SKILL.md 的绝对路径。
    pub path: String,
    pub description: String,
}

/// `skills_read` 的返回结构。
#[derive(Debug, Clone, Serialize)]
pub struct SkillsState {
    /// 技能根目录（CODEX_HOME/skills）。
    pub skills_dir: String,
    pub items: Vec<SkillInfo>,
}

/// 读取本地技能列表（真实 CODEX_HOME）。
pub fn read_state() -> Result<SkillsState, String> {
    let home = codex_home()?;
    Ok(scan_skills_dir(&home.join("skills")))
}

/// 扫描指定技能目录（目录不存在时返回空列表）。
fn scan_skills_dir(skills_dir: &Path) -> SkillsState {
    let mut items = Vec::new();
    if skills_dir.is_dir() {
        let Ok(entries) = fs::read_dir(skills_dir) else {
            return SkillsState {
                skills_dir: skills_dir.to_string_lossy().into_owned(),
                items,
            };
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let folder_name = entry.file_name().to_string_lossy().into_owned();
            if folder_name.starts_with('.') {
                continue;
            }
            let skill_md = path.join("SKILL.md");
            if !skill_md.is_file() {
                continue;
            }
            let (name, description) =
                read_frontmatter(&skill_md).unwrap_or((folder_name, String::new()));
            items.push(SkillInfo {
                name,
                path: skill_md.to_string_lossy().into_owned(),
                description,
            });
        }
    }
    items.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    SkillsState {
        skills_dir: skills_dir.to_string_lossy().into_owned(),
        items,
    }
}

/// 读取 SKILL.md 的 YAML frontmatter：返回 (name, description)。
/// frontmatter 缺失或解析失败时返回 None（由调用方回退）。
fn read_frontmatter(path: &std::path::Path) -> Option<(String, String)> {
    let text = fs::read_to_string(path).ok()?;
    let text = text.strip_prefix("\u{feff}").unwrap_or(&text);
    let trimmed = text.trim_start();
    let after_open = trimmed.strip_prefix("---")?;
    let end = after_open.find("\n---")?;
    let front = &after_open[..end];
    let mut name = String::new();
    let mut description = String::new();
    for line in front.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("name:") {
            name = unquote(v.trim());
        } else if let Some(v) = line.strip_prefix("description:") {
            description = unquote(v.trim());
        }
    }
    if name.is_empty() {
        None
    } else {
        Some((name, description))
    }
}

/// 去掉 YAML 标量首尾引号（支持单/双引号）。
fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 {
        let bytes = s.as_bytes();
        if (bytes[0] == b'"' && bytes[s.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[s.len() - 1] == b'\'')
        {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_skill(dir: &std::path::Path, folder: &str, frontmatter: &str) {
        let folder_path = dir.join(folder);
        fs::create_dir_all(&folder_path).unwrap();
        fs::write(folder_path.join("SKILL.md"), frontmatter).unwrap();
    }

    #[test]
    fn scans_skills_folder_with_frontmatter() {
        let dir = TempDir::new().unwrap();
        make_skill(
            dir.path(),
            "alpha",
            "---\nname: \"alpha\"\ndescription: \"首个技能\"\n---\n\n# Alpha\n",
        );
        make_skill(
            dir.path(),
            "beta",
            "---\nname: 'beta'\ndescription: '第二个'\n---\n",
        );
        let state = scan_skills_dir(dir.path());
        assert_eq!(state.items.len(), 2);
        assert_eq!(state.items[0].name, "alpha");
        assert_eq!(state.items[0].description, "首个技能");
        assert!(state.items[0].path.ends_with("SKILL.md"));
        assert_eq!(state.items[1].name, "beta");
        assert_eq!(state.items[1].description, "第二个");
    }

    #[test]
    fn skips_dot_folders_and_folders_without_skill_md() {
        let dir = TempDir::new().unwrap();
        make_skill(dir.path(), ".system", "---\nname: sys\n---\n");
        make_skill(dir.path(), "ok", "---\nname: ok\n---\n");
        fs::create_dir_all(dir.path().join("empty")).unwrap();
        fs::write(dir.path().join("note.txt"), "x").unwrap();

        let state = scan_skills_dir(dir.path());
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].name, "ok");
    }

    #[test]
    fn falls_back_when_frontmatter_missing() {
        let dir = TempDir::new().unwrap();
        make_skill(dir.path(), "no-frontmatter", "# No Frontmatter\n");
        let state = scan_skills_dir(dir.path());
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].name, "no-frontmatter");
        assert_eq!(state.items[0].description, "");
    }

    #[test]
    fn missing_dir_returns_empty_list() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("nope");
        let state = scan_skills_dir(&missing);
        assert!(state.items.is_empty());
    }
}
