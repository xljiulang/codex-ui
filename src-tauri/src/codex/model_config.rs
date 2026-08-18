//! 模型配置读写：管理 `CODEX_HOME/config.toml` 与 `model_catalog_json` 目标文件。
//!
//! CODEX_HOME 优先取 codex-ui 进程自身的 `CODEX_HOME` 环境变量，未设置时降级为
//! `%USERPROFILE%\.codex`；codex 子进程不再被显式注入 CODEX_HOME，因此继承同一值，
//! 两者保持一致。config.toml 与 model_catalog_json 目标文件均按用户编辑内容整文件
//! 原样读写：config 保存前校验 TOML 可解析，不注入任何受管键；model_catalog_json
//! 目标路径完全以 config.toml 中实际配置的值为准，不提供默认文件名。
//! 读取时若文件缺失会自动创建：config.toml 创建空文件，model_catalog_json 目标
//! 创建 `{"models":[]}`（该键未配置时不创建）。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use toml_edit::{value, DocumentMut, Item, Table};

/// 自动创建 model_catalog_json 目标文件时的默认内容（空模型目录，与 codex 目录格式一致）。
pub const DEFAULT_MODEL_CATALOG_CONTENT: &str = "{\"models\":[]}";

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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProviderInfo {
    /// [model_providers.<key>] 表名标识。
    pub key: String,
    pub name: String,
    pub base_url: String,
    pub env_key: String,
    pub experimental_bearer_token: String,
    pub wire_api: String,
}

/// `model_config_ui_save` 的输入：可视化覆盖的顶层键与全部提供方。
#[derive(Debug, Clone, Deserialize)]
pub struct ModelConfigUiEdit {
    pub model: String,
    pub model_reasoning_effort: String,
    /// 当前激活的提供方标识；空串表示不选择。
    pub model_provider: String,
    /// 顶层 preferred_auth_method；空串表示不写入。
    pub preferred_auth_method: String,
    /// 顶层 forced_login_method；空串表示不写入。
    pub forced_login_method: String,
    /// 顶层 model_catalog_json 原始值；空串表示不写入。
    pub model_catalog_json: String,
    pub providers: Vec<ModelProviderInfo>,
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

/// 检查 model_catalog_json 配置值对应的目标文件是否存在（不创建文件）。
/// 空值视为未配置，返回 false。
pub fn catalog_target_exists(value: &str) -> Result<bool, String> {
    let home = codex_home()?;
    let value = value.trim();
    if value.is_empty() {
        return Ok(false);
    }
    Ok(model_catalog_path_in(&home, value).is_file())
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
        model_catalog_json: structured.model_catalog_json,
        model_catalog_path: catalog_target
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
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

/// 保存可视化模型配置（真实 CODEX_HOME）：只写可视化覆盖的顶层键与提供方表，
/// 其余 TOML 内容（注释、其它键、提供方内未知字段）原样保留。
pub fn save_config_ui(edit: &ModelConfigUiEdit) -> Result<(), String> {
    save_config_ui_in(&codex_home()?, edit)
}

fn save_config_ui_in(home: &Path, edit: &ModelConfigUiEdit) -> Result<(), String> {
    let path = config_path_in(home);
    let original = if path.is_file() {
        fs::read_to_string(&path).map_err(|e| format!("读取 config.toml 失败: {e}"))?
    } else {
        String::new()
    };
    let mut doc: DocumentMut = original.parse().map_err(|e| {
        format!("config.toml 解析失败，无法用可视化编辑保存（可先用原始 config 修复）: {e}")
    })?;

    // 校验：提供方标识合法且不重复；激活项必须存在于列表。
    let mut seen: Vec<String> = Vec::new();
    for p in &edit.providers {
        let key = p.key.trim();
        if key.is_empty() {
            return Err("提供方标识不能为空".to_string());
        }
        if !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(format!(
                "提供方标识「{key}」只能包含字母、数字、下划线与连字符"
            ));
        }
        if seen.contains(&key.to_string()) {
            return Err(format!("提供方标识「{key}」重复"));
        }
        seen.push(key.to_string());
    }
    let active = edit.model_provider.trim();
    if !active.is_empty() && !seen.iter().any(|k| k == active) {
        return Err(format!("激活的提供方「{active}」不存在"));
    }

    // 顶层键：空串清除。
    set_or_remove(&mut doc, "model", edit.model.trim());
    set_or_remove(
        &mut doc,
        "model_reasoning_effort",
        edit.model_reasoning_effort.trim(),
    );
    set_or_remove(&mut doc, "model_provider", active);
    set_or_remove(
        &mut doc,
        "preferred_auth_method",
        edit.preferred_auth_method.trim(),
    );
    set_or_remove(
        &mut doc,
        "forced_login_method",
        edit.forced_login_method.trim(),
    );
    set_or_remove(
        &mut doc,
        "model_catalog_json",
        edit.model_catalog_json.trim(),
    );

    // 提供方同步：upsert 已知字段（保留未知字段），删除列表外的提供方表。
    let providers_table = doc
        .entry("model_providers")
        .or_insert(Item::Table(Table::new()));
    let providers_table = providers_table
        .as_table_mut()
        .ok_or_else(|| "config.toml 中 model_providers 必须是表".to_string())?;
    for p in &edit.providers {
        let key = p.key.trim();
        let entry = providers_table
            .entry(key)
            .or_insert(Item::Table(Table::new()));
        let table = entry
            .as_table_mut()
            .ok_or_else(|| format!("model_providers.{key} 必须是表"))?;
        set_or_remove(table, "name", p.name.trim());
        set_or_remove(table, "base_url", p.base_url.trim());
        set_or_remove(table, "env_key", p.env_key.trim());
        set_or_remove(
            table,
            "experimental_bearer_token",
            p.experimental_bearer_token.trim(),
        );
        set_or_remove(table, "wire_api", p.wire_api.trim());
    }
    let existing: Vec<String> = providers_table
        .iter()
        .map(|(k, _)| k.to_string())
        .collect();
    for k in existing {
        if !seen.contains(&k) {
            providers_table.remove(&k);
        }
    }

    atomic_write(&path, &doc.to_string())
}

/// 写入字符串值；空串时移除该键。
fn set_or_remove(table: &mut Table, key: &str, val: &str) {
    if val.is_empty() {
        table.remove(key);
    } else {
        table.insert(key, value(val));
    }
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
    fn save_config_ui_upserts_providers_preserving_unknown_fields() {
        let dir = TempDir::new().unwrap();
        fs::write(
            config_path_in(dir.path()),
            r#"[model_providers.a]
name = "旧名"
env_key = "OPENAI_API_KEY"
request_max_retries = 5
"#,
        )
        .unwrap();

        let edit = ModelConfigUiEdit {
            model: "gpt-x".into(),
            model_reasoning_effort: "max".into(),
            model_provider: "a".into(),
            preferred_auth_method: "apikey".into(),
            forced_login_method: "api".into(),
            model_catalog_json: "catalog/models.json".into(),
            providers: vec![ModelProviderInfo {
                key: "a".into(),
                name: "新名".into(),
                base_url: "https://x.example.com/v1".into(),
                env_key: "OPENAI_API_KEY".into(),
                experimental_bearer_token: "sk-x".into(),
                wire_api: "responses".into(),
            }],
        };
        save_config_ui_in(dir.path(), &edit).unwrap();

        let text = fs::read_to_string(config_path_in(dir.path())).unwrap();
        let doc: DocumentMut = text.parse().unwrap();
        assert_eq!(doc.get("model").unwrap().as_str(), Some("gpt-x"));
        assert_eq!(
            doc.get("model_reasoning_effort").unwrap().as_str(),
            Some("max")
        );
        assert_eq!(doc.get("model_provider").unwrap().as_str(), Some("a"));
        assert_eq!(
            doc.get("preferred_auth_method").unwrap().as_str(),
            Some("apikey")
        );
        assert_eq!(
            doc.get("forced_login_method").unwrap().as_str(),
            Some("api")
        );
        assert_eq!(
            doc.get("model_catalog_json").unwrap().as_str(),
            Some("catalog/models.json")
        );
        let p = doc
            .get("model_providers")
            .unwrap()
            .as_table()
            .unwrap()
            .get("a")
            .unwrap()
            .as_table()
            .unwrap();
        assert_eq!(p.get("name").unwrap().as_str(), Some("新名"));
        assert_eq!(
            p.get("base_url").unwrap().as_str(),
            Some("https://x.example.com/v1")
        );
        assert_eq!(
            p.get("env_key").unwrap().as_str(),
            Some("OPENAI_API_KEY")
        );
        assert_eq!(
            p.get("experimental_bearer_token").unwrap().as_str(),
            Some("sk-x")
        );
        assert_eq!(p.get("wire_api").unwrap().as_str(), Some("responses"));
        // env_key 已成为受管字段（编辑值写入）；真正的未知字段保留
        assert_eq!(p.get("env_key").unwrap().as_str(), Some("OPENAI_API_KEY"));
        assert_eq!(p.get("request_max_retries").unwrap().as_integer(), Some(5));
    }

    #[test]
    fn save_config_ui_removes_deleted_providers() {
        let dir = TempDir::new().unwrap();
        fs::write(
            config_path_in(dir.path()),
            r#"[model_providers.a]
name = "A"
[model_providers.b]
name = "B"
"#,
        )
        .unwrap();

        let edit = ModelConfigUiEdit {
            model: String::new(),
            model_reasoning_effort: String::new(),
            model_provider: "a".into(),
            preferred_auth_method: String::new(),
            forced_login_method: String::new(),
            model_catalog_json: String::new(),
            providers: vec![ModelProviderInfo {
                key: "a".into(),
                name: "A".into(),
                base_url: String::new(),
                env_key: String::new(),
                experimental_bearer_token: String::new(),
                wire_api: String::new(),
            }],
        };
        save_config_ui_in(dir.path(), &edit).unwrap();

        let text = fs::read_to_string(config_path_in(dir.path())).unwrap();
        let doc: DocumentMut = text.parse().unwrap();
        let providers = doc.get("model_providers").unwrap().as_table().unwrap();
        assert!(providers.get("a").is_some());
        assert!(providers.get("b").is_none());
    }

    #[test]
    fn save_config_ui_clears_top_level_keys_when_empty() {
        let dir = TempDir::new().unwrap();
        fs::write(
            config_path_in(dir.path()),
            r#"model = "old"
model_provider = "a"
model_reasoning_effort = "high"

[model_providers.a]
name = "A"
"#,
        )
        .unwrap();

        let edit = ModelConfigUiEdit {
            model: String::new(),
            model_reasoning_effort: String::new(),
            model_provider: String::new(),
            preferred_auth_method: String::new(),
            forced_login_method: String::new(),
            model_catalog_json: String::new(),
            providers: vec![ModelProviderInfo {
                key: "a".into(),
                name: "A".into(),
                base_url: String::new(),
                env_key: String::new(),
                experimental_bearer_token: String::new(),
                wire_api: String::new(),
            }],
        };
        save_config_ui_in(dir.path(), &edit).unwrap();

        let text = fs::read_to_string(config_path_in(dir.path())).unwrap();
        let doc: DocumentMut = text.parse().unwrap();
        assert!(doc.get("model").is_none());
        assert!(doc.get("model_reasoning_effort").is_none());
        assert!(doc.get("model_provider").is_none());
    }

    #[test]
    fn save_config_ui_rejects_invalid_key_and_missing_active() {
        let dir = TempDir::new().unwrap();
        fs::write(config_path_in(dir.path()), "").unwrap();

        let bad_key = ModelConfigUiEdit {
            model: String::new(),
            model_reasoning_effort: String::new(),
            model_provider: String::new(),
            preferred_auth_method: String::new(),
            forced_login_method: String::new(),
            model_catalog_json: String::new(),
            providers: vec![ModelProviderInfo {
                key: "a b".into(),
                name: String::new(),
                base_url: String::new(),
                env_key: String::new(),
                experimental_bearer_token: String::new(),
                wire_api: String::new(),
            }],
        };
        assert!(save_config_ui_in(dir.path(), &bad_key).is_err());

        let duplicate = ModelConfigUiEdit {
            model: String::new(),
            model_reasoning_effort: String::new(),
            model_provider: String::new(),
            preferred_auth_method: String::new(),
            forced_login_method: String::new(),
            model_catalog_json: String::new(),
            providers: vec![
                ModelProviderInfo {
                    key: "a".into(),
                    name: String::new(),
                    base_url: String::new(),
                    env_key: String::new(),
                    experimental_bearer_token: String::new(),
                    wire_api: String::new(),
                },
                ModelProviderInfo {
                    key: "a".into(),
                    name: String::new(),
                    base_url: String::new(),
                    env_key: String::new(),
                    experimental_bearer_token: String::new(),
                    wire_api: String::new(),
                },
            ],
        };
        assert!(save_config_ui_in(dir.path(), &duplicate).is_err());

        let missing_active = ModelConfigUiEdit {
            model: String::new(),
            model_reasoning_effort: String::new(),
            model_provider: "nope".into(),
            preferred_auth_method: String::new(),
            forced_login_method: String::new(),
            model_catalog_json: String::new(),
            providers: vec![],
        };
        assert!(save_config_ui_in(dir.path(), &missing_active).is_err());

        // 空列表 + 不选择激活：允许保存
        let empty_ok = ModelConfigUiEdit {
            model: String::new(),
            model_reasoning_effort: String::new(),
            model_provider: String::new(),
            preferred_auth_method: String::new(),
            forced_login_method: String::new(),
            model_catalog_json: String::new(),
            providers: vec![],
        };
        assert!(save_config_ui_in(dir.path(), &empty_ok).is_ok());
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
        // 目标文件仍按既有策略自动创建
        assert!(state.model_catalog_exists);
    }

    #[test]
    fn save_config_ui_clears_optional_top_level_keys_when_empty() {
        let dir = TempDir::new().unwrap();
        fs::write(
            config_path_in(dir.path()),
            r#"model = "old"
model_provider = "a"
preferred_auth_method = "apikey"
forced_login_method = "api"
model_catalog_json = "models.json"

[model_providers.a]
name = "A"
"#,
        )
        .unwrap();

        let edit = ModelConfigUiEdit {
            model: String::new(),
            model_reasoning_effort: String::new(),
            model_provider: String::new(),
            preferred_auth_method: String::new(),
            forced_login_method: String::new(),
            model_catalog_json: String::new(),
            providers: vec![],
        };
        save_config_ui_in(dir.path(), &edit).unwrap();

        let text = fs::read_to_string(config_path_in(dir.path())).unwrap();
        let doc: DocumentMut = text.parse().unwrap();
        assert!(doc.get("preferred_auth_method").is_none());
        assert!(doc.get("forced_login_method").is_none());
        assert!(doc.get("model_catalog_json").is_none());
    }

    #[test]
    fn catalog_target_exists_checks_file_without_creating() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        let home_s = home.to_string_lossy().into_owned();
        with_envs(
            &[("CODEX_HOME", Some(&home_s)), ("USERPROFILE", None)],
            || {
                assert!(!catalog_target_exists("models.json").unwrap());
                assert!(!home.join("models.json").exists());
                fs::create_dir_all(&home).unwrap();
                fs::write(home.join("models.json"), "[]").unwrap();
                assert!(catalog_target_exists("models.json").unwrap());
                assert!(!catalog_target_exists("").unwrap());
            },
        );
    }
}
