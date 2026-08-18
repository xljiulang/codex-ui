//! 模型配置读写：管理 `CODEX_HOME/config.toml` 与 `CODEX_HOME/models.json`。
//!
//! CODEX_HOME 与 codex 子进程保持一致（`<可执行文件目录>/.codex`，见
//! `app_server::apply_codex_env`）。保存 config.toml 时只更新受管键，
//! 其余内容（键、段落、注释）原样保留。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use toml_edit::{value, DocumentMut, Item, Table};

use crate::codex::app_server::app_exe_dir;

/// 受管固定值：保存时由程序强制写入，前端不提交。
pub const FIXED_MODEL_PROVIDER: &str = "codex-ui";
pub const FIXED_FORCED_LOGIN_METHOD: &str = "api";
pub const FIXED_MODEL_CATALOG_JSON: &str = "models.json";
pub const FIXED_PREFERRED_AUTH_METHOD: &str = "apikey";
pub const FIXED_WIRE_API: &str = "responses";

/// 示例默认值：config.toml 不存在或某键缺失时回填，供用户直接编辑。
pub const DEFAULT_MODEL: &str = "deepseek-v4-flash";
pub const DEFAULT_MODEL_REASONING_EFFORT: &str = "high";
pub const DEFAULT_PROVIDER_NAME: &str = "deepseek";
pub const DEFAULT_BASE_URL: &str = "https://api.deepseek.com/";
pub const DEFAULT_BEARER_TOKEN: &str = "你的 DeepSeek API Key";

/// config.toml 中由用户编辑的字段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelConfigEdit {
    pub model: String,
    pub model_reasoning_effort: String,
    pub name: String,
    pub base_url: String,
    pub experimental_bearer_token: String,
}

impl Default for ModelConfigEdit {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.into(),
            model_reasoning_effort: DEFAULT_MODEL_REASONING_EFFORT.into(),
            name: DEFAULT_PROVIDER_NAME.into(),
            base_url: DEFAULT_BASE_URL.into(),
            experimental_bearer_token: DEFAULT_BEARER_TOKEN.into(),
        }
    }
}

/// `model_config_read` 的返回结构：含路径、存在性、5 个可编辑值与 5 个固定值。
#[derive(Debug, Clone, Serialize)]
pub struct ModelConfigState {
    pub config_path: String,
    pub config_exists: bool,
    pub model: String,
    pub model_reasoning_effort: String,
    pub model_provider: String,
    pub forced_login_method: String,
    pub model_catalog_json: String,
    pub preferred_auth_method: String,
    pub wire_api: String,
    pub name: String,
    pub base_url: String,
    pub experimental_bearer_token: String,
    pub models_json_path: String,
    pub models_json_exists: bool,
    pub models_json: String,
}

/// 应用实际使用的 CODEX_HOME：`<可执行文件目录>/.codex`（与 codex 子进程一致）。
pub fn codex_home() -> Result<PathBuf, String> {
    let dir = app_exe_dir().ok_or_else(|| "无法定位应用目录".to_string())?;
    Ok(dir.join(".codex"))
}

fn config_path_in(home: &Path) -> PathBuf {
    home.join("config.toml")
}

fn models_json_path_in(home: &Path) -> PathBuf {
    home.join("models.json")
}

/// 读取模型配置状态（真实 CODEX_HOME）。
pub fn read_state() -> Result<ModelConfigState, String> {
    read_state_in(&codex_home()?)
}

fn read_state_in(home: &Path) -> Result<ModelConfigState, String> {
    let config_path = config_path_in(home);
    let models_json_path = models_json_path_in(home);
    let (config_exists, edit) = read_edit(&config_path)?;
    let (models_json_exists, models_json) = if models_json_path.is_file() {
        (
            true,
            fs::read_to_string(&models_json_path)
                .map_err(|e| format!("读取 models.json 失败: {e}"))?,
        )
    } else {
        (false, String::new())
    };
    Ok(ModelConfigState {
        config_path: config_path.to_string_lossy().into_owned(),
        config_exists,
        model: edit.model,
        model_reasoning_effort: edit.model_reasoning_effort,
        model_provider: FIXED_MODEL_PROVIDER.into(),
        forced_login_method: FIXED_FORCED_LOGIN_METHOD.into(),
        model_catalog_json: FIXED_MODEL_CATALOG_JSON.into(),
        preferred_auth_method: FIXED_PREFERRED_AUTH_METHOD.into(),
        wire_api: FIXED_WIRE_API.into(),
        name: edit.name,
        base_url: edit.base_url,
        experimental_bearer_token: edit.experimental_bearer_token,
        models_json_path: models_json_path.to_string_lossy().into_owned(),
        models_json_exists,
        models_json,
    })
}

/// 读取可编辑字段：文件缺失或键缺失时回退示例默认值；
/// 文件存在但 TOML 无法解析时返回错误（不静默丢弃原内容）。
fn read_edit(path: &Path) -> Result<(bool, ModelConfigEdit), String> {
    if !path.is_file() {
        return Ok((false, ModelConfigEdit::default()));
    }
    let text = fs::read_to_string(path).map_err(|e| format!("读取 config.toml 失败: {e}"))?;
    let doc: DocumentMut = text
        .parse()
        .map_err(|e| format!("config.toml 解析失败: {e}"))?;
    let provider = doc
        .get("model_providers")
        .and_then(Item::as_table)
        .and_then(|t| t.get("codex-ui"))
        .and_then(Item::as_table);
    let mut edit = ModelConfigEdit::default();
    if let Some(v) = doc.get("model").and_then(Item::as_str) {
        edit.model = v.to_string();
    }
    if let Some(v) = doc.get("model_reasoning_effort").and_then(Item::as_str) {
        edit.model_reasoning_effort = v.to_string();
    }
    if let Some(p) = provider {
        if let Some(v) = p.get("name").and_then(Item::as_str) {
            edit.name = v.to_string();
        }
        if let Some(v) = p.get("base_url").and_then(Item::as_str) {
            edit.base_url = v.to_string();
        }
        if let Some(v) = p.get("experimental_bearer_token").and_then(Item::as_str) {
            edit.experimental_bearer_token = v.to_string();
        }
    }
    Ok((true, edit))
}

/// 保存 config.toml（真实 CODEX_HOME）：只更新受管键，其余内容保留。
pub fn save_config(edit: &ModelConfigEdit) -> Result<(), String> {
    save_config_in(&codex_home()?, edit)
}

fn save_config_in(home: &Path, edit: &ModelConfigEdit) -> Result<(), String> {
    let path = config_path_in(home);
    let original = if path.is_file() {
        fs::read_to_string(&path).map_err(|e| format!("读取 config.toml 失败: {e}"))?
    } else {
        String::new()
    };
    // 原文件无法解析时按空文档重建受管键（视为修复路径，无法保留原内容）。
    let mut doc: DocumentMut = original.parse().unwrap_or_default();
    doc.insert("model", value(edit.model.trim()));
    doc.insert(
        "model_reasoning_effort",
        value(edit.model_reasoning_effort.trim()),
    );
    doc.insert("model_provider", value(FIXED_MODEL_PROVIDER));
    doc.insert("forced_login_method", value(FIXED_FORCED_LOGIN_METHOD));
    doc.insert("model_catalog_json", value(FIXED_MODEL_CATALOG_JSON));
    doc.insert("preferred_auth_method", value(FIXED_PREFERRED_AUTH_METHOD));

    let providers = doc
        .entry("model_providers")
        .or_insert(Item::Table(Table::new()));
    let provider = providers
        .as_table_mut()
        .ok_or_else(|| "config.toml 中 model_providers 必须是表".to_string())?
        .entry("codex-ui")
        .or_insert(Item::Table(Table::new()));
    let provider = provider
        .as_table_mut()
        .ok_or_else(|| "config.toml 中 model_providers.codex-ui 必须是表".to_string())?;
    provider.insert("name", value(edit.name.trim()));
    provider.insert("base_url", value(edit.base_url.trim()));
    provider.insert(
        "experimental_bearer_token",
        value(edit.experimental_bearer_token.trim()),
    );
    provider.insert("wire_api", value(FIXED_WIRE_API));

    atomic_write(&path, &doc.to_string())
}

/// 保存 models.json（真实 CODEX_HOME）：内容去空白后必须非空且为合法 JSON，
/// 合法后按原文原样写入。
pub fn save_models_json(content: &str) -> Result<(), String> {
    save_models_json_in(&codex_home()?, content)
}

fn save_models_json_in(home: &Path, content: &str) -> Result<(), String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("models.json 内容不能为空".to_string());
    }
    serde_json::from_str::<serde_json::Value>(trimmed)
        .map_err(|e| format!("models.json 不是合法 JSON: {e}"))?;
    atomic_write(&models_json_path_in(home), content)
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
    use tempfile::TempDir;

    #[test]
    fn missing_config_returns_defaults_and_exists_false() {
        let dir = TempDir::new().unwrap();
        let (exists, edit) = read_edit(&config_path_in(dir.path())).unwrap();
        assert!(!exists);
        assert_eq!(edit.model, DEFAULT_MODEL);
        assert_eq!(edit.model_reasoning_effort, DEFAULT_MODEL_REASONING_EFFORT);
        assert_eq!(edit.name, DEFAULT_PROVIDER_NAME);
        assert_eq!(edit.base_url, "https://api.deepseek.com/");
        assert_eq!(edit.experimental_bearer_token, "你的 DeepSeek API Key");
    }

    #[test]
    fn save_creates_config_with_fixed_and_editable_values() {
        let dir = TempDir::new().unwrap();
        let edit = ModelConfigEdit {
            model: "deepseek-v4-pro".into(),
            model_reasoning_effort: "max".into(),
            name: "deepseek".into(),
            base_url: "http://10.0.0.20:9080/v1".into(),
            experimental_bearer_token: "tc-chenguowei".into(),
        };
        save_config_in(dir.path(), &edit).unwrap();

        let text = fs::read_to_string(config_path_in(dir.path())).unwrap();
        let doc: DocumentMut = text.parse().unwrap();
        assert_eq!(doc.get("model").unwrap().as_str(), Some("deepseek-v4-pro"));
        assert_eq!(
            doc.get("model_reasoning_effort").unwrap().as_str(),
            Some("max")
        );
        assert_eq!(
            doc.get("model_provider").unwrap().as_str(),
            Some(FIXED_MODEL_PROVIDER)
        );
        assert_eq!(
            doc.get("forced_login_method").unwrap().as_str(),
            Some(FIXED_FORCED_LOGIN_METHOD)
        );
        assert_eq!(
            doc.get("model_catalog_json").unwrap().as_str(),
            Some(FIXED_MODEL_CATALOG_JSON)
        );
        assert_eq!(
            doc.get("preferred_auth_method").unwrap().as_str(),
            Some(FIXED_PREFERRED_AUTH_METHOD)
        );
        let provider = doc
            .get("model_providers")
            .unwrap()
            .as_table()
            .unwrap()
            .get("codex-ui")
            .unwrap()
            .as_table()
            .unwrap();
        assert_eq!(provider.get("name").unwrap().as_str(), Some("deepseek"));
        assert_eq!(
            provider.get("base_url").unwrap().as_str(),
            Some("http://10.0.0.20:9080/v1")
        );
        assert_eq!(
            provider.get("experimental_bearer_token").unwrap().as_str(),
            Some("tc-chenguowei")
        );
        assert_eq!(
            provider.get("wire_api").unwrap().as_str(),
            Some(FIXED_WIRE_API)
        );
        // 原子写不残留临时文件
        assert!(!config_path_in(dir.path())
            .with_extension("toml.tmp")
            .exists());
    }

    #[test]
    fn save_preserves_other_keys_sections_and_comments() {
        let dir = TempDir::new().unwrap();
        let path = config_path_in(dir.path());
        fs::write(
            &path,
            r#"# keep this comment
instructions = "项目指引"

[mcp_servers.filesystem]
command = "npx"

[model_providers.codex-ui]
request_max_retries = 5
"#,
        )
        .unwrap();

        save_config_in(dir.path(), &ModelConfigEdit::default()).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("# keep this comment"));
        assert!(text.contains("instructions = \"项目指引\""));
        assert!(text.contains("[mcp_servers.filesystem]"));
        assert!(text.contains("command = \"npx\""));
        assert!(text.contains("request_max_retries = 5"));
    }

    #[test]
    fn save_keeps_managed_root_keys_at_root_when_tables_exist() {
        let dir = TempDir::new().unwrap();
        let path = config_path_in(dir.path());
        fs::write(
            &path,
            r#"[mcp_servers.filesystem]
command = "npx"

[model_providers.codex-ui]
request_max_retries = 5
"#,
        )
        .unwrap();

        save_config_in(dir.path(), &ModelConfigEdit::default()).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        let doc: DocumentMut = text.parse().unwrap();
        // 顶层受管键必须仍在根表，不能掉进 mcp_servers 或 provider 表
        assert_eq!(doc.get("model").unwrap().as_str(), Some(DEFAULT_MODEL));
        assert_eq!(
            doc.get("model_provider").unwrap().as_str(),
            Some(FIXED_MODEL_PROVIDER)
        );
        assert!(
            doc.get("mcp_servers")
                .and_then(Item::as_table)
                .unwrap()
                .get("model")
                .is_none()
        );
        let provider = doc
            .get("model_providers")
            .and_then(Item::as_table)
            .unwrap()
            .get("codex-ui")
            .and_then(Item::as_table)
            .unwrap();
        assert!(provider.get("model").is_none());
        assert_eq!(provider.get("name").unwrap().as_str(), Some(DEFAULT_PROVIDER_NAME));
    }

    #[test]
    fn save_rebuilds_unparseable_config() {
        let dir = TempDir::new().unwrap();
        let path = config_path_in(dir.path());
        fs::write(&path, "not valid toml = = [").unwrap();

        save_config_in(dir.path(), &ModelConfigEdit::default()).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.parse::<DocumentMut>().is_ok());
        assert!(text.contains(FIXED_MODEL_PROVIDER));
    }

    #[test]
    fn read_roundtrip_returns_saved_values() {
        let dir = TempDir::new().unwrap();
        let edit = ModelConfigEdit {
            model: "gpt-5.2-codex".into(),
            model_reasoning_effort: "low".into(),
            name: "deepseek".into(),
            base_url: "https://api.deepseek.com/".into(),
            experimental_bearer_token: "sk-test".into(),
        };
        save_config_in(dir.path(), &edit).unwrap();

        let (exists, loaded) = read_edit(&config_path_in(dir.path())).unwrap();
        assert!(exists);
        assert_eq!(loaded.model, "gpt-5.2-codex");
        assert_eq!(loaded.model_reasoning_effort, "low");
        assert_eq!(loaded.name, "deepseek");
        assert_eq!(loaded.base_url, "https://api.deepseek.com/");
        assert_eq!(loaded.experimental_bearer_token, "sk-test");
    }

    #[test]
    fn read_state_reports_paths_and_models_json() {
        let dir = TempDir::new().unwrap();
        fs::write(
            models_json_path_in(dir.path()),
            r#"[{"id":"deepseek-v4-flash"}]"#,
        )
        .unwrap();

        let state = read_state_in(dir.path()).unwrap();
        assert!(!state.config_exists);
        assert_eq!(state.model, DEFAULT_MODEL);
        assert_eq!(state.model_provider, FIXED_MODEL_PROVIDER);
        assert_eq!(state.wire_api, FIXED_WIRE_API);
        assert!(state.models_json_exists);
        assert_eq!(state.models_json, r#"[{"id":"deepseek-v4-flash"}]"#);
        assert_eq!(state.config_path, config_path_in(dir.path()).to_string_lossy());
        assert_eq!(
            state.models_json_path,
            models_json_path_in(dir.path()).to_string_lossy()
        );
    }

    #[test]
    fn models_json_rejects_empty_and_invalid_content() {
        let dir = TempDir::new().unwrap();
        assert!(save_models_json_in(dir.path(), "   ").is_err());
        assert!(save_models_json_in(dir.path(), "{ not json").is_err());
        assert!(!models_json_path_in(dir.path()).exists());
    }

    #[test]
    fn models_json_writes_valid_content_verbatim() {
        let dir = TempDir::new().unwrap();
        let content = "{\n  \"models\": []\n}\n";
        save_models_json_in(dir.path(), content).unwrap();
        assert_eq!(
            fs::read_to_string(models_json_path_in(dir.path())).unwrap(),
            content
        );
        assert!(!models_json_path_in(dir.path())
            .with_extension("json.tmp")
            .exists());
    }
}
