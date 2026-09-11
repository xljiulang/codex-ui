//! 会话诊断日志：按天滚动写入 `<app_data_dir>/logs/session-YYYY-MM-DD.log`，
//! 保留最近 7 天。仅记录诊断所需的安全字段，不落盘提示词/文件内容/工具输出。

use chrono::{DateTime, Local, NaiveDate};
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 保留天数（含当天共 7 个自然日）
const KEEP_DAYS: i64 = 7;
const DEFAULT_PREFIX: &str = "session-";

pub struct SessionLog {
    dir: PathBuf,
    prefix: String,
    inner: Mutex<Inner>,
}

struct Inner {
    /// 当前日期（YYYY-MM-DD）与已打开的文件句柄
    current: Option<(String, File)>,
    /// 最近一次执行清理的日期，避免每天重复扫描目录
    last_cleanup: Option<String>,
}

impl SessionLog {
    pub fn new(dir: PathBuf) -> Self {
        Self::with_prefix(dir, DEFAULT_PREFIX)
    }

    /// 使用自定义文件名前缀创建日志（如 `codex-`）；目录不可创建时写入静默失败。
    pub fn with_prefix(dir: PathBuf, prefix: &str) -> Self {
        let _ = fs::create_dir_all(&dir);
        Self {
            dir,
            prefix: prefix.to_string(),
            inner: Mutex::new(Inner {
                current: None,
                last_cleanup: None,
            }),
        }
    }

    /// 追加一条日志；写盘失败静默忽略，绝不影响应用主流程。
    pub fn write(&self, level: &str, thread: Option<&str>, event: &str, kv: &[(String, String)]) {
        self.write_at(Local::now(), level, thread, event, kv);
    }

    /// 内部实现：时间由调用方注入，便于测试日期切换与清理逻辑。
    fn write_at(
        &self,
        now: DateTime<Local>,
        level: &str,
        thread: Option<&str>,
        event: &str,
        kv: &[(String, String)],
    ) {
        let date = now.format("%Y-%m-%d").to_string();
        let mut inner = match self.inner.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        let same_day = inner.current.as_ref().map(|(d, _)| d.as_str()) == Some(date.as_str());
        // 当天日志文件被外部删除时（句柄不会因此失效）重新建文件，否则后续日志写进已删除的句柄。
        let deleted = same_day && !log_path(&self.dir, &date, &self.prefix).exists();
        if !same_day || deleted {
            inner.current = open_file(&self.dir, &date, &self.prefix)
                .ok()
                .map(|f| (date.clone(), f));
            if inner.last_cleanup.as_deref() != Some(date.as_str()) {
                cleanup_impl(&self.dir, &date, &self.prefix);
                inner.last_cleanup = Some(date);
            }
        }
        let Some((_, file)) = inner.current.as_mut() else {
            return;
        };
        let mut line = format!(
            "[{}] [{}] thread={} event={}",
            now.format("%Y-%m-%d %H:%M:%S%.3f%:z"),
            level.to_uppercase(),
            thread.unwrap_or("-"),
            event
        );
        for (k, v) in kv {
            line.push(' ');
            line.push_str(k);
            line.push('=');
            line.push_str(&sanitize(v));
        }
        line.push('\n');
        let _ = file.write_all(line.as_bytes());
    }

    /// 白名单抽取 RPC 参数中的安全字段；其余内容（提示词/工具输出等）一律丢弃。
    /// 顶层取 threadId/turnId/itemId/requestId/status，嵌套 turn/item 对象取 id/status/type；
    /// 仅对携带用户输入的写方法额外记录 approvalPolicy/sandbox 策略与输入条数。
    pub fn summarize_params(method: &str, params: &Value) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        let mut push = |key: &str, val: String| {
            if !out.iter().any(|(k, _)| k == key) {
                out.push((key.to_string(), val));
            }
        };
        let Some(obj) = params.as_object() else {
            return out;
        };
        for key in ["threadId", "turnId", "itemId", "requestId", "status"] {
            if let Some(v) = obj.get(key) {
                if !v.is_null() {
                    push(key, value_str(v));
                }
            }
        }
        if let Some(t) = obj.get("turn").and_then(|x| x.as_object()) {
            for (key, tag) in [("id", "turnId"), ("status", "turnStatus")] {
                if let Some(v) = t.get(key) {
                    push(tag, value_str(v));
                }
            }
        }
        if let Some(it) = obj.get("item").and_then(|x| x.as_object()) {
            for (key, tag) in [("id", "itemId"), ("type", "itemType"), ("status", "itemStatus")]
            {
                if let Some(v) = it.get(key) {
                    push(tag, value_str(v));
                }
            }
        }
        if matches!(method, "turn/start" | "turn/steer" | "thread/resume" | "thread/start") {
            if let Some(v) = obj.get("approvalPolicy") {
                push("approvalPolicy", value_str(v));
            }
            for key in ["sandbox", "sandboxPolicy"] {
                if let Some(v) = obj.get(key) {
                    push(key, value_str(v));
                }
            }
            if let Some(arr) = obj.get("input").and_then(|x| x.as_array()) {
                push("inputCount", arr.len().to_string());
            }
        }
        out
    }
}

/// 字符串值原样输出（不带 JSON 引号），非字符串按 JSON 序列化输出。
fn value_str(v: &Value) -> String {
    v.as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| v.to_string())
}

fn log_path(dir: &Path, date: &str, prefix: &str) -> PathBuf {
    dir.join(format!("{prefix}{date}.log"))
}

fn open_file(dir: &Path, date: &str, prefix: &str) -> std::io::Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path(dir, date, prefix))
}

/// 删除日期早于今天-6 天的日志文件（含今天共保留 7 个自然日）。
fn cleanup_impl(dir: &Path, today: &str, prefix: &str) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let Some(today) = parse_date(today) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(date) = name
            .strip_prefix(prefix)
            .and_then(|s| s.strip_suffix(".log"))
        else {
            continue;
        };
        let Some(day) = parse_date(date) else {
            continue;
        };
        if (today - day).num_days() >= KEEP_DAYS {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

/// 单行日志：值中的换行/回车转义为字面量，保证一行一条记录。
fn sanitize(v: &str) -> String {
    v.replace('\r', "\\r").replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::TempDir;

    fn tmp_dir() -> TempDir {
        TempDir::new().unwrap()
    }

    fn read_files(dir: &Path) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for e in fs::read_dir(dir).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let content = fs::read_to_string(e.path()).unwrap();
            out.push((name, content));
        }
        out.sort();
        out
    }

    fn local(y: i32, m: u32, d: u32, h: u32, min: u32, s: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(y, m, d, h, min, s)
            .single()
            .unwrap()
    }

    #[test]
    fn write_creates_daily_file_with_stable_format() {
        let dir = tmp_dir();
        let log = SessionLog::new(dir.path().to_path_buf());
        log.write(
            "info",
            Some("t-1"),
            "turn/started",
            &[("status".to_string(), "running".into())],
        );
        let files = read_files(dir.path());
        assert_eq!(files.len(), 1);
        let (name, content) = &files[0];
        assert!(name.starts_with("session-"));
        assert!(name.ends_with(".log"));
        assert!(content.contains(" [INFO] thread=t-1 event=turn/started status=running"));
        assert!(content.starts_with('['));
    }

    #[test]
    fn date_change_rolls_to_new_file() {
        let dir = tmp_dir();
        let log = SessionLog::new(dir.path().to_path_buf());
        log.write_at(local(2026, 8, 16, 23, 59, 59), "info", Some("t"), "ev", &[]);
        log.write_at(local(2026, 8, 17, 0, 0, 1), "info", Some("t"), "ev", &[]);
        let files = read_files(dir.path());
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|(n, _)| n.contains("2026-08-16")));
        assert!(files.iter().any(|(n, _)| n.contains("2026-08-17")));
    }

    #[test]
    fn deleted_same_day_file_is_recreated() {
        let dir = tmp_dir();
        let log = SessionLog::new(dir.path().to_path_buf());
        log.write_at(local(2026, 9, 12, 10, 0, 0), "info", Some("t"), "ev", &[]);
        let path = dir.path().join("session-2026-09-12.log");
        assert!(path.exists());
        // 模拟外部删除当天日志文件：句柄仍在，后续日志必须重建文件而不是写进已删除的句柄。
        fs::remove_file(&path).unwrap();
        log.write_at(local(2026, 9, 12, 10, 0, 1), "info", Some("t"), "ev2", &[]);
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("event=ev2"));
    }

    #[test]
    fn cleanup_keeps_last_7_days() {
        let dir = tmp_dir();
        for day in 10..=17 {
            let date = format!("2026-08-{day:02}");
            let _ = open_file(dir.path(), &date, DEFAULT_PREFIX).unwrap();
        }
        cleanup_impl(dir.path(), "2026-08-17", DEFAULT_PREFIX);
        let names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(!names.iter().any(|n| n.contains("2026-08-10")));
        for day in 11..=17 {
            assert!(names.iter().any(|n| n.contains(&format!("2026-08-{day:02}"))));
        }
    }

    #[test]
    fn custom_prefix_writes_codex_file_and_cleanup_is_isolated() {
        let dir = tmp_dir();
        let codex = SessionLog::with_prefix(dir.path().to_path_buf(), "codex-");
        codex.write_at(local(2026, 9, 9, 12, 0, 0), "warn", Some("t"), "warning", &[]);
        let session = SessionLog::new(dir.path().to_path_buf());
        session.write_at(local(2026, 9, 9, 12, 0, 0), "info", Some("t"), "ev", &[]);
        let names: Vec<String> = read_files(dir.path())
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert!(names.iter().any(|n| n == "codex-2026-09-09.log"));
        assert!(names.iter().any(|n| n == "session-2026-09-09.log"));

        // codex 前缀清理不删除 session 文件，反之亦然
        let _ = open_file(dir.path(), "2026-09-02", "codex-").unwrap();
        let _ = open_file(dir.path(), "2026-09-02", DEFAULT_PREFIX).unwrap();
        cleanup_impl(dir.path(), "2026-09-09", "codex-");
        let names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(!names.iter().any(|n| n == "codex-2026-09-02.log"));
        assert!(names.iter().any(|n| n == "session-2026-09-02.log"));
    }

    #[test]
    fn summarize_keeps_safe_fields_and_strips_content() {
        let params = serde_json::json!({
            "threadId": "t1",
            "turnId": "tu1",
            "status": "running",
            "input": [{"text": "secret-prompt"}, {"filePath": "C:/x.txt"}],
            "approvalPolicy": "never",
            "sandbox": "read-only",
            "toolResult": "secret-output",
        });
        let kv = SessionLog::summarize_params("turn/start", &params);
        let joined: String = kv.iter().flat_map(|(_, v)| v.chars()).collect();
        assert!(!joined.contains("secret"));
        let map: std::collections::HashMap<_, _> = kv.into_iter().collect();
        assert_eq!(map.get("threadId").map(String::as_str), Some("t1"));
        assert_eq!(map.get("inputCount").map(String::as_str), Some("2"));
        assert_eq!(map.get("approvalPolicy").map(String::as_str), Some("never"));
    }

    #[test]
    fn summarize_extracts_nested_turn_and_item_ids() {
        let params = serde_json::json!({
            "threadId": "t1",
            "turn": { "id": "tu1", "status": "completed" },
            "item": { "id": "i1", "type": "agentMessage", "status": "completed" },
        });
        let kv = SessionLog::summarize_params("turn/completed", &params);
        let map: std::collections::HashMap<_, _> = kv.into_iter().collect();
        assert_eq!(map.get("turnId").map(String::as_str), Some("tu1"));
        assert_eq!(map.get("turnStatus").map(String::as_str), Some("completed"));
        assert_eq!(map.get("itemId").map(String::as_str), Some("i1"));
        assert_eq!(map.get("itemType").map(String::as_str), Some("agentMessage"));
    }

    #[test]
    fn write_failure_is_silent() {
        let dir = tmp_dir();
        // 目标“目录”是普通文件：create_dir_all/open 都会失败，写入必须静默
        let blocker = dir.path().join("blocked");
        fs::write(&blocker, b"x").unwrap();
        let log = SessionLog::new(blocker);
        log.write("info", None, "ev", &[]);
        // 正常目录仍可写入
        let log2 = SessionLog::new(dir.path().to_path_buf());
        log2.write("info", None, "ev", &[]);
        let session_files = read_files(dir.path())
            .into_iter()
            .filter(|(n, _)| n.starts_with("session-"))
            .count();
        assert_eq!(session_files, 1);
    }
}
