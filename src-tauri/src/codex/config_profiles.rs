//! 配置快照管理：在 `CODEX_HOME/codex-ui/<配置名>` 下保存/切换 codex 配置快照。
//!
//! 快照目录成员：
//! - `config.toml`：`CODEX_HOME/config.toml` 的原文复制，**绝不改写结构**；
//! - 模型目录文件：`model_catalog_json` 解析目标文件的内容，按源文件 basename 存放。
//!
//! 语义：
//! - 保存：把当前 codex-home 的 config.toml 与 model_catalog 值内容快照到
//!   `codex-ui/<配置名>/`（同名已存在则覆盖更新）。
//! - 应用：把快照 config.toml 覆盖回 codex-home/config.toml，并把快照模型目录文件
//!   内容写回 config 解析出的 model_catalog 目标文件（目标可能在 codex-home 之外）。
//! - 删除：删除该快照目录，仅允许位于 codex-home/codex-ui 之下。

use std::fs;
use std::path::{Path, PathBuf};

use crate::codex::model_config::{self, atomic_write};
use crate::codex::path_util::clean_path;

/// codex-home 下存放全部配置快照的根目录。
fn profiles_root(home: &Path) -> PathBuf {
    home.join("codex-ui")
}

/// 单个配置快照目录。
fn profile_dir(home: &Path, name: &str) -> PathBuf {
    profiles_root(home).join(name)
}

/// 从 `model_catalog_json` 原始值中取文件 basename（如 `catalog/custom.json` → `custom.json`）。
/// 值为空或无法取文件名时返回 None。
fn catalog_file_name(value: &str) -> Option<String> {
    let name = Path::new(value).file_name()?.to_str()?;
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// 校验配置名（Windows 文件夹名）：非空、不是 `.`/`..`、不含非法字符、
/// 不是保留设备名、不以点或空格开头/结尾。
fn validate_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("配置名不能为空".into());
    }
    if name == "." || name == ".." {
        return Err(format!("配置名「{name}」非法"));
    }
    if let Some(c) = name.chars().find(|c| r#"\/:*?"<>|"#.contains(*c)) {
        return Err(format!("配置名「{name}」包含非法字符「{c}」"));
    }
    let base = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6",
        "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7",
        "LPT8", "LPT9",
    ];
    if reserved.contains(&base.as_str()) {
        return Err(format!("配置名「{name}」是系统保留名"));
    }
    if name.starts_with('.') || name.starts_with(' ') || name.ends_with('.') || name.ends_with(' ') {
        return Err(format!("配置名「{name}」不能以点或空格开头/结尾"));
    }
    Ok(())
}

/// 列出全部配置快照名（按名排序）。
pub fn list() -> Result<Vec<String>, String> {
    list_in(&model_config::codex_home()?)
}

fn list_in(home: &Path) -> Result<Vec<String>, String> {
    let root = profiles_root(home);
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut names: Vec<String> = Vec::new();
    for entry in fs::read_dir(&root).map_err(|e| format!("读取配置目录失败: {e}"))? {
        let entry = entry.map_err(|e| format!("读取配置目录项失败: {e}"))?;
        let is_dir = entry
            .file_type()
            .map(|t| t.is_dir())
            .unwrap_or(false);
        if is_dir {
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

/// 新建/覆盖配置快照（把当前 codex-home 的 config.toml + model_catalog 内容快照）。
pub fn save(name: &str) -> Result<(), String> {
    save_in(&model_config::codex_home()?, name)
}

fn save_in(home: &Path, name: &str) -> Result<(), String> {
    validate_name(name)?;
    let dir = profile_dir(home, name);
    fs::create_dir_all(&dir).map_err(|e| format!("创建配置目录失败: {e}"))?;

    let config_path = model_config::config_path_in(home);
    let config_content = if config_path.is_file() {
        fs::read_to_string(&config_path).map_err(|e| format!("读取 config.toml 失败: {e}"))?
    } else {
        String::new()
    };
    atomic_write(&dir.join("config.toml"), &config_content)?;

    // 仅当 config 引用了 model_catalog 且目标文件存在时，按源文件名记录其内容。
    if let Some(value) = model_config::model_catalog_json_value(&config_path) {
        if let Some(name) = catalog_file_name(value.trim()) {
            let target = model_config::resolve_catalog_path(home, Some(value.as_str()));
            if target.is_file() {
                let content = fs::read_to_string(&target)
                    .map_err(|e| format!("读取模型目录文件失败: {e}"))?;
                if !content.is_empty() {
                    atomic_write(&dir.join(name), &content)?;
                }
            }
        }
    }
    Ok(())
}

/// 应用配置快照：覆盖 codex-home/config.toml，并把快照 models.json 写回 model_catalog 目标。
pub fn apply(name: &str) -> Result<(), String> {
    apply_in(&model_config::codex_home()?, name)
}

fn apply_in(home: &Path, name: &str) -> Result<(), String> {
    validate_name(name)?;
    let dir = profile_dir(home, name);
    let cfg_path = dir.join("config.toml");
    if !cfg_path.is_file() {
        return Err(format!("配置快照「{name}」不存在"));
    }
    let config_content =
        fs::read_to_string(&cfg_path).map_err(|e| format!("读取配置快照失败: {e}"))?;
    // 校验 TOML（非法拒绝）并原文写回 codex-home.
    model_config::save_config_in(home, &config_content)?;

    // 依据刚应用的 config 重新解析 model_catalog 目标并回写快照内容（相对/绝对/`~` 统一处理）。
    let config_path = model_config::config_path_in(home);
    if let Some(value) = model_config::model_catalog_json_value(&config_path) {
        if let Some(name) = catalog_file_name(value.trim()) {
            // 优先按源名读取；旧快照曾固定存为 models.json，找不到时回退兼容。
            let name_path = dir.join(name);
            let fallback_path = dir.join("models.json");
            let models_path = if name_path.is_file() {
                Some(name_path)
            } else if fallback_path.is_file() {
                Some(fallback_path)
            } else {
                None
            };
            if let Some(models_path) = models_path {
                let content = fs::read_to_string(&models_path)
                    .map_err(|e| format!("读取模型目录快照失败: {e}"))?;
                let target = model_config::resolve_catalog_path(home, Some(value.as_str()));
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
                }
                atomic_write(&target, &content)?;
            }
        }
    }
    Ok(())
}

/// 删除配置快照目录。
pub fn delete_profile(name: &str) -> Result<(), String> {
    delete_in(&model_config::codex_home()?, name)
}

fn delete_in(home: &Path, name: &str) -> Result<(), String> {
    validate_name(name)?;
    let root = profiles_root(home);
    let dir = profile_dir(home, name);
    // 双保险：名字已确认是单段，再确保落在 codex-ui 根之下、且父目录即根。
    if dir.parent() != Some(root.as_path()) || !dir.starts_with(&root) {
        return Err(format!("非法的配置路径「{name}」"));
    }
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| format!("删除配置快照失败: {e}"))?;
    }
    Ok(())
}

/// 列出配置快照目录下所有文件（config.toml 置顶 + 模型目录文件）的绝对路径，供编辑器打开。
pub fn open_files(name: &str) -> Result<Vec<String>, String> {
    open_files_in(&model_config::codex_home()?, name)
}

fn open_files_in(home: &Path, name: &str) -> Result<Vec<String>, String> {
    validate_name(name)?;
    let root = profiles_root(home);
    let dir = profile_dir(home, name);
    if dir.parent() != Some(root.as_path()) || !dir.starts_with(&root) {
        return Err(format!("非法的配置路径「{name}」"));
    }
    if !dir.is_dir() {
        return Err(format!("配置快照「{name}」不存在"));
    }
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|e| format!("读取配置快照目录失败: {e}"))? {
        let entry = entry.map_err(|e| format!("读取配置快照目录项失败: {e}"))?;
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            files.push(entry.path());
        }
    }
    // config.toml 置顶，其余按文件名排序。
    files.sort_by(|a, b| {
        let a_cfg = a.file_name().map(|f| f == "config.toml").unwrap_or(false);
        let b_cfg = b.file_name().map(|f| f == "config.toml").unwrap_or(false);
        b_cfg.cmp(&a_cfg).then_with(|| a.cmp(b))
    });
    Ok(files.into_iter().map(|p| clean_path(&p)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(home: &Path, rel: &str, content: &str) {
        let p = home.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&p, content).unwrap();
    }

    #[test]
    fn validate_name_rejects_invalid() {
        for bad in [
            "",
            "   ",
            ".",
            "..",
            "a/b",
            "a\\b",
            "a:b",
            "a?b",
            "con",
            "LPT3",
            "x.",
            " x",
            "x ",
        ] {
            assert!(validate_name(bad).is_err(), "should reject {bad:?}");
        }
        for ok in ["a", "abc", "my-config", "配置1", "model.2"] {
            assert!(validate_name(ok).is_ok(), "should accept {ok:?}");
        }
    }

    #[test]
    fn save_snapshots_config_and_catalog() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        write(home, "config.toml", "model = \"gpt\"\nmodel_catalog_json = \"models.json\"\n");
        write(home, "models.json", r#"{"models":[]}"#);

        save_in(home, "dev").unwrap();
        let snap = profile_dir(home, "dev");
        assert_eq!(
            fs::read_to_string(snap.join("config.toml")).unwrap(),
            "model = \"gpt\"\nmodel_catalog_json = \"models.json\"\n"
        );
        assert_eq!(
            fs::read_to_string(snap.join("models.json")).unwrap(),
            r#"{"models":[]}"#
        );
        assert!(list_in(home).unwrap() == vec!["dev".to_string()]);
    }

    #[test]
    fn save_skips_catalog_when_missing() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        write(home, "config.toml", "model = \"gpt\"\nmodel_catalog_json = \"models.json\"\n");
        save_in(home, "dev").unwrap();
        let snap = profile_dir(home, "dev");
        assert!(!snap.join("models.json").exists());
    }

    #[test]
    fn save_upserts_existing() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        write(home, "config.toml", "model = \"gpt\"\n");
        save_in(home, "dev").unwrap();
        write(home, "config.toml", "model = \"gpt2\"\n");
        save_in(home, "dev").unwrap();
        assert_eq!(
            fs::read_to_string(profile_dir(home, "dev").join("config.toml")).unwrap(),
            "model = \"gpt2\"\n"
        );
        assert_eq!(list_in(home).unwrap(), vec!["dev".to_string()]);
    }

    #[test]
    fn save_uses_source_basename_for_catalog() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        write(
            home,
            "config.toml",
            "model = \"gpt\"\nmodel_catalog_json = \"catalog/custom.json\"\n",
        );
        write(home, "catalog/custom.json", r#"{"models":[{"slug":"x"}]}"#);

        save_in(home, "dev").unwrap();
        let snap = profile_dir(home, "dev");
        // 按源文件名存储，而非固定 models.json
        assert!(snap.join("custom.json").is_file());
        assert!(!snap.join("models.json").exists());
        assert_eq!(
            fs::read_to_string(snap.join("custom.json")).unwrap(),
            r#"{"models":[{"slug":"x"}]}"#
        );
        // 应用写回解析目标
        write(home, "catalog/custom.json", r#"{"models":[{"slug":"changed"}]}"#);
        apply_in(home, "dev").unwrap();
        assert_eq!(
            fs::read_to_string(home.join("catalog").join("custom.json")).unwrap(),
            r#"{"models":[{"slug":"x"}]}"#
        );
    }

    #[test]
    fn open_files_lists_snapshot_files_config_first() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        write(home, "config.toml", "model = \"gpt\"\nmodel_catalog_json = \"custom.json\"\n");
        write(home, "custom.json", "[]");
        save_in(home, "dev").unwrap();

        let files = open_files_in(home, "dev").unwrap();
        assert_eq!(files.len(), 2);
        assert!(files[0].ends_with("config.toml"));
        assert!(files.iter().any(|f| f.ends_with("custom.json")));
        assert!(open_files_in(home, "nope").is_err());
    }

    #[test]
    fn apply_restores_config_and_catalog_at_relative_target() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        write(
            home,
            "config.toml",
            "model = \"old\"\nmodel_catalog_json = \"models.json\"\n",
        );
        write(home, "models.json", r#"{"models":[]}"#);
        // 保存后改坏 codex-home，再应用恢复
        save_in(home, "dev").unwrap();
        write(home, "config.toml", "model = \"broken\"\n");
        write(home, "models.json", r#"{"models":[]}"#);

        apply_in(home, "dev").unwrap();
        assert_eq!(
            fs::read_to_string(home.join("config.toml")).unwrap(),
            "model = \"old\"\nmodel_catalog_json = \"models.json\"\n"
        );
        assert_eq!(
            fs::read_to_string(home.join("models.json")).unwrap(),
            r#"{"models":[]}"#
        );
    }

    #[test]
    fn apply_restores_catalog_to_absolute_target_outside_home() {
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("codexhome");
        let outside = dir.path().join("shared");
        fs::create_dir_all(&outside).unwrap();
        let abs = outside.join("models.json");
        // config 指向 codex-home 外绝对路径
        let abs_toml = abs.to_string_lossy().replace('\\', "/");
        write(
            &home,
            "config.toml",
            &format!("model_catalog_json = \"{abs_toml}\"\n"),
        );
        fs::write(&abs, r#"{"models":[{"slug":"a"}]}"#).unwrap();

        save_in(&home, "ext").unwrap();
        // 外部文件被改，应用应回写
        fs::write(&abs, r#"{"models":[{"slug":"changed"}]}"#).unwrap();
        apply_in(&home, "ext").unwrap();
        assert_eq!(
            fs::read_to_string(&abs).unwrap(),
            r#"{"models":[{"slug":"a"}]}"#
        );
    }

    #[test]
    fn apply_fails_when_snapshot_missing() {
        let dir = TempDir::new().unwrap();
        assert!(apply_in(dir.path(), "nope").is_err());
    }

    #[test]
    fn delete_removes_snapshot_and_rejects_outside() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        write(home, "config.toml", "model = \"gpt\"\n");
        save_in(home, "dev").unwrap();
        assert!(profile_dir(home, "dev").is_dir());
        delete_in(home, "dev").unwrap();
        assert!(!profile_dir(home, "dev").exists());
        // 越界名拒绝
        assert!(delete_in(home, "../x").is_err());
        assert!(delete_in(home, "sub/x").is_err());
    }
}
