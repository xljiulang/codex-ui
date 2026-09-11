//! 本机 codex 官方条目导出：`codex debug models --bundled` → 运行时缓存。
//!
//! `resources/official-models.json` 只内置第三方官方条目；GPT/codex 基线条目随用户安装的
//! codex 版本变化，启动时从该二进制自带的目录导出，避免把研发环境的字段写到老版本 codex 上。
//! `--bundled` 不读 `CODEX_HOME/config.toml`、不联网；导出失败保留上次缓存。
//!
//! 导出结果以**进程内快照**复用：每次启动最多跑一次子进程，生成目录时只读快照
//! （官方条目池与模板基底共用同一份，保证两者来自同一次导出）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::Value;
use tokio::process::Command;

use super::sources::write_atomic;
use crate::codex::app_server::apply_codex_env;

/// 导出超时：本机实测约 2s（含 codex CLI 冷启动），留足冷启动余量。
const EXPORT_TIMEOUT: Duration = Duration::from_secs(30);
const CACHE_FILE: &str = "codex-models.json";

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 运行时缓存路径：`<app_dir>/cache/codex-models.json`。
pub fn cache_path(app_dir: &Path) -> PathBuf {
    app_dir.join("cache").join(CACHE_FILE)
}

/// 进程内快照（按缓存路径索引，避免「换目录读旧快照」；实际使用只有一个应用数据目录）。
static SNAPSHOT: OnceLock<Mutex<HashMap<PathBuf, Arc<Vec<Value>>>>> = OnceLock::new();

fn snapshot_slot() -> &'static Mutex<HashMap<PathBuf, Arc<Vec<Value>>>> {
    SNAPSHOT.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 取导出条目的进程内快照：内存命中直接返回，未命中才读一次磁盘缓存。
///
/// 生成目录时官方条目池与模板基底都走这里，因此不会重复解析、也不会再起子进程。
pub fn codex_snapshot(app_dir: &Path) -> Arc<Vec<Value>> {
    let path = cache_path(app_dir);
    if let Ok(guard) = snapshot_slot().lock() {
        if let Some(entries) = guard.get(&path) {
            return Arc::clone(entries);
        }
    }
    let entries = Arc::new(read_entries(&path));
    remember(&path, Arc::clone(&entries));
    entries
}

/// 模板基底：导出目录的第一条（版本相关，确定性即可）。
pub fn template_base(app_dir: &Path) -> Option<Value> {
    codex_snapshot(app_dir).first().cloned()
}

fn read_entries(path: &Path) -> Vec<Value> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_export(&text).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn remember(path: &Path, entries: Arc<Vec<Value>>) {
    if let Ok(mut guard) = snapshot_slot().lock() {
        guard.insert(path.to_path_buf(), entries);
    }
}

/// 导出本机 codex 自带的官方条目并写入缓存；失败返回原因，调用方记日志并保留旧缓存。
pub async fn refresh_codex_models(app_dir: &Path, codex: &Path) -> Result<(), String> {
    let mut cmd = Command::new(codex);
    cmd.args(["debug", "models", "--bundled"]);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    apply_codex_env(cmd.as_std_mut());

    let child = cmd
        .spawn()
        .map_err(|e| format!("启动 codex debug models 失败: {e}"))?;
    let output = tokio::time::timeout(EXPORT_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| format!("codex debug models 超时（{}s）", EXPORT_TIMEOUT.as_secs()))?
        .map_err(|e| format!("执行 codex debug models 失败: {e}"))?;
    if !output.status.success() {
        return Err(format!("codex debug models 退出码 {}", output.status));
    }
    // 只看 stdout：该命令会往 stderr 打 PATH 别名之类的 WARNING
    store_export(app_dir, &String::from_utf8_lossy(&output.stdout))
}

/// 校验导出文本并原子写入缓存；校验不通过时不动旧文件。
///
/// 成功写盘后同步刷新进程内快照（失败保留旧快照）。
fn store_export(app_dir: &Path, text: &str) -> Result<(), String> {
    let entries = Arc::new(parse_export(text)?);
    let path = cache_path(app_dir);
    write_atomic(&path, text.trim().as_bytes())?;
    remember(&path, entries);
    Ok(())
}

/// 测试辅助：按正式校验路径写入导出缓存。
#[cfg(test)]
pub(crate) fn write_export_for_test(app_dir: &Path, models: Vec<Value>) {
    store_export(app_dir, &serde_json::json!({ "models": models }).to_string()).unwrap();
}

/// 解析并校验 `codex debug models --bundled` 的输出：
/// 顶层 `models` 数组非空，且每条都有非空 `slug` 与 `model_messages.instructions_template`。
fn parse_export(text: &str) -> Result<Vec<Value>, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| format!("codex 模型目录不是合法 JSON: {e}"))?;
    let entries = value
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| "codex 模型目录缺少 models 数组".to_string())?;
    if entries.is_empty() {
        return Err("codex 模型目录为空".to_string());
    }
    for entry in entries {
        if !entry.is_object() {
            return Err("codex 模型目录存在非对象条目".to_string());
        }
        let slug = entry
            .get("slug")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if slug.is_empty() {
            return Err("codex 模型目录存在缺少 slug 的条目".to_string());
        }
        let template = entry
            .pointer("/model_messages/instructions_template")
            .and_then(Value::as_str)
            .unwrap_or("");
        if template.trim().is_empty() {
            return Err(format!("codex 模型目录条目 {slug} 缺少 instructions_template"));
        }
    }
    Ok(entries.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn export_text(slugs: &[&str]) -> String {
        let models: Vec<Value> = slugs
            .iter()
            .map(|slug| {
                json!({
                    "slug": slug,
                    "model_messages": { "instructions_template": "You are Codex" }
                })
            })
            .collect();
        json!({ "models": models }).to_string()
    }

    #[test]
    fn store_export_round_trips_valid_payload() {
        let dir = tempdir().unwrap();
        store_export(dir.path(), &export_text(&["gpt-5.6-sol", "codex-auto-review"])).unwrap();

        let entries = codex_snapshot(dir.path());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["slug"], json!("gpt-5.6-sol"));
        assert!(cache_path(dir.path()).exists());
        // 快照与文件同源
        assert_eq!(codex_snapshot(dir.path()).len(), 2);
        assert_eq!(template_base(dir.path()).unwrap()["slug"], json!("gpt-5.6-sol"));
    }

    #[test]
    fn store_export_rejects_invalid_payload_without_touching_cache() {
        let dir = tempdir().unwrap();
        store_export(dir.path(), &export_text(&["gpt-5.6-sol"])).unwrap();
        let before = std::fs::read_to_string(cache_path(dir.path())).unwrap();

        let invalid = [
            "{ broken".to_string(),
            json!({ "models": [] }).to_string(),
            json!({ "models": [{ "model_messages": { "instructions_template": "x" } }] })
                .to_string(),
            json!({ "models": [{ "slug": "gpt-5.6-sol" }] }).to_string(),
            json!({ "models": ["gpt-5.6-sol"] }).to_string(),
            json!({ "providers": {} }).to_string(),
        ];
        for text in &invalid {
            assert!(store_export(dir.path(), text).is_err(), "应拒绝: {text}");
        }

        assert_eq!(
            std::fs::read_to_string(cache_path(dir.path())).unwrap(),
            before,
            "非法导出不得覆盖旧缓存"
        );
        assert_eq!(codex_snapshot(dir.path()).len(), 1);
    }

    #[test]
    fn snapshot_without_cache_is_empty() {
        let dir = tempdir().unwrap();
        assert!(codex_snapshot(dir.path()).is_empty());
        assert!(template_base(dir.path()).is_none());

        // 缓存损坏同样按“没有导出”处理
        std::fs::create_dir_all(dir.path().join("cache")).unwrap();
        std::fs::write(cache_path(dir.path()), "{ broken").unwrap();
        assert!(codex_snapshot(dir.path()).is_empty());
    }

    #[test]
    fn snapshot_is_reused_without_repeated_disk_reads() {
        let dir = tempdir().unwrap();
        write_export_for_test(
            dir.path(),
            vec![json!({
                "slug": "gpt-5.6-sol",
                "model_messages": { "instructions_template": "You are Codex" }
            })],
        );
        assert_eq!(codex_snapshot(dir.path()).len(), 1);

        // 磁盘缓存被删掉后仍能从快照读到（证明生成期只读内存，不再读盘/起子进程）
        std::fs::remove_file(cache_path(dir.path())).unwrap();
        assert_eq!(codex_snapshot(dir.path()).len(), 1);
        assert_eq!(template_base(dir.path()).unwrap()["slug"], json!("gpt-5.6-sol"));
    }

    #[test]
    fn refresh_replaces_snapshot() {
        let dir = tempdir().unwrap();
        write_export_for_test(dir.path(), vec![json!({
            "slug": "gpt-5.6-sol",
            "model_messages": { "instructions_template": "You are Codex" }
        })]);
        assert_eq!(template_base(dir.path()).unwrap()["slug"], json!("gpt-5.6-sol"));

        write_export_for_test(dir.path(), vec![json!({
            "slug": "gpt-5.7-sol",
            "model_messages": { "instructions_template": "You are Codex" }
        })]);
        assert_eq!(codex_snapshot(dir.path()).len(), 1);
        assert_eq!(template_base(dir.path()).unwrap()["slug"], json!("gpt-5.7-sol"));
    }
}
