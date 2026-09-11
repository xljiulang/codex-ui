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
use std::process::{Output, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

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
    publish_initial(&path, entries)
}

/// 磁盘读取期间后台可能已发布新快照；只允许首次读取填充空槽。
fn publish_initial(path: &Path, entries: Arc<Vec<Value>>) -> Arc<Vec<Value>> {
    let mut guard = snapshot_slot().lock().unwrap_or_else(|error| error.into_inner());
    Arc::clone(guard.entry(path.to_path_buf()).or_insert(entries))
}

/// 模板基底：导出目录的第一条（版本相关，确定性即可）。
#[cfg(test)]
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
    // 同路径刷新串行执行，避免旧导出迟到覆盖新发布、临时文件互相覆盖。
    static REFRESH_LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let refresh_lock = {
        let mut locks = REFRESH_LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
            .lock().unwrap_or_else(|error| error.into_inner());
        Arc::clone(locks.entry(cache_path(app_dir)).or_default())
    };
    let _refresh_guard = refresh_lock.lock().await;
    let mut cmd = Command::new(codex);
    cmd.args(["debug", "models", "--bundled"]);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    apply_codex_env(cmd.as_std_mut());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动 codex debug models 失败: {e}"))?;
    let output = wait_for_export(&mut child, EXPORT_TIMEOUT).await?;
    if !output.status.success() {
        return Err(format!("codex debug models 退出码 {}", output.status));
    }
    // 只看 stdout：该命令会往 stderr 打 PATH 别名之类的 WARNING
    store_export(app_dir, &String::from_utf8_lossy(&output.stdout))
}

/// 同时排空两条管道，超时则明确终止并回收子进程，不留下后台读取任务。
async fn wait_for_export(child: &mut Child, timeout: Duration) -> Result<Output, String> {
    let mut stdout = child.stdout.take().ok_or("导出缺少 stdout 管道")?;
    let mut stderr = child.stderr.take().ok_or("导出缺少 stderr 管道")?;
    let mut output = Vec::new();
    let mut errors = Vec::new();
    let result = tokio::time::timeout(timeout, async {
        tokio::try_join!(child.wait(), stdout.read_to_end(&mut output), stderr.read_to_end(&mut errors))
    }).await;
    match result {
        Ok(Ok((status, _, _))) => Ok(Output { status, stdout: output, stderr: errors }),
        Ok(Err(error)) => {
            let _ = child.kill().await;
            Err(format!("执行 codex debug models 失败: {error}"))
        }
        Err(_) => {
            child.kill().await.map_err(|error| format!("导出超时且终止子进程失败: {error}"))?;
            Err(format!("codex debug models 超时（{}ms），子进程已终止", timeout.as_millis()))
        }
    }
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

    /// 仅由超时测试启动的子进程阻塞；正常单测调用直接返回。
    #[test]
    fn export_timeout_child() {
        if std::env::var_os("CODEX_UI_EXPORT_TIMEOUT_TEST").is_some() {
            loop { std::thread::park(); }
        }
    }

    #[tokio::test]
    async fn export_timeout_terminates_and_reaps_child() {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "codex::model_catalog::codex_models::tests::export_timeout_child"])
            .env("CODEX_UI_EXPORT_TIMEOUT_TEST", "1")
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);
        let mut child = command.spawn().unwrap();
        let error = wait_for_export(&mut child, Duration::from_millis(100)).await.unwrap_err();
        assert!(error.contains("超时"));
        assert!(child.try_wait().unwrap().is_some(), "超时后必须已回收子进程");
    }

    #[test]
    fn late_initial_read_does_not_replace_refreshed_snapshot() {
        let dir = tempdir().unwrap();
        let path = cache_path(dir.path());
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let reader_barrier = Arc::clone(&barrier);
        let reader_path = path.clone();
        let reader = std::thread::spawn(move || {
            let old = Arc::new(vec![json!({"slug":"old"})]);
            reader_barrier.wait();
            reader_barrier.wait();
            publish_initial(&reader_path, old)
        });
        barrier.wait();
        remember(&path, Arc::new(vec![json!({"slug":"new"})]));
        barrier.wait();
        assert_eq!(reader.join().unwrap()[0]["slug"], json!("new"));
        assert_eq!(codex_snapshot(dir.path())[0]["slug"], json!("new"));
    }

    #[test]
    fn captured_snapshot_is_shared_by_official_source_and_template() {
        let dir = tempdir().unwrap();
        write_export_for_test(dir.path(), vec![json!({
            "slug":"generation-old", "model_messages":{"instructions_template":"old"}
        })]);
        let captured = codex_snapshot(dir.path());
        write_export_for_test(dir.path(), vec![json!({
            "slug":"generation-new", "model_messages":{"instructions_template":"new"}
        })]);
        let sources = super::super::sources::full_entry_sources(&captured, "https://relay.example.com");
        assert!(sources[0].full_entry("generation-old").is_some());
        assert!(sources[0].full_entry("generation-new").is_none());
        let template = super::super::template::from_snapshot(&captured).unwrap();
        assert_eq!(template.entry["model_messages"]["instructions_template"], json!("old"));
    }

    #[tokio::test]
    async fn refresh_failure_retains_previous_file_and_snapshot() {
        let dir = tempdir().unwrap();
        write_export_for_test(dir.path(), vec![json!({
            "slug":"old", "model_messages":{"instructions_template":"old"}
        })]);
        let before = std::fs::read(cache_path(dir.path())).unwrap();
        assert!(refresh_codex_models(dir.path(), &dir.path().join("missing-codex.exe")).await.is_err());
        assert_eq!(std::fs::read(cache_path(dir.path())).unwrap(), before);
        assert_eq!(codex_snapshot(dir.path())[0]["slug"], json!("old"));
    }

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
