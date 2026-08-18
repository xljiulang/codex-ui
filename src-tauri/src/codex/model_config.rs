//! 模型配置读写：管理 `CODEX_HOME/config.toml` 与 `model_catalog_json` 目标文件。
//!
//! CODEX_HOME 优先取 codex-ui 进程自身的 `CODEX_HOME` 环境变量，未设置时降级为
//! `%USERPROFILE%\.codex`；codex 子进程不再被显式注入 CODEX_HOME，因此继承同一值，
//! 两者保持一致。config.toml 与 model_catalog_json 目标文件均按用户编辑内容整文件
//! 原样读写：config 保存前校验 TOML 可解析，不注入任何受管键；model_catalog_json
//! 目标路径完全以 config.toml 中实际配置的值为准，不提供默认文件名。
//! 读取时若文件缺失会自动创建：config.toml 创建空文件，model_catalog_json 目标
//! 创建 `{"models":[]}`（该键未配置时不创建）。

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

use toml_edit::DocumentMut;

/// 自动创建 model_catalog_json 目标文件时的默认内容（空模型目录，与 codex 目录格式一致）。
pub const DEFAULT_MODEL_CATALOG_CONTENT: &str = "{\"models\":[]}";

/// `model_config_read` 的返回结构：config 整文件内容 + model_catalog_json 目标状态。
#[derive(Debug, Clone, Serialize)]
pub struct ModelConfigState {
    pub config_path: String,
    pub config_exists: bool,
    pub config_content: String,
    /// model_catalog_json 目标文件路径；config 未配置该键时为空字符串。
    pub model_catalog_path: String,
    pub model_catalog_exists: bool,
    pub model_catalog: String,
}

/// 应用实际使用的 CODEX_HOME：优先 `CODEX_HOME` 环境变量，否则 `%USERPROFILE%\.codex`
/// （与未显式注入环境变量的 codex 子进程一致）。
pub fn codex_home() -> Result<PathBuf, String> {
    if let Some(home) = std::env::var_os("CODEX_HOME") {
        if !home.is_empty() {
            return Ok(PathBuf::from(home));
        }
    }
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        if !profile.is_empty() {
            return Ok(Path::new(&profile).join(".codex"));
        }
    }
    Err("无法定位 CODEX_HOME（未设置 CODEX_HOME 且找不到 %USERPROFILE%）".to_string())
}

fn config_path_in(home: &Path) -> PathBuf {
    home.join("config.toml")
}

/// 解析 config.toml 中的 model_catalog_json：字符串值原样返回；
/// 文件缺失、解析失败、未配置或值为空白时返回 None（不提供默认文件名）。
fn model_catalog_json_value(config_path: &Path) -> Option<String> {
    let text = fs::read_to_string(config_path).ok()?;
    let doc = text.parse::<DocumentMut>().ok()?;
    let value = doc.get("model_catalog_json").and_then(|v| v.as_str())?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// model_catalog_json 目标路径：绝对路径原样使用，相对路径基于 CODEX_HOME 拼接。
fn model_catalog_path_in(home: &Path, value: &str) -> PathBuf {
    let p = Path::new(value);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        home.join(p)
    }
}

/// 读取模型配置状态（真实 CODEX_HOME）。
pub fn read_state() -> Result<ModelConfigState, String> {
    read_state_in(&codex_home()?)
}

fn read_state_in(home: &Path) -> Result<ModelConfigState, String> {
    let config_path = config_path_in(home);
    if !config_path.is_file() {
        // 不存在则创建：保证设置页可直接编辑
        atomic_write(&config_path, "")?;
    }
    let config_content = fs::read_to_string(&config_path)
        .map_err(|e| format!("读取 config.toml 失败: {e}"))?;
    let catalog_target = model_catalog_json_value(&config_path)
        .map(|v| model_catalog_path_in(home, &v));
    let (model_catalog_exists, model_catalog) = match catalog_target.as_deref() {
        Some(p) if p.is_file() => (
            true,
            fs::read_to_string(p).map_err(|e| format!("读取模型目录文件失败: {e}"))?,
        ),
        Some(p) => {
            // 目标缺失：创建默认目录文件（空模型目录），再返回其内容
            atomic_write(p, DEFAULT_MODEL_CATALOG_CONTENT)?;
            (
                true,
                fs::read_to_string(p).map_err(|e| format!("读取模型目录文件失败: {e}"))?,
            )
        }
        _ => (false, String::new()),
    };
    Ok(ModelConfigState {
        config_path: config_path.to_string_lossy().into_owned(),
        config_exists: true,
        config_content,
        model_catalog_path: catalog_target
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        model_catalog_exists,
        model_catalog,
    })
}

/// 保存 config.toml（真实 CODEX_HOME）：整文件原文写入，不注入任何受管键；
/// 保存前校验 TOML 可解析，非法内容拒绝保存（空内容视为合法空文档）。
pub fn save_config(content: &str) -> Result<(), String> {
    save_config_in(&codex_home()?, content)
}

fn save_config_in(home: &Path, content: &str) -> Result<(), String> {
    content
        .parse::<DocumentMut>()
        .map_err(|e| format!("config 不是合法 TOML: {e}"))?;
    atomic_write(&config_path_in(home), content)
}

/// 保存 model_catalog_json 目标文件：目标路径从 config.toml 实时解析；
/// 内容去空白后必须非空且为合法 JSON，合法后按原文原样写入。
pub fn save_model_catalog(content: &str) -> Result<(), String> {
    save_model_catalog_in(&codex_home()?, content)
}

fn save_model_catalog_in(home: &Path, content: &str) -> Result<(), String> {
    let Some(value) = model_catalog_json_value(&config_path_in(home)) else {
        return Err("config.toml 未配置 model_catalog_json".to_string());
    };
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("模型目录内容不能为空".to_string());
    }
    serde_json::from_str::<serde_json::Value>(trimmed)
        .map_err(|e| format!("模型目录不是合法 JSON: {e}"))?;
    atomic_write(&model_catalog_path_in(home, &value), content)
}

/// 先写临时文件再替换，避免写入中途崩溃留下截断文件（与 settings.rs 同策略）。
pub(crate) fn atomic_write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("tmp");
    let tmp = path.with_extension(format!("{ext}.tmp"));
    fs::write(&tmp, text).map_err(|e| format!("写入临时文件失败: {e}"))?;
    if path.exists() {
        fs::remove_file(path).map_err(|e| format!("删除旧文件失败: {e}"))?;
    }
    fs::rename(&tmp, path).map_err(|e| format!("替换文件失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// 环境变量读写不是线程安全的，测试串行化；结束后恢复原值。
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_envs(pairs: &[(&str, Option<&str>)], f: impl FnOnce()) {
        let _guard = ENV_LOCK.lock().unwrap();
        let old: Vec<(String, Option<std::ffi::OsString>)> = pairs
            .iter()
            .map(|(k, _)| (k.to_string(), std::env::var_os(k)))
            .collect();
        for (k, v) in pairs {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
        f();
        for (k, v) in old {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }

    /// 统一路径分隔符后再比较（Windows 下 `\` 与 `/` 等价）。
    fn norm_path(s: &str) -> String {
        s.replace('\\', "/")
    }

    #[test]
    fn codex_home_prefers_env_over_userprofile() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("custom-home");
        let profile = tmp.path().join("profile");
        let home_s = home.to_string_lossy().into_owned();
        let profile_s = profile.to_string_lossy().into_owned();
        with_envs(
            &[
                ("CODEX_HOME", Some(&home_s)),
                ("USERPROFILE", Some(&profile_s)),
            ],
            || {
                assert_eq!(codex_home().unwrap(), home);
            },
        );
    }

    #[test]
    fn codex_home_falls_back_to_userprofile_dot_codex() {
        let tmp = TempDir::new().unwrap();
        let profile = tmp.path().join("profile");
        let profile_s = profile.to_string_lossy().into_owned();
        with_envs(
            &[("CODEX_HOME", None), ("USERPROFILE", Some(&profile_s))],
            || {
                assert_eq!(codex_home().unwrap(), profile.join(".codex"));
            },
        );
    }

    #[test]
    fn codex_home_errors_when_neither_set() {
        with_envs(&[("CODEX_HOME", None), ("USERPROFILE", None)], || {
            assert!(codex_home().is_err());
        });
    }

    #[test]
    fn read_state_creates_missing_config_and_no_catalog_target() {
        let dir = TempDir::new().unwrap();
        let state = read_state_in(dir.path()).unwrap();
        assert!(state.config_exists);
        assert_eq!(state.config_content, "");
        assert!(config_path_in(dir.path()).is_file());
        assert_eq!(fs::read_to_string(config_path_in(dir.path())).unwrap(), "");
        assert_eq!(state.model_catalog_path, "");
        assert!(!state.model_catalog_exists);
        assert_eq!(state.model_catalog, "");
    }

    #[test]
    fn read_state_reads_config_verbatim_and_ignores_unparseable() {
        let dir = TempDir::new().unwrap();
        let path = config_path_in(dir.path());
        fs::write(&path, "not valid toml = = [").unwrap();

        let state = read_state_in(dir.path()).unwrap();
        assert!(state.config_exists);
        assert_eq!(state.config_content, "not valid toml = = [");
        // 解析失败 → model_catalog_json 无目标
        assert_eq!(state.model_catalog_path, "");
        assert!(!state.model_catalog_exists);
    }

    #[test]
    fn read_state_resolves_relative_model_catalog_against_home() {
        let dir = TempDir::new().unwrap();
        fs::write(
            config_path_in(dir.path()),
            "model_catalog_json = \"catalog/custom.json\"",
        )
        .unwrap();
        let target = dir.path().join("catalog").join("custom.json");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, r#"{"models":[]}"#).unwrap();

        let state = read_state_in(dir.path()).unwrap();
        assert_eq!(
            norm_path(&state.model_catalog_path),
            norm_path(&target.to_string_lossy())
        );
        assert!(state.model_catalog_exists);
        assert_eq!(state.model_catalog, r#"{"models":[]}"#);
    }

    #[test]
    fn read_state_uses_absolute_model_catalog_path_as_is() {
        let dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let target = outside.path().join("custom.json");
        fs::write(&target, "[]").unwrap();
        let abs = target.to_string_lossy().replace('\\', "/");
        fs::write(
            config_path_in(dir.path()),
            format!("model_catalog_json = \"{abs}\""),
        )
        .unwrap();

        let state = read_state_in(dir.path()).unwrap();
        assert_eq!(
            norm_path(&state.model_catalog_path),
            norm_path(&target.to_string_lossy())
        );
        assert!(state.model_catalog_exists);
        assert_eq!(state.model_catalog, "[]");
    }

    #[test]
    fn read_state_creates_missing_catalog_target_with_default_content() {
        let dir = TempDir::new().unwrap();
        fs::write(
            config_path_in(dir.path()),
            "model_catalog_json = \"catalog/custom.json\"",
        )
        .unwrap();

        let state = read_state_in(dir.path()).unwrap();
        let target = dir.path().join("catalog").join("custom.json");
        assert_eq!(
            norm_path(&state.model_catalog_path),
            norm_path(&target.to_string_lossy())
        );
        assert!(state.model_catalog_exists);
        assert_eq!(state.model_catalog, DEFAULT_MODEL_CATALOG_CONTENT);
        // 父目录与文件均已创建
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            DEFAULT_MODEL_CATALOG_CONTENT
        );
        assert!(!target.with_extension("json.tmp").exists());
    }

    #[test]
    fn read_state_treats_empty_catalog_value_as_unconfigured() {
        let dir = TempDir::new().unwrap();
        fs::write(config_path_in(dir.path()), "model_catalog_json = \"\"").unwrap();

        let state = read_state_in(dir.path()).unwrap();
        assert_eq!(state.model_catalog_path, "");
        assert!(!state.model_catalog_exists);
        assert_eq!(state.model_catalog, "");
        // 未创建任何目标文件
        let entries: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec!["config.toml"]);
    }

    #[test]
    fn save_config_writes_verbatim_without_managed_keys() {
        let dir = TempDir::new().unwrap();
        let content = "instructions = \"项目指引\"\n\n[mcp_servers.filesystem]\ncommand = \"npx\"\n";
        save_config_in(dir.path(), content).unwrap();

        let text = fs::read_to_string(config_path_in(dir.path())).unwrap();
        assert_eq!(text, content);
        assert!(!text.contains("model_provider"));
        assert!(!text.contains("model_catalog_json"));
        assert!(!config_path_in(dir.path())
            .with_extension("toml.tmp")
            .exists());
    }

    #[test]
    fn save_config_rejects_invalid_toml() {
        let dir = TempDir::new().unwrap();
        assert!(save_config_in(dir.path(), "not valid toml = = [").is_err());
        assert!(!config_path_in(dir.path()).exists());
    }

    #[test]
    fn save_config_creates_file_and_parent_dir_when_missing() {
        let base = TempDir::new().unwrap();
        let home = base.path().join("nested").join(".codex");
        save_config_in(&home, "model = \"x\"\n").unwrap();
        assert_eq!(
            fs::read_to_string(config_path_in(&home)).unwrap(),
            "model = \"x\"\n"
        );
    }

    #[test]
    fn save_config_allows_empty_content() {
        let dir = TempDir::new().unwrap();
        save_config_in(dir.path(), "").unwrap();
        assert_eq!(fs::read_to_string(config_path_in(dir.path())).unwrap(), "");
    }

    #[test]
    fn save_model_catalog_rejects_when_unconfigured() {
        let dir = TempDir::new().unwrap();
        assert!(save_model_catalog_in(dir.path(), "[]").is_err());
    }

    #[test]
    fn save_model_catalog_rejects_empty_and_invalid_content() {
        let dir = TempDir::new().unwrap();
        fs::write(
            config_path_in(dir.path()),
            "model_catalog_json = \"models.json\"",
        )
        .unwrap();

        assert!(save_model_catalog_in(dir.path(), "   ").is_err());
        assert!(save_model_catalog_in(dir.path(), "{ not json").is_err());
        assert!(!dir.path().join("models.json").exists());
    }

    #[test]
    fn save_model_catalog_writes_valid_content_to_resolved_target() {
        let dir = TempDir::new().unwrap();
        fs::write(
            config_path_in(dir.path()),
            "model_catalog_json = \"catalog/custom.json\"",
        )
        .unwrap();
        let content = "{\n  \"models\": []\n}\n";
        save_model_catalog_in(dir.path(), content).unwrap();

        let target = dir.path().join("catalog").join("custom.json");
        assert_eq!(fs::read_to_string(&target).unwrap(), content);
        assert!(!target.with_extension("json.tmp").exists());
    }
}
