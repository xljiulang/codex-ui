//! 模型快照：在 `CODEX_HOME/codex-ui/<名称>.json` 下保存/还原模型配置。
//!
//! 快照只包含模型相关字段、原始 `[model_providers.*]` 表、`model_catalog_json`
//! 字段及模型目录文件解析后的 JSON 内容；还原时直接读写 config.toml 与模型目录
//! 文件，不调用 app-server。config.toml 通过同目录临时文件 + 原子替换写入，
//! 还原失败时保留旧文件完整内容。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};
use toml_edit::{Array, DocumentMut, Item, Table, Value as TomlValue};

use crate::codex::model_config;
use crate::codex::path_util::{clean_path, norm_key};

const SNAPSHOT_VERSION: u32 = 1;
const SNAPSHOTS_DIR: &str = "codex-ui";
const FALLBACK_CATALOG_NAME: &str = "model_catalog.json";
static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// 单个模型快照（JSON 文件内容）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSnapshot {
    pub version: u32,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub model_reasoning_effort: String,
    #[serde(default)]
    pub personality: String,
    #[serde(default)]
    pub model_verbosity: String,
    #[serde(default)]
    pub model_provider: String,
    #[serde(default)]
    pub preferred_auth_method: String,
    #[serde(default)]
    pub forced_login_method: String,
    /// config.toml 中 model_catalog_json 的原始值。
    #[serde(default)]
    pub model_catalog_json: String,
    /// 原始 [model_providers.*] 表（保留未知字段与密钥）。
    #[serde(default)]
    pub model_providers: Map<String, Value>,
    /// 模型目录文件解析后的 JSON；文件不存在时为 None。
    #[serde(default)]
    pub model_catalog: Option<Value>,
}

/// 还原结果：返回实际写入的目录字段与路径，便于前端提示/诊断。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedModelSnapshot {
    pub model_catalog_json: Option<String>,
    pub model_catalog_path: String,
}

struct CatalogTarget {
    field: Option<String>,
    path: Option<PathBuf>,
    content: Option<String>,
}

fn snapshots_root(home: &Path) -> PathBuf {
    home.join(SNAPSHOTS_DIR)
}

fn normalize_name(name: &str) -> Result<String, String> {
    if name.trim() != name {
        return Err("快照名不能包含首尾空格".into());
    }
    let mut name = name;
    if name.to_ascii_lowercase().ends_with(".json") {
        name = &name[..name.len() - ".json".len()];
    }
    if name.trim() != name {
        return Err("快照名不能包含首尾空格".into());
    }
    if name.is_empty() {
        return Err("快照名不能为空".into());
    }
    if name == "." || name == ".." {
        return Err(format!("快照名「{name}」非法"));
    }
    if let Some(c) = name.chars().find(|c| r#"\/:*?"<>|"#.contains(*c)) {
        return Err(format!("快照名「{name}」包含非法字符「{c}」"));
    }
    let base = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if reserved.contains(&base.as_str()) {
        return Err(format!("快照名「{name}」是系统保留名"));
    }
    if name.starts_with('.') || name.starts_with(' ') || name.ends_with('.') || name.ends_with(' ')
    {
        return Err(format!("快照名「{name}」不能以点或空格开头/结尾"));
    }
    Ok(name.to_string())
}

fn snapshot_path(home: &Path, name: &str) -> Result<PathBuf, String> {
    let name = normalize_name(name)?;
    let root = snapshots_root(home);
    let path = root.join(format!("{name}.json"));
    if path.parent() != Some(root.as_path()) {
        return Err(format!("非法的快照路径「{name}」"));
    }
    Ok(path)
}

fn doc_string(doc: &DocumentMut, key: &str) -> String {
    doc.get(key)
        .and_then(Item::as_str)
        .unwrap_or("")
        .to_string()
}

fn toml_item_to_json(item: &Item) -> Value {
    match item {
        Item::None => Value::Null,
        Item::Value(value) => toml_value_to_json(value),
        Item::Table(table) => Value::Object(
            table
                .iter()
                .map(|(key, item)| (key.to_string(), toml_item_to_json(item)))
                .collect(),
        ),
        Item::ArrayOfTables(array) => Value::Array(
            array
                .iter()
                .map(|table| {
                    Value::Object(
                        table
                            .iter()
                            .map(|(key, item)| (key.to_string(), toml_item_to_json(item)))
                            .collect(),
                    )
                })
                .collect(),
        ),
    }
}

fn toml_value_to_json(value: &TomlValue) -> Value {
    if let Some(v) = value.as_str() {
        return Value::String(v.to_string());
    }
    if let Some(v) = value.as_integer() {
        return Value::Number(Number::from(v));
    }
    if let Some(v) = value.as_float() {
        return Number::from_f64(v)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(v.to_string()));
    }
    if let Some(v) = value.as_bool() {
        return Value::Bool(v);
    }
    if let Some(v) = value.as_datetime() {
        return Value::String(v.to_string());
    }
    if let Some(v) = value.as_array() {
        return Value::Array(v.iter().map(toml_value_to_json).collect());
    }
    if let Some(v) = value.as_inline_table() {
        return Value::Object(
            v.iter()
                .map(|(key, value)| (key.to_string(), toml_value_to_json(value)))
                .collect(),
        );
    }
    Value::String(value.to_string())
}

fn json_to_toml_item(value: &Value) -> Item {
    match value {
        Value::Null => Item::None,
        Value::Bool(v) => toml_edit::value(*v),
        Value::Number(v) => {
            if let Some(i) = v.as_i64() {
                toml_edit::value(i)
            } else if let Some(f) = v.as_f64() {
                toml_edit::value(f)
            } else {
                toml_edit::value(v.to_string())
            }
        }
        Value::String(v) => toml_edit::value(v.clone()),
        Value::Array(values) => {
            let mut array = Array::new();
            for value in values {
                if let Item::Value(value) = json_to_toml_item(value) {
                    array.push_formatted(value);
                }
            }
            Item::Value(TomlValue::Array(array))
        }
        Value::Object(values) => {
            let mut table = Table::new();
            for (key, value) in values {
                table.insert(key, json_to_toml_item(value));
            }
            Item::Table(table)
        }
    }
}

fn read_config_doc(home: &Path) -> Result<DocumentMut, String> {
    let path = model_config::config_path_in(home);
    if !path.is_file() {
        return Ok(DocumentMut::new());
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("读取 config.toml 失败: {e}"))?;
    text.parse::<DocumentMut>()
        .map_err(|e| format!("config.toml 不是合法 TOML: {e}"))
}

fn current_snapshot(home: &Path) -> Result<ModelSnapshot, String> {
    let doc = read_config_doc(home)?;
    let mut providers = Map::new();
    if let Some(table) = doc.get("model_providers").and_then(Item::as_table) {
        for (key, item) in table.iter() {
            providers.insert(key.to_string(), toml_item_to_json(item));
        }
    }
    let catalog_value = doc_string(&doc, "model_catalog_json");
    let catalog = if catalog_value.trim().is_empty() {
        None
    } else {
        let target = model_config::resolve_catalog_path(home, Some(catalog_value.as_str()));
        if target.is_file() {
            let text =
                fs::read_to_string(&target).map_err(|e| format!("读取模型目录文件失败: {e}"))?;
            Some(
                serde_json::from_str::<Value>(&text)
                    .map_err(|e| format!("模型目录不是合法 JSON: {e}"))?,
            )
        } else {
            None
        }
    };
    Ok(ModelSnapshot {
        version: SNAPSHOT_VERSION,
        model: doc_string(&doc, "model"),
        model_reasoning_effort: doc_string(&doc, "model_reasoning_effort"),
        personality: doc_string(&doc, "personality"),
        model_verbosity: doc_string(&doc, "model_verbosity"),
        model_provider: doc_string(&doc, "model_provider"),
        preferred_auth_method: doc_string(&doc, "preferred_auth_method"),
        forced_login_method: doc_string(&doc, "forced_login_method"),
        model_catalog_json: catalog_value,
        model_providers: providers,
        model_catalog: catalog,
    })
}

fn read_snapshot(home: &Path, name: &str) -> Result<ModelSnapshot, String> {
    let path = snapshot_path(home, name)?;
    let text = fs::read_to_string(&path).map_err(|e| format!("读取模型快照「{name}」失败: {e}"))?;
    let snapshot: ModelSnapshot =
        serde_json::from_str(&text).map_err(|e| format!("模型快照「{name}」不是合法 JSON: {e}"))?;
    if snapshot.version != SNAPSHOT_VERSION {
        return Err(format!(
            "模型快照「{name}」版本不支持（{}），当前支持版本 {SNAPSHOT_VERSION}",
            snapshot.version
        ));
    }
    Ok(snapshot)
}

fn write_snapshot(home: &Path, name: &str, snapshot: &ModelSnapshot) -> Result<(), String> {
    let path = snapshot_path(home, name)?;
    let text =
        serde_json::to_string_pretty(snapshot).map_err(|e| format!("序列化模型快照失败: {e}"))?;
    atomic_write(&path, text.as_bytes())
}

fn resolve_catalog_target(
    home: &Path,
    snapshot_path: &Path,
    snapshot: &ModelSnapshot,
) -> Result<CatalogTarget, String> {
    let raw = snapshot.model_catalog_json.trim();
    let Some(content) = snapshot.model_catalog.as_ref() else {
        return Ok(CatalogTarget {
            field: (!raw.is_empty()).then(|| raw.to_string()),
            path: None,
            content: None,
        });
    };
    let content =
        serde_json::to_string_pretty(content).map_err(|e| format!("序列化模型目录失败: {e}"))?;
    if !raw.is_empty() {
        let candidate = model_config::resolve_catalog_path(home, Some(raw));
        let usable = !candidate.is_dir()
            && norm_key(&candidate) != norm_key(snapshot_path)
            && (candidate.is_file()
                || candidate
                    .parent()
                    .map(|parent| parent.is_dir())
                    .unwrap_or(false));
        if usable {
            return Ok(CatalogTarget {
                field: Some(raw.to_string()),
                path: Some(candidate),
                content: Some(content),
            });
        }
    }
    let fallback = home.join(FALLBACK_CATALOG_NAME);
    Ok(CatalogTarget {
        field: Some(FALLBACK_CATALOG_NAME.to_string()),
        path: Some(fallback),
        content: Some(content),
    })
}

fn set_or_remove(doc: &mut DocumentMut, key: &str, value: &str) {
    if value.trim().is_empty() {
        doc.remove(key);
    } else {
        doc.insert(key, toml_edit::value(value));
    }
}

fn apply_model_fields(
    doc: &mut DocumentMut,
    snapshot: &ModelSnapshot,
    catalog_field: Option<&str>,
) {
    set_or_remove(doc, "model", &snapshot.model);
    set_or_remove(
        doc,
        "model_reasoning_effort",
        &snapshot.model_reasoning_effort,
    );
    set_or_remove(doc, "personality", &snapshot.personality);
    set_or_remove(doc, "model_verbosity", &snapshot.model_verbosity);
    set_or_remove(doc, "model_provider", &snapshot.model_provider);
    set_or_remove(
        doc,
        "preferred_auth_method",
        &snapshot.preferred_auth_method,
    );
    set_or_remove(doc, "forced_login_method", &snapshot.forced_login_method);
    match catalog_field {
        Some(value) if !value.trim().is_empty() => {
            doc.insert("model_catalog_json", toml_edit::value(value));
        }
        _ => {
            doc.remove("model_catalog_json");
        }
    }
    if snapshot.model_providers.is_empty() {
        doc.remove("model_providers");
    } else {
        doc.insert(
            "model_providers",
            json_to_toml_item(&Value::Object(snapshot.model_providers.clone())),
        );
    }
}

fn temp_path(target: &Path) -> Result<PathBuf, String> {
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("无法确定临时文件名: {}", clean_path(target)))?;
    Ok(target.with_file_name(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        TEMP_SEQ.fetch_add(1, Ordering::Relaxed)
    )))
}

fn write_temp(target: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let tmp = temp_path(target)?;
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp)
        .map_err(|e| format!("写入临时文件失败: {e}"))?;
    file.write_all(bytes)
        .map_err(|e| format!("写入临时文件失败: {e}"))?;
    file.sync_all()
        .map_err(|e| format!("同步临时文件失败: {e}"))?;
    Ok(tmp)
}

fn atomic_replace(tmp: &Path, target: &Path) -> Result<(), String> {
    if fs::rename(tmp, target).is_ok() {
        return Ok(());
    }
    // 平台 rename 未替换既有文件时的兜底：备份旧文件 → 替换 → 失败回滚。
    let backup = target.with_file_name(format!(
        ".{}.{}.codex-ui-bak",
        target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("backup"),
        std::process::id()
    ));
    let had_target = target.exists();
    if had_target {
        if let Err(e) = fs::rename(target, &backup) {
            let _ = fs::remove_file(tmp);
            return Err(format!("备份旧文件失败: {e}"));
        }
    }
    match fs::rename(tmp, target) {
        Ok(()) => {
            if had_target {
                let _ = fs::remove_file(&backup);
            }
            Ok(())
        }
        Err(e) => {
            let _ = fs::remove_file(tmp);
            if had_target {
                let _ = fs::rename(&backup, target);
            }
            Err(format!("原子替换失败: {e}"))
        }
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = write_temp(path, bytes)?;
    atomic_replace(&tmp, path)
}

fn list_in(home: &Path) -> Result<Vec<String>, String> {
    let root = snapshots_root(home);
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(&root).map_err(|e| format!("读取模型快照目录失败: {e}"))? {
        let entry = entry.map_err(|e| format!("读取模型快照目录项失败: {e}"))?;
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        if path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("json"))
            != Some(true)
        {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            names.push(stem.to_string());
        }
    }
    names.sort_by(|a, b| {
        a.to_lowercase()
            .cmp(&b.to_lowercase())
            .then_with(|| a.cmp(b))
    });
    Ok(names)
}

fn delete_in(home: &Path, name: &str) -> Result<(), String> {
    let path = snapshot_path(home, name)?;
    if path.is_file() {
        fs::remove_file(&path).map_err(|e| format!("删除模型快照失败: {e}"))?;
    }
    Ok(())
}

fn open_in(home: &Path, name: &str) -> Result<String, String> {
    let path = snapshot_path(home, name)?;
    if !path.is_file() {
        return Err(format!("模型快照「{name}」不存在"));
    }
    Ok(clean_path(&path))
}

fn save_in(home: &Path, name: &str) -> Result<(), String> {
    let snapshot = current_snapshot(home)?;
    write_snapshot(home, name, &snapshot)
}

fn apply_in(home: &Path, name: &str) -> Result<AppliedModelSnapshot, String> {
    let snapshot = read_snapshot(home, name)?;
    let path = snapshot_path(home, name)?;
    let mut doc = read_config_doc(home)?;
    let catalog = resolve_catalog_target(home, &path, &snapshot)?;
    apply_model_fields(&mut doc, &snapshot, catalog.field.as_deref());
    let config_text = doc.to_string();
    config_text
        .parse::<DocumentMut>()
        .map_err(|e| format!("生成的新 config.toml 不是合法 TOML: {e}"))?;

    let config_path = model_config::config_path_in(home);
    let old_catalog = catalog
        .path
        .as_ref()
        .filter(|target| target.is_file())
        .map(|target| fs::read(target).map_err(|e| format!("读取旧模型目录失败: {e}")))
        .transpose()?;
    let config_tmp = write_temp(&config_path, config_text.as_bytes())?;
    let catalog_tmp = match (&catalog.path, &catalog.content) {
        (Some(target), Some(content)) => match write_temp(target, content.as_bytes()) {
            Ok(tmp) => Some((target.clone(), tmp)),
            Err(e) => {
                let _ = fs::remove_file(&config_tmp);
                return Err(e);
            }
        },
        _ => None,
    };

    // 先提交目录文件，再提交 config.toml；config 提交失败时回滚目录文件。
    if let Some((target, tmp)) = &catalog_tmp {
        if let Err(e) = atomic_replace(tmp, target) {
            let _ = fs::remove_file(&config_tmp);
            return Err(e);
        }
    }
    if let Err(e) = atomic_replace(&config_tmp, &config_path) {
        if let Some((target, _)) = &catalog_tmp {
            match old_catalog {
                Some(bytes) => {
                    if let Err(rollback) = atomic_write(target, &bytes) {
                        return Err(format!(
                            "{e}；模型目录回滚失败：{rollback}（config.toml 保持原内容）"
                        ));
                    }
                }
                None => {
                    let _ = fs::remove_file(target);
                }
            }
        }
        return Err(format!("{e}（config.toml 保持原内容）"));
    }

    Ok(AppliedModelSnapshot {
        model_catalog_json: catalog.field,
        model_catalog_path: catalog.path.map(|p| clean_path(&p)).unwrap_or_default(),
    })
}

#[tauri::command]
pub fn model_snapshots_list() -> Result<Vec<String>, String> {
    list_in(&model_config::codex_home()?)
}

#[tauri::command]
pub fn model_snapshots_save(name: String) -> Result<(), String> {
    save_in(&model_config::codex_home()?, &name)
}

#[tauri::command]
pub fn model_snapshots_apply(name: String) -> Result<AppliedModelSnapshot, String> {
    apply_in(&model_config::codex_home()?, &name)
}

#[tauri::command]
pub fn model_snapshots_delete(name: String) -> Result<(), String> {
    delete_in(&model_config::codex_home()?, &name)
}

#[tauri::command]
pub fn model_snapshots_open(name: String) -> Result<String, String> {
    open_in(&model_config::codex_home()?, &name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn sample_snapshot() -> ModelSnapshot {
        let mut providers = Map::new();
        providers.insert(
            "deepseek".into(),
            serde_json::json!({
                "name": "DeepSeek",
                "base_url": "https://api.deepseek.com/",
                "experimental_bearer_token": "sk-secret",
                "wire_api": "responses",
                "custom": 42
            }),
        );
        ModelSnapshot {
            version: SNAPSHOT_VERSION,
            model: "deepseek-v4-flash".into(),
            model_reasoning_effort: "high".into(),
            personality: "pragmatic".into(),
            model_verbosity: "low".into(),
            model_provider: "deepseek".into(),
            preferred_auth_method: "apikey".into(),
            forced_login_method: "api".into(),
            model_catalog_json: "models.json".into(),
            model_providers: providers,
            model_catalog: Some(serde_json::json!({"models":[{"slug":"deepseek-v4-flash"}]})),
        }
    }

    #[test]
    fn name_validation_strips_json_and_rejects_unsafe_names() {
        assert_eq!(normalize_name("dev.json").unwrap(), "dev");
        assert_eq!(normalize_name("配置1").unwrap(), "配置1");
        for bad in ["", "   ", ".", "..", "a/b", "a\\b", "con", "x.", " x", "x "] {
            assert!(normalize_name(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn list_only_json_files_ignores_legacy_directories() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        fs::create_dir_all(snapshots_root(home).join("legacy")).unwrap();
        write(&snapshots_root(home).join("b.json"), "{}");
        write(&snapshots_root(home).join("a.json"), "{}");
        write(&snapshots_root(home).join("ignore.txt"), "x");
        assert_eq!(list_in(home).unwrap(), vec!["a", "b"]);
    }

    #[test]
    fn save_and_read_roundtrip_keeps_raw_providers_and_catalog() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        write(
            &model_config::config_path_in(home),
            r#"model = "gpt"
model_reasoning_effort = "high"
model_provider = "deepseek"
preferred_auth_method = "apikey"
forced_login_method = "api"
model_catalog_json = "models.json"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com/"
experimental_bearer_token = "sk-secret"
wire_api = "responses"
custom = 42
"#,
        );
        write(
            &home.join("models.json"),
            r#"{"models":[{"slug":"deepseek-v4-flash"}]}"#,
        );
        save_in(home, "dev").unwrap();
        let snapshot = read_snapshot(home, "dev").unwrap();
        assert_eq!(snapshot.model, "gpt");
        assert_eq!(snapshot.model_providers["deepseek"]["custom"], 42);
        assert_eq!(
            snapshot.model_providers["deepseek"]["experimental_bearer_token"],
            "sk-secret"
        );
        assert_eq!(
            snapshot.model_catalog.unwrap()["models"][0]["slug"],
            "deepseek-v4-flash"
        );
        assert!(snapshot_path(home, "dev").unwrap().is_file());
    }

    #[test]
    fn save_rejects_invalid_toml_and_invalid_catalog_json() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        write(&model_config::config_path_in(home), "not valid toml = = [");
        assert!(save_in(home, "dev").is_err());

        write(
            &model_config::config_path_in(home),
            "model_catalog_json = \"models.json\"\n",
        );
        write(&home.join("models.json"), "{ not json");
        assert!(save_in(home, "dev").is_err());
    }

    #[test]
    fn apply_only_replaces_model_fields_and_preserves_other_config() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        write(
            &model_config::config_path_in(home),
            r#"# 保留注释
instructions = "keep me"
model = "old"

[mcp_servers.filesystem]
command = "npx"

[model_providers.old]
name = "Old"
"#,
        );
        write_snapshot(home, "dev", &sample_snapshot()).unwrap();
        apply_in(home, "dev").unwrap();
        let text = fs::read_to_string(model_config::config_path_in(home)).unwrap();
        assert!(text.contains("# 保留注释"));
        assert!(text.contains("instructions = \"keep me\""));
        assert!(text.contains("[mcp_servers.filesystem]"));
        assert!(text.contains("model = \"deepseek-v4-flash\""));
        assert!(text.contains("experimental_bearer_token = \"sk-secret\""));
        assert!(text.contains("custom = 42"));
        assert!(!text.contains("model_providers.old"));
    }

    #[test]
    fn apply_falls_back_to_model_catalog_json_when_target_missing() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let mut snapshot = sample_snapshot();
        snapshot.model_catalog_json = "missing/custom.json".into();
        write_snapshot(home, "dev", &snapshot).unwrap();
        let applied = apply_in(home, "dev").unwrap();
        assert_eq!(
            applied.model_catalog_json.as_deref(),
            Some("model_catalog.json")
        );
        assert_eq!(
            applied.model_catalog_path,
            clean_path(&home.join("model_catalog.json"))
        );
        assert!(home.join("model_catalog.json").is_file());
        let text = fs::read_to_string(model_config::config_path_in(home)).unwrap();
        assert!(text.contains("model_catalog_json = \"model_catalog.json\""));
    }

    #[test]
    fn apply_never_overwrites_snapshot_file_with_catalog_content() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let mut snapshot = sample_snapshot();
        snapshot.model_catalog_json = "codex-ui/dev.json".into();
        write_snapshot(home, "dev", &snapshot).unwrap();
        let before = fs::read_to_string(snapshot_path(home, "dev").unwrap()).unwrap();
        let applied = apply_in(home, "dev").unwrap();
        assert_eq!(
            applied.model_catalog_json.as_deref(),
            Some("model_catalog.json")
        );
        assert_eq!(
            fs::read_to_string(snapshot_path(home, "dev").unwrap()).unwrap(),
            before
        );
    }

    #[test]
    fn apply_uses_existing_catalog_path_and_keeps_original_field() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let target = home.join("catalog").join("custom.json");
        write(&target, r#"{"models":[]}"#);
        let mut snapshot = sample_snapshot();
        snapshot.model_catalog_json = "catalog/custom.json".into();
        write_snapshot(home, "dev", &snapshot).unwrap();
        let applied = apply_in(home, "dev").unwrap();
        assert_eq!(
            applied.model_catalog_json.as_deref(),
            Some("catalog/custom.json")
        );
        assert_eq!(applied.model_catalog_path, clean_path(&target));
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            serde_json::to_string_pretty(&snapshot.model_catalog.unwrap()).unwrap()
        );
    }

    #[test]
    fn apply_without_catalog_content_does_not_write_catalog() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let mut snapshot = sample_snapshot();
        snapshot.model_catalog = None;
        snapshot.model_catalog_json = "models.json".into();
        write_snapshot(home, "dev", &snapshot).unwrap();
        let applied = apply_in(home, "dev").unwrap();
        assert_eq!(applied.model_catalog_json.as_deref(), Some("models.json"));
        assert_eq!(applied.model_catalog_path, "");
        assert!(!home.join("models.json").exists());
    }

    #[test]
    fn apply_rejects_invalid_current_config_before_writing() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        write_snapshot(home, "dev", &sample_snapshot()).unwrap();
        let broken = "not valid toml = = [";
        write(&model_config::config_path_in(home), broken);
        assert!(apply_in(home, "dev").is_err());
        assert_eq!(
            fs::read_to_string(model_config::config_path_in(home)).unwrap(),
            broken
        );
    }

    #[test]
    fn apply_rejects_unsupported_version() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let mut snapshot = sample_snapshot();
        snapshot.version = 99;
        write_snapshot(home, "dev", &snapshot).unwrap();
        assert!(apply_in(home, "dev").is_err());
    }

    #[test]
    fn atomic_replace_failure_keeps_old_file_complete() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("config.toml");
        fs::write(&target, "old").unwrap();
        let missing = dir.path().join("missing.tmp");
        assert!(atomic_replace(&missing, &target).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "old");
    }

    #[test]
    fn delete_and_open_manage_single_json_file() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        write_snapshot(home, "dev", &sample_snapshot()).unwrap();
        let path = snapshot_path(home, "dev").unwrap();
        assert_eq!(open_in(home, "dev").unwrap(), clean_path(&path));
        delete_in(home, "dev").unwrap();
        assert!(!path.exists());
        assert!(open_in(home, "dev").is_err());
    }
}
