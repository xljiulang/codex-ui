//! 模型配置读写：管理 `CODEX_HOME/config.toml` 与 `model_catalog_json` 目标文件。
//!
//! CODEX_HOME 优先取 codex-ui 进程自身的 `CODEX_HOME` 环境变量，未设置时降级为
//! `%USERPROFILE%\.codex`；codex 子进程不再被显式注入 CODEX_HOME，因此继承同一值，
//! 两者保持一致。config.toml 与 model_catalog_json 目标文件均按用户编辑内容整文件
//! 原样读写：config 保存前校验 TOML 可解析，不注入任何受管键；model_catalog_json
//! 目标路径统一由 resolve_catalog_path 解析——config 已配置则按实际值（绝对/相对/~），
//! 未配置则默认 `CODEX_HOME/models.json`（绝对路径）。读取时不自动创建目录文件；
//! 目录内容为空即视为未配置目录（由前端在保存时删除 config 中的 model_catalog_json 键）。

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item};

use crate::codex::path_util::clean_path;

/// `model_config_read` 的返回结构：config 整文件内容 + model_catalog_json 目标状态。
#[derive(Debug, Clone, Serialize)]
pub struct ModelConfigState {
    pub config_path: String,
    pub config_exists: bool,
    pub config_content: String,
    /// 顶层 model_catalog_json 的原始配置值（未配置时为空字符串）。
    pub model_catalog_json: String,
    /// model_catalog_json 目标文件路径；config 未配置该键时为空字符串。
    pub model_catalog_path: String,
    pub model_catalog_exists: bool,
    pub model_catalog: String,
    /// 顶层 model（模型名称）；未配置时为空字符串。
    pub model: String,
    /// 顶层 model_reasoning_effort；未配置时为空字符串。
    pub model_reasoning_effort: String,
    /// 顶层 model_provider（当前激活的提供方标识）；未配置时为空字符串。
    pub model_provider: String,
    /// 顶层 preferred_auth_method；未配置时为空字符串。
    pub preferred_auth_method: String,
    /// 顶层 forced_login_method；未配置时为空字符串。
    pub forced_login_method: String,
    /// 进程环境是否已设置非空 OPENAI_API_KEY（供 UI 认证三选一提示）。
    pub openai_api_key_present: bool,
    /// 全部 [model_providers.*]（按文档顺序）。
    pub providers: Vec<ModelProviderInfo>,
}

/// 单个 model_provider 的可视化字段（标识 key 创建后不可改名）。
#[derive(Debug, Clone, Serialize)]
pub struct ModelProviderInfo {
    /// [model_providers.<key>] 表名标识。
    pub key: String,
    pub name: String,
    pub base_url: String,
    pub env_key: String,
    pub experimental_bearer_token: String,
    pub wire_api: String,
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

pub(crate) fn config_path_in(home: &Path) -> PathBuf {
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

/// model_catalog_json 的目标路径：config 已配置则按既有规则解析（绝对值原样、相对按
/// CODEX_HOME 拼接、`~` 展开），未配置或值为空白则返回 `CODEX_HOME/models.json`（绝对路径）。
fn resolve_catalog_path(home: &Path, config_value: Option<&str>) -> PathBuf {
    match config_value {
        Some(v) if !v.trim().is_empty() => model_catalog_path_in(home, v.trim()),
        _ => home.join("models.json"),
    }
}

/// model_catalog_json 目标路径：绝对路径原样使用，相对路径基于 CODEX_HOME 拼接；
/// 独立首段 `~`（`~`、`~/...`、`~\...`）先展开为 %USERPROFILE%，再走上述判定。
fn model_catalog_path_in(home: &Path, value: &str) -> PathBuf {
    let expanded = expand_tilde(value);
    let p = Path::new(&expanded);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        home.join(p)
    }
}

/// 展开前导 `~`（仅当 `~` 为独立首段）：`~`、`~/...`、`~\...` → %USERPROFILE% 下路径；
/// `~user/...` 保持字面（Windows 无用户名展开语义）；USERPROFILE 缺失或空白时原样返回。
fn expand_tilde(value: &str) -> String {
    let Some(rest) = value.strip_prefix('~') else {
        return value.to_string();
    };
    if !(rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\')) {
        return value.to_string();
    }
    let Some(profile) = std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()) else {
        return value.to_string();
    };
    let profile = PathBuf::from(profile);
    if rest.is_empty() {
        return profile.to_string_lossy().into_owned();
    }
    // 去掉前导分隔符：Windows 下 join 以 `/`/`\` 开头的路径会重置到盘符根目录
    let rest = rest.trim_start_matches(|c| c == '/' || c == '\\');
    profile.join(rest).to_string_lossy().into_owned()
}

/// 结构化读取的顶层键与全部 [model_providers.*]。
#[derive(Debug, Default)]
struct StructuredConfig {
    model: String,
    model_reasoning_effort: String,
    model_provider: String,
    preferred_auth_method: String,
    forced_login_method: String,
    model_catalog_json: String,
    providers: Vec<ModelProviderInfo>,
}

/// 结构化读取顶层键与全部 [model_providers.*]（best-effort：
/// 文件缺失或无法解析时返回空值，原始文本仍由调用方返回）。
fn read_structured(config_path: &Path) -> StructuredConfig {
    let Ok(text) = fs::read_to_string(config_path) else {
        return StructuredConfig::default();
    };
    let Ok(doc) = text.parse::<DocumentMut>() else {
        return StructuredConfig::default();
    };
    let top = |key: &str| {
        doc.get(key)
            .and_then(Item::as_str)
            .unwrap_or("")
            .to_string()
    };
    let mut providers = Vec::new();
    if let Some(table) = doc.get("model_providers").and_then(Item::as_table) {
        for (key, item) in table.iter() {
            let Some(p) = item.as_table() else {
                continue;
            };
            providers.push(ModelProviderInfo {
                key: key.to_string(),
                name: p
                    .get("name")
                    .and_then(Item::as_str)
                    .unwrap_or("")
                    .to_string(),
                base_url: p
                    .get("base_url")
                    .and_then(Item::as_str)
                    .unwrap_or("")
                    .to_string(),
                env_key: p
                    .get("env_key")
                    .and_then(Item::as_str)
                    .unwrap_or("")
                    .to_string(),
                experimental_bearer_token: p
                    .get("experimental_bearer_token")
                    .and_then(Item::as_str)
                    .unwrap_or("")
                    .to_string(),
                wire_api: p
                    .get("wire_api")
                    .and_then(Item::as_str)
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }
    StructuredConfig {
        model: top("model"),
        model_reasoning_effort: top("model_reasoning_effort"),
        model_provider: top("model_provider"),
        preferred_auth_method: top("preferred_auth_method"),
        forced_login_method: top("forced_login_method"),
        model_catalog_json: top("model_catalog_json"),
        providers,
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
    let structured = read_structured(&config_path);
    let catalog_value = model_catalog_json_value(&config_path);
    let catalog_target = resolve_catalog_path(home, catalog_value.as_deref());
    // 仅在 config 显式配置了 model_catalog_json 键时才读取目录文件内容；
    // 未配置（含清空后删除的键）即使默认 models.json 残留，也不回显其内容。
    let (model_catalog_exists, model_catalog) = match catalog_value.as_deref() {
        Some(_) if catalog_target.is_file() => (
            true,
            fs::read_to_string(&catalog_target)
                .map_err(|e| format!("读取模型目录文件失败: {e}"))?,
        ),
        _ => (false, String::new()),
    };
    Ok(ModelConfigState {
        config_path: clean_path(&config_path),
        config_exists: true,
        config_content,
        model_catalog_json: structured.model_catalog_json,
        model_catalog_path: clean_path(&catalog_target),
        model_catalog_exists,
        model_catalog,
        model: structured.model,
        model_reasoning_effort: structured.model_reasoning_effort,
        model_provider: structured.model_provider,
        preferred_auth_method: structured.preferred_auth_method,
        forced_login_method: structured.forced_login_method,
        openai_api_key_present: std::env::var_os("OPENAI_API_KEY")
            .is_some_and(|v| !v.is_empty()),
        providers: structured.providers,
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

/// 保存 model_catalog_json 目标文件：目标路径实时解析（config 未配置时默认
/// CODEX_HOME/models.json，不再报「未配置」）；内容去空白后必须非空，且经
/// validate_model_catalog 校验（合理 JSON + codex 结构），通过后按原文原样写入。
pub fn save_model_catalog(content: &str) -> Result<(), String> {
    save_model_catalog_in(&codex_home()?, content)
}

fn save_model_catalog_in(home: &Path, content: &str) -> Result<(), String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("模型目录内容不能为空".to_string());
    }
    validate_model_catalog(trimmed)?;
    let target =
        resolve_catalog_path(home, model_catalog_json_value(&config_path_in(home)).as_deref());
    atomic_write(&target, content)
}

/// 校验模型目录 JSON：必须为合法 JSON、顶层为对象、含 `models` 数组，且数组每一项为对象。
/// `models` 可为空数组（`[]` 属合法空目录）。返回中文错误。
fn validate_model_catalog(trimmed: &str) -> Result<(), String> {
    let value: serde_json::Value = serde_json::from_str(trimmed)
        .map_err(|e| format!("模型目录不是合法 JSON: {e}"))?;
    let obj = value
        .as_object()
        .ok_or_else(|| "模型目录必须是顶层 JSON 对象（含 models 数组）".to_string())?;
    let models = obj
        .get("models")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "模型目录缺少 models 数组（应为 {\"models\":[...]}）".to_string())?;
    for (i, m) in models.iter().enumerate() {
        if !m.is_object() {
            return Err(format!("模型目录中第 {} 个模型不是对象", i + 1));
        }
    }
    Ok(())
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
        // 未配置时默认解析为 CODEX_HOME/models.json，但不自动创建文件
        assert_eq!(
            norm_path(&state.model_catalog_path),
            norm_path(&dir.path().join("models.json").to_string_lossy())
        );
        assert!(!state.model_catalog_exists);
        assert_eq!(state.model_catalog, "");
        assert!(!dir.path().join("models.json").exists());
    }

    #[test]
    fn read_state_reads_config_verbatim_and_ignores_unparseable() {
        let dir = TempDir::new().unwrap();
        let path = config_path_in(dir.path());
        fs::write(&path, "not valid toml = = [").unwrap();

        let state = read_state_in(dir.path()).unwrap();
        assert!(state.config_exists);
        assert_eq!(state.config_content, "not valid toml = = [");
        // 解析失败 → 仍回退默认 CODEX_HOME/models.json（不创建文件）
        assert_eq!(
            norm_path(&state.model_catalog_path),
            norm_path(&dir.path().join("models.json").to_string_lossy())
        );
        assert!(!state.model_catalog_exists);
        assert!(!dir.path().join("models.json").exists());
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
    fn read_state_does_not_create_missing_catalog_target() {
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
        assert!(!state.model_catalog_exists);
        assert_eq!(state.model_catalog, "");
        // 读取不自动创建目录文件
        assert!(!target.exists());
        assert!(!target.with_extension("json.tmp").exists());
    }

    #[test]
    fn read_state_does_not_read_default_models_json_when_unconfigured() {
        let dir = TempDir::new().unwrap();
        // 仅创建默认 models.json（含旧内容），config 不配置 model_catalog_json 键
        fs::write(dir.path().join("models.json"), r#"{"models":[{"slug":"old"}]}"#)
            .unwrap();

        let state = read_state_in(dir.path()).unwrap();
        // 未配置键 → 不读取默认文件内容、不视为已配置
        assert_eq!(
            norm_path(&state.model_catalog_path),
            norm_path(&dir.path().join("models.json").to_string_lossy())
        );
        assert!(!state.model_catalog_exists);
        assert_eq!(state.model_catalog, "");
    }

    #[test]
    fn read_state_treats_empty_catalog_value_as_unconfigured() {
        let dir = TempDir::new().unwrap();
        fs::write(config_path_in(dir.path()), "model_catalog_json = \"\"").unwrap();

        let state = read_state_in(dir.path()).unwrap();
        // 空值 → 解析回退默认 CODEX_HOME/models.json（不视为已配置）
        assert_eq!(
            norm_path(&state.model_catalog_path),
            norm_path(&dir.path().join("models.json").to_string_lossy())
        );
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
    fn save_model_catalog_defaults_to_models_json_when_unconfigured() {
        let dir = TempDir::new().unwrap();
        let content = "{\"models\":[]}";
        save_model_catalog_in(dir.path(), content).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("models.json")).unwrap(),
            content
        );
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
        // 结构校验：顶层非对象 / 缺 models 数组 / models 非数组 / 数组项非对象
        assert!(save_model_catalog_in(dir.path(), "[]").is_err());
        assert!(save_model_catalog_in(dir.path(), "{}").is_err());
        assert!(save_model_catalog_in(dir.path(), "{\"models\":{}}").is_err());
        assert!(save_model_catalog_in(dir.path(), "{\"models\":[1]}").is_err());
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

    #[test]
    fn read_state_parses_providers_and_top_level_keys() {
        let dir = TempDir::new().unwrap();
        fs::write(
            config_path_in(dir.path()),
            r#"model = "deepseek-v4-flash"
model_reasoning_effort = "high"
model_provider = "a"
preferred_auth_method = "apikey"
forced_login_method = "api"

[model_providers.a]
name = "A服务"
base_url = "https://a.example.com/v1"
env_key = "OPENAI_API_KEY"
experimental_bearer_token = "sk-a"
wire_api = "responses"

[model_providers.b]
name = "B服务"
base_url = "https://b.example.com/v1"
"#,
        )
        .unwrap();

        let state = read_state_in(dir.path()).unwrap();
        assert_eq!(state.model, "deepseek-v4-flash");
        assert_eq!(state.model_reasoning_effort, "high");
        assert_eq!(state.model_provider, "a");
        assert_eq!(state.preferred_auth_method, "apikey");
        assert_eq!(state.forced_login_method, "api");
        assert_eq!(state.providers.len(), 2);
        assert_eq!(state.providers[0].key, "a");
        assert_eq!(state.providers[0].name, "A服务");
        assert_eq!(state.providers[0].base_url, "https://a.example.com/v1");
        assert_eq!(state.providers[0].env_key, "OPENAI_API_KEY");
        assert_eq!(state.providers[0].experimental_bearer_token, "sk-a");
        assert_eq!(state.providers[0].wire_api, "responses");
        assert_eq!(state.providers[1].key, "b");
        assert_eq!(state.providers[1].env_key, "");
        assert_eq!(state.providers[1].wire_api, "");
    }

    #[test]
    fn read_state_unparseable_config_returns_empty_structured_fields() {
        let dir = TempDir::new().unwrap();
        fs::write(config_path_in(dir.path()), "not valid toml = = [").unwrap();

        let state = read_state_in(dir.path()).unwrap();
        assert_eq!(state.config_content, "not valid toml = = [");
        assert_eq!(state.model, "");
        assert_eq!(state.model_reasoning_effort, "");
        assert_eq!(state.model_provider, "");
        assert_eq!(state.preferred_auth_method, "");
        assert_eq!(state.forced_login_method, "");
        assert_eq!(state.model_catalog_json, "");
        assert!(state.providers.is_empty());
    }

    #[test]
    fn read_state_reads_optional_top_level_keys_raw() {
        let dir = TempDir::new().unwrap();
        fs::write(
            config_path_in(dir.path()),
            r#"model_catalog_json = "catalog/models.json"
preferred_auth_method = "apikey"
forced_login_method = "api"
"#,
        )
        .unwrap();

        let state = read_state_in(dir.path()).unwrap();
        assert_eq!(state.model_catalog_json, "catalog/models.json");
        assert_eq!(state.preferred_auth_method, "apikey");
        assert_eq!(state.forced_login_method, "api");
        // 读取不再自动创建目录文件
        assert!(!state.model_catalog_exists);
        assert_eq!(state.model_catalog, "");
    }

    #[test]
    fn expand_tilde_resolves_only_standalone_tilde() {
        let tmp = TempDir::new().unwrap();
        let profile = tmp.path().join("profile");
        let profile_s = profile.to_string_lossy().into_owned();
        with_envs(
            &[("CODEX_HOME", None), ("USERPROFILE", Some(&profile_s))],
            || {
                assert_eq!(expand_tilde("~"), profile_s);
                assert_eq!(
                    norm_path(&expand_tilde("~/a/b.json")),
                    norm_path(&profile.join("a").join("b.json").to_string_lossy())
                );
                assert_eq!(
                    norm_path(&expand_tilde("~\\a\\b.json")),
                    norm_path(&profile.join("a").join("b.json").to_string_lossy())
                );
                // `~user/...` 与普通相对路径不展开
                assert_eq!(expand_tilde("~user/a.json"), "~user/a.json");
                assert_eq!(expand_tilde("plain/a.json"), "plain/a.json");
            },
        );
    }

    #[test]
    fn expand_tilde_keeps_literal_without_userprofile() {
        with_envs(&[("CODEX_HOME", None), ("USERPROFILE", None)], || {
            assert_eq!(expand_tilde("~/a.json"), "~/a.json");
            assert_eq!(expand_tilde("~"), "~");
        });
    }

    #[test]
    fn read_state_resolves_tilde_catalog_against_userprofile() {
        let dir = TempDir::new().unwrap();
        let profile = TempDir::new().unwrap();
        let profile_s = profile.path().to_string_lossy().into_owned();
        fs::write(
            config_path_in(dir.path()),
            "model_catalog_json = \"~/.codex/models.json\"",
        )
        .unwrap();
        let target = profile.path().join(".codex").join("models.json");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, r#"{"models":[]}"#).unwrap();

        with_envs(
            &[("CODEX_HOME", None), ("USERPROFILE", Some(&profile_s))],
            || {
                let state = read_state_in(dir.path()).unwrap();
                assert_eq!(
                    norm_path(&state.model_catalog_path),
                    norm_path(&target.to_string_lossy())
                );
                assert!(state.model_catalog_exists);
                assert_eq!(state.model_catalog, r#"{"models":[]}"#);
            },
        );
    }

    #[test]
    fn read_state_resolves_tilde_with_backslash_separator() {
        let dir = TempDir::new().unwrap();
        let profile = TempDir::new().unwrap();
        let profile_s = profile.path().to_string_lossy().into_owned();
        fs::write(
            config_path_in(dir.path()),
            r#"model_catalog_json = "~\\models.json""#,
        )
        .unwrap();
        let target = profile.path().join("models.json");
        fs::write(&target, "[]").unwrap();

        with_envs(
            &[("CODEX_HOME", None), ("USERPROFILE", Some(&profile_s))],
            || {
                let state = read_state_in(dir.path()).unwrap();
                assert_eq!(
                    norm_path(&state.model_catalog_path),
                    norm_path(&target.to_string_lossy())
                );
                assert!(state.model_catalog_exists);
                assert_eq!(state.model_catalog, "[]");
            },
        );
    }

    #[test]
    fn read_state_does_not_create_missing_tilde_catalog_target() {
        let dir = TempDir::new().unwrap();
        let profile = TempDir::new().unwrap();
        let profile_s = profile.path().to_string_lossy().into_owned();
        fs::write(
            config_path_in(dir.path()),
            "model_catalog_json = \"~/catalog/custom.json\"",
        )
        .unwrap();

        with_envs(
            &[("CODEX_HOME", None), ("USERPROFILE", Some(&profile_s))],
            || {
                let state = read_state_in(dir.path()).unwrap();
                let target = profile.path().join("catalog").join("custom.json");
                assert_eq!(
                    norm_path(&state.model_catalog_path),
                    norm_path(&target.to_string_lossy())
                );
                assert!(!state.model_catalog_exists);
                assert_eq!(state.model_catalog, "");
                assert!(!target.exists());
            },
        );
    }
}
