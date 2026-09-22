//! 兼容代理内容诊断日志：把每次上游调用的**发往上游请求体**、**上游响应原文**与
//! **发回 codex 的收尾事件**落到 `<app_data_dir>/logs/compat/`，用于事后分析
//! 「回合提前结束（任务未完成）」这类偶发问题。
//!
//! 与 `session_log` 的「只记安全字段、不落盘提示词/文件内容/工具输出」约定不同，
//! 本模块**有意**落盘完整对话内容（提示词、工具参数与结果、代码片段），因此：
//!
//! - 单独目录 `logs/compat`、单独上限：单文件 8MB、目录 256MB、保留 3 天；
//! - `Authorization` 只记 `present|absent`，任何情况下不落盘 API Key；
//! - 所有写入失败静默，绝不影响代理主流程。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use chrono::Local;

/// 单个日志文件上限：超出后停止写入并追加一行截断标记（允许跨界的那一次写入略超）。
pub(crate) const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// 目录总量上限：超出时按文件名时间戳从最旧的「调用组」整组删除。
pub(crate) const MAX_DIR_BYTES: u64 = 256 * 1024 * 1024;
/// 日志保留天数（与文件名时间戳比较）。
pub(crate) const KEEP_DAYS: i64 = 3;
/// 两次目录清理之间的最小间隔（避免每个请求都扫描目录）。
const CLEANUP_INTERVAL_MS: u64 = 60_000;
const MS_PER_DAY: i64 = 86_400_000;
/// 单文件超限时写入的标记行。
const TRUNCATED_MARK: &str = "[truncated: 超过单文件上限，后续内容未写入]";
/// 一次调用的文件后缀集合：用于从文件名还原「调用组」，实现整组删除。
const ARTIFACT_SUFFIXES: [&str; 5] = [
    ".request.json",
    ".response.sse",
    ".response.json",
    ".response.txt",
    ".summary.txt",
];

/// 日志文件的体积/保留上限（集中定义，测试可注入小值）。
#[derive(Debug, Clone, Copy)]
pub(crate) struct TraceLimits {
    pub max_file_bytes: u64,
    pub max_dir_bytes: u64,
    pub keep_days: i64,
    pub cleanup_interval_ms: u64,
}

impl Default for TraceLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: MAX_FILE_BYTES,
            max_dir_bytes: MAX_DIR_BYTES,
            keep_days: KEEP_DAYS,
            cleanup_interval_ms: CLEANUP_INTERVAL_MS,
        }
    }
}

/// 内容日志的写入器；`Arc` 由代理状态共享。
pub(crate) struct CompatTrace {
    dir: PathBuf,
    limits: TraceLimits,
    /// 上次清理时间（epoch 毫秒），0 表示尚未清理过。
    last_cleanup_ms: AtomicU64,
}

impl CompatTrace {
    /// 使用默认上限创建（目录在首次写入前按需创建）。
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self::with_limits(dir, TraceLimits::default())
    }

    pub(crate) fn with_limits(dir: PathBuf, limits: TraceLimits) -> Self {
        Self {
            dir,
            limits,
            last_cleanup_ms: AtomicU64::new(0),
        }
    }

    /// 开始记录一次上游调用（`attempt` 从 1 开始）；顺带按节流策略清理旧文件。
    pub(crate) fn begin(&self, call_id: &str, attempt: usize) -> TraceCall {
        self.begin_at(call_id, attempt, now_ms())
    }

    /// 内部实现：时间由调用方注入，便于测试文件名与清理判定。
    pub(crate) fn begin_at(&self, call_id: &str, attempt: usize, now_ms: i64) -> TraceCall {
        self.cleanup_if_due(now_ms);
        let stamp = Local::now().format("%Y%m%d-%H%M%S%3f").to_string();
        let base_name = format!("{stamp}-{call_id}-a{attempt}");
        let _ = fs::create_dir_all(&self.dir);
        TraceCall {
            inner: Some(CallInner {
                dir: self.dir.clone(),
                limits: self.limits,
                base_name,
                call_id: call_id.to_string(),
                attempt,
                started: Instant::now(),
                fields: Vec::new(),
                suffixes: Vec::new(),
                open: None,
                truncated: false,
                finished: false,
            }),
        }
    }

    /// 按节流策略清理：间隔未到直接返回，间隔到了记录本次时间再清理。
    fn cleanup_if_due(&self, now_ms: i64) {
        let now = now_ms.max(0) as u64;
        let last = self.last_cleanup_ms.load(Ordering::Relaxed);
        if last != 0 && now.saturating_sub(last) < self.limits.cleanup_interval_ms {
            return;
        }
        self.last_cleanup_ms.store(now, Ordering::Relaxed);
        cleanup_impl(&self.dir, self.limits, now_ms);
    }

    /// 立即清理一次（仅供测试：正常路径按节流策略在 `begin` 时清理）。
    #[cfg(test)]
    pub(crate) fn cleanup_now(&self) {
        cleanup_impl(&self.dir, self.limits, now_ms());
    }
}

/// 代理侧持有的写入句柄；`disabled()` 用于未启用/无日志目录的场景（全部空操作）。
#[derive(Clone)]
pub(crate) struct TraceSink(Option<Arc<CompatTrace>>);

impl TraceSink {
    pub(crate) fn new(trace: Arc<CompatTrace>) -> Self {
        Self(Some(trace))
    }

    pub(crate) fn disabled() -> Self {
        Self(None)
    }

    /// 开始记录一次上游尝试；未启用时返回空句柄。
    pub(crate) fn begin(&self, call_id: &str, attempt: usize) -> TraceCall {
        match self.0.as_ref() {
            Some(trace) => trace.begin(call_id, attempt),
            None => TraceCall::disabled(),
        }
    }
}

/// 一次上游尝试的记录句柄：写请求体、上游响应原文与收尾摘要。
pub(crate) struct TraceCall {
    inner: Option<CallInner>,
}

struct CallInner {
    dir: PathBuf,
    limits: TraceLimits,
    /// 文件基名：`<yyyyMMdd-HHmmssSSS>-<call_id>-a<attempt>`。
    base_name: String,
    call_id: String,
    attempt: usize,
    started: Instant,
    /// 摘要字段（按写入顺序输出）。
    fields: Vec<(String, String)>,
    /// 已产生的文件后缀（写入 summary 的 `files=`）。
    suffixes: Vec<&'static str>,
    /// 当前打开的响应文件（流式时按行追加）。
    open: Option<TraceFile>,
    truncated: bool,
    finished: bool,
}

impl TraceCall {
    /// 未启用内容日志时的空句柄：所有方法空操作。
    pub(crate) fn disabled() -> Self {
        Self { inner: None }
    }

    /// 追加/覆盖一条摘要字段。
    pub(crate) fn note(&mut self, key: &str, value: impl Into<String>) {
        let Some(inner) = self.inner.as_mut() else {
            return;
        };
        let value = value.into();
        match inner.fields.iter_mut().find(|(k, _)| k == key) {
            Some((_, slot)) => *slot = value,
            None => inner.fields.push((key.to_string(), value)),
        }
    }

    /// 写 `<base>.request.json`（发往上游的 chat/completions 请求体，缩进 JSON）。
    pub(crate) fn write_request_json(&mut self, body: &serde_json::Value) {
        let text = serde_json::to_string_pretty(body).unwrap_or_else(|_| body.to_string());
        self.write_file("request.json", &text);
    }

    /// 追加一行上游响应原文（流式：保留 `data:` 前缀、`[DONE]` 与心跳行）。
    pub(crate) fn write_response_line(&mut self, line: &str) {
        let Some(inner) = self.inner.as_mut() else {
            return;
        };
        if inner.open.is_none() {
            inner.open = TraceFile::create(&inner.dir, &inner.base_name, "response.sse");
            if inner.open.is_some() {
                inner.suffixes.push("response.sse");
            }
        }
        let limit = inner.limits.max_file_bytes;
        let Some(file) = inner.open.as_mut() else {
            return;
        };
        let mut text = String::with_capacity(line.len() + 1);
        text.push_str(line);
        text.push('\n');
        if !file.append(&text, limit) {
            inner.truncated = true;
        }
    }

    /// 写一整份上游响应（非流式 `response.json` / 非 2xx 原文 `response.txt`）。
    pub(crate) fn write_response_text(&mut self, suffix: &'static str, text: &str) {
        self.write_file(suffix, text);
    }

    /// 写一个整体文件（请求体 / 整份响应 / 摘要）。
    fn write_file(&mut self, suffix: &'static str, text: &str) {
        let Some(inner) = self.inner.as_mut() else {
            return;
        };
        let limit = inner.limits.max_file_bytes;
        let Some(mut file) = TraceFile::create(&inner.dir, &inner.base_name, suffix) else {
            return;
        };
        inner.suffixes.push(suffix);
        let mut body = text.to_string();
        if !body.ends_with('\n') {
            body.push('\n');
        }
        if !file.append(&body, limit) {
            inner.truncated = true;
        }
    }

    /// 写 `<base>.summary.txt` 并结束本次记录（重复调用只生效一次）。
    pub(crate) fn finish(&mut self) {
        let Some(inner) = self.inner.as_mut() else {
            return;
        };
        if inner.finished {
            return;
        }
        inner.finished = true;
        let elapsed = inner.started.elapsed().as_millis().to_string();
        match inner.fields.iter_mut().find(|(k, _)| k == "elapsed_ms") {
            Some((_, slot)) => *slot = elapsed,
            None => inner.fields.push(("elapsed_ms".to_string(), elapsed)),
        }
        let mut text = String::new();
        text.push_str("# zen proxy trace\n");
        text.push_str(&format!("call_id={}\n", inner.call_id));
        text.push_str(&format!("attempt={}\n", inner.attempt));
        for (key, value) in inner.fields.iter() {
            text.push_str(&format!("{key}={}\n", sanitize(value)));
        }
        let files = {
            let mut files = inner.suffixes.clone();
            files.push("summary.txt");
            files.join(",")
        };
        text.push_str(&format!("files={files}\n"));
        text.push_str(&format!("truncated={}\n", inner.truncated));
        let limit = inner.limits.max_file_bytes;
        let base = inner.base_name.clone();
        let dir = inner.dir.clone();
        let Some(mut file) = TraceFile::create(&dir, &base, "summary.txt") else {
            return;
        };
        let _ = file.append(&text, limit);
    }
}

impl Drop for TraceCall {
    /// 客户端中途断开等未显式收尾的场景：留下已收集到的摘要，便于确认「回合被中断」。
    fn drop(&mut self) {
        let pending = self
            .inner
            .as_ref()
            .is_some_and(|inner| !inner.finished && !inner.fields.is_empty());
        if pending {
            self.note("ended", "dropped_without_finish");
            self.finish();
        }
    }
}

/// 单个日志文件：带体积上限的顺序追加。
struct TraceFile {
    file: File,
    bytes: u64,
    truncated: bool,
}

impl TraceFile {
    fn create(dir: &Path, base_name: &str, suffix: &str) -> Option<Self> {
        let path = dir.join(format!("{base_name}.{suffix}"));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()?;
        Some(Self {
            file,
            bytes: 0,
            truncated: false,
        })
    }

    /// 追加内容；已到上限时只写一次截断标记并返回 false（表示内容被截断）。
    fn append(&mut self, text: &str, limit: u64) -> bool {
        if self.truncated {
            return false;
        }
        if self.bytes >= limit {
            self.truncated = true;
            let _ = self
                .file
                .write_all(format!("\n{TRUNCATED_MARK}\n").as_bytes());
            return false;
        }
        if self.file.write_all(text.as_bytes()).is_err() {
            return false;
        }
        self.bytes += text.len() as u64;
        true
    }
}

/// 目录清理：先删超过保留天数的调用组，再按总量上限删最旧的组。
fn cleanup_impl(dir: &Path, limits: TraceLimits, now_ms: i64) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(String, u64)> = Vec::new();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        files.push((entry.file_name().to_string_lossy().into_owned(), meta.len()));
    }
    for name in cleanup_targets(&files, now_ms, limits) {
        let _ = fs::remove_file(dir.join(name));
    }
}

/// 计算应删除的文件名（纯函数，便于单测）：
/// ① 文件名时间戳早于 `now - keep_days` 的调用组整组删除；
/// ② 剩余总量超过 `max_dir_bytes` 时，按时间戳从最旧的调用组开始删；
/// ③ 文件名无法解析的一律不删。
fn cleanup_targets(files: &[(String, u64)], now_ms: i64, limits: TraceLimits) -> Vec<String> {
    // 组键 = 文件名去掉后缀；同一次尝试的文件一起删。
    let mut groups: Vec<(String, i64, u64)> = Vec::new();
    for (name, size) in files {
        let Some((group, ts)) = parse_artifact(name) else {
            continue;
        };
        match groups.iter_mut().find(|(existing, _, _)| *existing == group) {
            Some((_, _, total)) => *total += *size,
            None => groups.push((group, ts, *size)),
        }
    }
    let deadline = now_ms - limits.keep_days.max(0) * MS_PER_DAY;
    let mut dead: std::collections::HashSet<String> = groups
        .iter()
        .filter(|(_, ts, _)| *ts < deadline)
        .map(|(key, _, _)| key.clone())
        .collect();
    let mut alive: Vec<&(String, i64, u64)> = groups
        .iter()
        .filter(|(key, _, _)| !dead.contains(key))
        .collect();
    alive.sort_by_key(|(_, ts, _)| *ts);
    let mut total: u64 = alive.iter().map(|(_, _, size)| *size).sum();
    for (key, _, size) in alive {
        if total <= limits.max_dir_bytes {
            break;
        }
        total = total.saturating_sub(*size);
        dead.insert(key.clone());
    }

    let mut out: Vec<String> = Vec::new();
    for (name, _) in files {
        if let Some((group, _)) = parse_artifact(name) {
            if dead.contains(&group) {
                out.push(name.clone());
            }
        }
    }
    out
}

/// 解析 trace 文件名：`<yyyyMMdd>-<HHmmssSSS>-<call_id>-a<attempt>.<suffix>`。
/// 返回（组键 = 去掉后缀的基名, 时间戳毫秒）；格式不符时返回 None（调用方保守保留）。
fn parse_artifact(name: &str) -> Option<(String, i64)> {
    let group = ARTIFACT_SUFFIXES
        .iter()
        .find_map(|suffix| name.strip_suffix(suffix))?;
    let (date, rest) = group.split_once('-')?;
    let (time, tail) = rest.split_once('-')?;
    if date.len() != 8 || time.len() != 9 || tail.is_empty() {
        return None;
    }
    let day = chrono::NaiveDate::parse_from_str(date, "%Y%m%d").ok()?;
    let hour: u32 = time[0..2].parse().ok()?;
    let minute: u32 = time[2..4].parse().ok()?;
    let second: u32 = time[4..6].parse().ok()?;
    let milli: u32 = time[6..9].parse().ok()?;
    let naive = day.and_hms_milli_opt(hour, minute, second, milli)?;
    // 文件名里的时间是本地时间戳，按本地时区换算回 epoch 毫秒。
    let local = naive.and_local_timezone(Local).single()?;
    Some((group.to_string(), local.timestamp_millis()))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 摘要字段值里的换行转义为字面量，保证一行一条记录。
fn sanitize(value: &str) -> String {
    value.replace('\r', "\\r").replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;
    use tempfile::TempDir;

    fn tmp_dir() -> TempDir {
        TempDir::new().unwrap()
    }

    fn read_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// 构造指定时间的文件名（本地时区），供清理判定用例使用。
    fn stamped(now_ms: i64, call: &str, suffix: &str) -> String {
        let local = Local.timestamp_millis_opt(now_ms).unwrap();
        format!("{}-{call}-a1.{suffix}", local.format("%Y%m%d-%H%M%S%3f"))
    }

    #[test]
    fn writes_request_response_and_summary_files() {
        let dir = tmp_dir();
        let trace = CompatTrace::new(dir.path().to_path_buf());
        let mut call = trace.begin("req_abc", 1);
        call.note("model", "mimo-v2.5-flash");
        call.note("upstream_status", "200");
        call.write_request_json(&json!({ "model": "m", "messages": [{ "role": "user" }] }));
        call.write_response_line("data: {\"choices\":[]}");
        call.write_response_line("data: [DONE]");
        call.note("suspicious", "-");
        call.finish();

        let names = read_names(dir.path());
        assert_eq!(names.len(), 3, "{names:?}");
        let base = names
            .iter()
            .find_map(|name| name.strip_suffix(".request.json"))
            .expect("应写 request.json")
            .to_string();
        assert!(base.contains("req_abc-a1"), "{base}");
        assert!(names.iter().any(|n| n.ends_with(".request.json")));
        assert!(names.iter().any(|n| n.ends_with(".response.sse")));
        assert!(names.iter().any(|n| n.ends_with(".summary.txt")));

        let request = fs::read_to_string(dir.path().join(format!("{base}.request.json"))).unwrap();
        assert!(request.contains("\"model\": \"m\""));
        assert!(request.contains("\"role\": \"user\""));

        let response = fs::read_to_string(dir.path().join(format!("{base}.response.sse"))).unwrap();
        assert_eq!(
            response,
            "data: {\"choices\":[]}\ndata: [DONE]\n",
            "上游行应原样落盘"
        );

        let summary = fs::read_to_string(dir.path().join(format!("{base}.summary.txt"))).unwrap();
        assert!(summary.contains("call_id=req_abc"));
        assert!(summary.contains("attempt=1"));
        assert!(summary.contains("model=mimo-v2.5-flash"));
        assert!(summary.contains("suspicious=-"));
        assert!(
            summary.contains("files=request.json,response.sse,summary.txt"),
            "{summary}"
        );
        assert!(summary.contains("truncated=false"));
    }

    #[test]
    fn write_response_text_writes_whole_file() {
        let dir = tmp_dir();
        let trace = CompatTrace::new(dir.path().to_path_buf());
        let mut call = trace.begin_at("req_txt", 2, 1_770_000_000_000);
        call.write_response_text("response.txt", "{\"error\":\"boom\"}");
        call.finish();
        let names = read_names(dir.path());
        let text_file = names
            .iter()
            .find(|n| n.ends_with(".response.txt"))
            .expect("应写 response.txt");
        assert!(text_file.contains("req_txt-a2"), "{text_file}");
        assert_eq!(
            fs::read_to_string(dir.path().join(text_file)).unwrap(),
            "{\"error\":\"boom\"}\n"
        );
    }

    #[test]
    fn file_is_truncated_at_limit_with_marker() {
        let dir = tmp_dir();
        let limits = TraceLimits {
            max_file_bytes: 32,
            ..TraceLimits::default()
        };
        let trace = CompatTrace::with_limits(dir.path().to_path_buf(), limits);
        let mut call = trace.begin("req_big", 1);
        for _ in 0..10 {
            call.write_response_line("data: 0123456789");
        }
        call.finish();

        let names = read_names(dir.path());
        let response = names
            .iter()
            .find(|n| n.ends_with(".response.sse"))
            .expect("应写 response.sse");
        let content = fs::read_to_string(dir.path().join(response)).unwrap();
        assert!(content.contains(TRUNCATED_MARK), "{content}");
        assert!(content.len() < 256, "截断后不应继续增长：{content}");

        let summary_name = names
            .iter()
            .find(|n| n.ends_with(".summary.txt"))
            .unwrap();
        let summary = fs::read_to_string(dir.path().join(summary_name)).unwrap();
        assert!(summary.contains("truncated=true"), "{summary}");
    }

    #[test]
    fn cleanup_targets_removes_expired_and_oversize_groups() {
        let limits = TraceLimits {
            max_file_bytes: 1024,
            max_dir_bytes: 300,
            keep_days: 3,
            cleanup_interval_ms: 0,
        };
        let now = Local
            .with_ymd_and_hms(2026, 9, 14, 12, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let day = MS_PER_DAY;
        let files = vec![
            // 过期组（4 天前）整组删除
            (stamped(now - 4 * day, "old", "request.json"), 100u64),
            (stamped(now - 4 * day, "old", "summary.txt"), 50),
            // 2 天前：未过期，但总量超上限时整组删除
            (stamped(now - 2 * day, "mid", "request.json"), 200),
            (stamped(now - 2 * day, "mid", "summary.txt"), 100),
            // 1 天前：放得下时保留
            (stamped(now - day, "new", "request.json"), 100),
            (stamped(now - day, "new", "summary.txt"), 50),
            // 不可解析的名字一律不删
            ("not-a-trace.log".to_string(), 999),
        ];
        let targets = cleanup_targets(&files, now, limits);
        assert!(targets.contains(&stamped(now - 4 * day, "old", "request.json")));
        assert!(targets.contains(&stamped(now - 4 * day, "old", "summary.txt")));
        assert!(targets.contains(&stamped(now - 2 * day, "mid", "request.json")));
        assert!(targets.contains(&stamped(now - 2 * day, "mid", "summary.txt")));
        assert!(!targets.contains(&stamped(now - day, "new", "request.json")));
        assert!(!targets.contains(&"not-a-trace.log".to_string()));
    }

    #[test]
    fn cleanup_targets_keeps_everything_within_limits() {
        let limits = TraceLimits {
            max_file_bytes: 1024,
            max_dir_bytes: 10_000,
            keep_days: 3,
            cleanup_interval_ms: 0,
        };
        let now = Local
            .with_ymd_and_hms(2026, 9, 14, 12, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let files = vec![
            (stamped(now, "keep", "request.json"), 100u64),
            ("not-a-trace.log".to_string(), 999),
        ];
        assert!(cleanup_targets(&files, now, limits).is_empty());
    }

    #[test]
    fn cleanup_impl_deletes_expired_files_and_ignores_unparsable() {
        let dir = tmp_dir();
        let limits = TraceLimits {
            max_file_bytes: 1024,
            max_dir_bytes: 10_000,
            keep_days: 3,
            cleanup_interval_ms: 0,
        };
        let trace = CompatTrace::with_limits(dir.path().to_path_buf(), limits);
        let old = stamped(now_ms() - 5 * MS_PER_DAY, "gone", "request.json");
        fs::write(dir.path().join(old), "x").unwrap();
        fs::write(dir.path().join("keep-me.log"), "x").unwrap();
        trace.cleanup_now();
        assert_eq!(read_names(dir.path()), vec!["keep-me.log".to_string()]);
    }

    #[test]
    fn disabled_call_is_noop() {
        let dir = tmp_dir();
        let sink = TraceSink::disabled();
        let mut call = sink.begin("req_none", 1);
        call.note("model", "m");
        call.write_request_json(&json!({ "model": "m" }));
        call.write_response_line("data: [DONE]");
        call.write_response_text("response.json", "{}");
        call.finish();
        drop(call);
        assert!(read_names(dir.path()).is_empty());
    }

    #[test]
    fn write_failure_is_silent() {
        let dir = tmp_dir();
        // 目标“目录”是普通文件：create_dir_all 与 open 都会失败，写入必须静默
        let blocker = dir.path().join("blocked");
        fs::write(&blocker, b"x").unwrap();
        let trace = CompatTrace::new(blocker);
        let mut call = trace.begin("req_x", 1);
        call.write_request_json(&json!({ "model": "m" }));
        call.finish();
    }

    #[test]
    fn drop_writes_summary_for_interrupted_call() {
        let dir = tmp_dir();
        let trace = CompatTrace::new(dir.path().to_path_buf());
        {
            let mut call = trace.begin("req_drop", 1);
            call.note("model", "m");
            call.write_response_line("data: partial");
        }
        let names = read_names(dir.path());
        let summary_name = names
            .iter()
            .find(|n| n.ends_with(".summary.txt"))
            .expect("中断也应落盘摘要");
        let summary = fs::read_to_string(dir.path().join(summary_name)).unwrap();
        assert!(summary.contains("ended=dropped_without_finish"), "{summary}");
    }
}
