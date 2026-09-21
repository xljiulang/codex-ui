//! `codexui-kb` 子进程调用层：可执行文件定位、NDJSON 读取、错误映射。
//!
//! 知识库的全部重活（PDF/docx 抽取、切块、ONNX 向量化、SQLite 混合检索）都在
//! `codexui-kb.exe` 里完成；这里只负责起进程、读协议、把错误翻译成中文。
//!
//! 协议见 `crates/knowledge-cli/src/cli.rs`：stdout 为 NDJSON（progress / result /
//! error），退出码 0 成功、2 参数错误、3 模型未就绪、4 库被占用、5 已取消。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

/// 默认返回条数（与子进程保持一致）
pub const DEFAULT_TOP_K: usize = 8;
/// 子进程可执行文件名
const BIN_NAME: &str = "codexui-kb.exe";
/// 期望的 CLI 协议版本
const API_VERSION: u64 = 1;
/// 隐藏子进程控制台窗口（GUI 应用里起控制台程序会闪黑框）
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// stderr 诊断信息保留的最大字符数
const STDERR_TAIL_LIMIT: usize = 800;

/// 首次版本校验通过后缓存（失败不缓存，便于补齐/重装后重试）
static READY: OnceLock<()> = OnceLock::new();

/// 检索命中的切块（与子进程 `ChunkHit` 字段一一对应，字段名即前端契约）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkHit {
    pub doc_path: String,
    pub title_path: String,
    pub text: String,
    pub score: f32,
}

/// 知识库列表行（与子进程 `KbSummary` 字段一一对应）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KbSummary {
    /// 库文件名（删除时回传该值）
    pub file: String,
    pub cwd: String,
    pub docs: i64,
    pub chunks: i64,
    pub updated_at: i64,
    /// 是否可正常使用（损坏/缺元数据为 false，仍允许删除）
    pub available: bool,
    pub error: Option<String>,
}

/// 建库进度载荷（事件 `knowledge/index-progress` 的 payload）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexProgress {
    pub cwd: String,
    pub phase: String,
    pub processed: usize,
    pub total: usize,
    pub current: String,
}

/// 建库结果摘要（同时作为 `index_docs` 工具返回值与完成事件载荷）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexSummary {
    pub cwd: String,
    pub roots: Vec<String>,
    pub total: usize,
    pub added: i64,
    pub updated: i64,
    pub skipped: i64,
    pub removed: i64,
    /// 失败明细（`路径: 原因`）
    pub failed: Vec<String>,
    pub docs: i64,
    pub chunks: i64,
    pub elapsed_ms: u64,
    /// 是否已转后台执行（当次调用只返回启动信息）
    pub background: bool,
    pub cancelled: bool,
    /// 人类可读摘要（由子进程生成，codex-ui 直接透传给 agent）
    pub text: String,
}

impl IndexSummary {
    /// 后台启动占位：文件数过多或同步等待超时时由 codex-ui 生成（子进程仍在跑）
    pub fn background_started(cwd: &str, roots: Vec<String>, total: usize) -> Self {
        let mut summary = IndexSummary {
            cwd: cwd.to_string(),
            roots: roots.clone(),
            total,
            background: true,
            ..Default::default()
        };
        summary.text = format!(
            "已在后台开始建库：工作目录 {}，来源 {}，共 {} 个文件。完成后设置页「知识库」列表会更新；\
             再次调用 index_docs 可查看当前统计。",
            cwd,
            roots.join("、"),
            total
        );
        summary
    }
}

/// 进行中的建库子进程句柄：取消 = 终止子进程
#[derive(Default)]
pub struct JobHandle {
    child: tokio::sync::Mutex<Option<Child>>,
    cancelled: AtomicBool,
}

impl JobHandle {
    /// 取消：置位标志并终止子进程（SQLite WAL 保证中断后库仍可用）
    pub async fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        let mut guard = self.child.lock().await;
        if let Some(child) = guard.as_mut() {
            let _ = child.start_kill();
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// 定位 `codexui-kb.exe`：显式环境变量 → `<exe 目录>/` → `<exe 目录>/bin/`
pub fn binary_path() -> Result<PathBuf, String> {
    if let Some(raw) = std::env::var_os("CODEXUI_KB_BIN") {
        let path = PathBuf::from(raw);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "CODEXUI_KB_BIN 指向的文件不存在：{}",
            path.display()
        ));
    }
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .ok_or_else(|| "无法定位应用目录".to_string())?;
    for candidate in [exe_dir.join(BIN_NAME), exe_dir.join("bin").join(BIN_NAME)] {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "未找到 {BIN_NAME}（发布版应随安装包放在应用目录的 bin/ 下；\
         开发环境请先运行 cargo build -p knowledge-cli，或用 CODEXUI_KB_BIN 指定）"
    ))
}

/// 首次调用时校验子进程协议版本（成功即缓存；失败每次重试）
async fn ensure_ready() -> Result<(), String> {
    if READY.get().is_some() {
        return Ok(());
    }
    let value = run_once(vec!["version".to_string()]).await?;
    let api = value.get("codexUiApi").and_then(Value::as_u64).unwrap_or(0);
    if api != API_VERSION {
        return Err(format!(
            "codexui-kb 版本不匹配：CLI 协议 v{api}，当前 codex-ui 需要 v{API_VERSION}，请更新或重新构建知识库 CLI"
        ));
    }
    let _ = READY.set(());
    Ok(())
}

/// 知识库列表（不加载向量模型）
pub async fn list(data_dir: &Path) -> Result<Vec<KbSummary>, String> {
    ensure_ready().await?;
    let value = run_once(vec![
        "list".to_string(),
        "--data-dir".to_string(),
        path_arg(data_dir),
    ])
    .await?;
    let rows = value.get("kbs").cloned().unwrap_or_else(|| Value::Array(vec![]));
    serde_json::from_value(rows).map_err(|e| format!("解析知识库列表失败：{e}"))
}

/// 删除指定知识库文件（只删索引，不动原始文档与模型）
pub async fn delete(data_dir: &Path, file: &str) -> Result<(), String> {
    ensure_ready().await?;
    run_once(vec![
        "delete".to_string(),
        "--data-dir".to_string(),
        path_arg(data_dir),
        "--file".to_string(),
        file.to_string(),
    ])
    .await?;
    Ok(())
}

/// 检索当前会话工作目录对应的知识库（未建库返回空数组）
pub async fn search(
    data_dir: &Path,
    cwd: &str,
    query: &str,
    top_k: usize,
) -> Result<Vec<ChunkHit>, String> {
    ensure_ready().await?;
    let value = run_once(vec![
        "search".to_string(),
        "--data-dir".to_string(),
        path_arg(data_dir),
        "--cwd".to_string(),
        cwd.to_string(),
        "--query".to_string(),
        query.to_string(),
        "--top-k".to_string(),
        top_k.to_string(),
    ])
    .await?;
    let hits = value.get("hits").cloned().unwrap_or_else(|| Value::Array(vec![]));
    serde_json::from_value(hits).map_err(|e| format!("解析检索结果失败：{e}"))
}

/// 建库/增量更新：边读 NDJSON 边回调进度，返回最终摘要
pub async fn run_index(
    data_dir: &Path,
    cwd: &str,
    roots: &[String],
    full: bool,
    job: &Arc<JobHandle>,
    on_progress: &mut (dyn FnMut(IndexProgress) + Send),
) -> Result<IndexSummary, String> {
    ensure_ready().await?;
    let mut args = vec![
        "index".to_string(),
        "--data-dir".to_string(),
        path_arg(data_dir),
        "--cwd".to_string(),
        cwd.to_string(),
    ];
    for root in roots {
        args.push("--roots".to_string());
        args.push(root.clone());
    }
    if full {
        args.push("--full".to_string());
    }
    let bin = binary_path()?;
    let mut child = command(&bin, &args)
        .spawn()
        .map_err(|e| format!("无法启动 codexui-kb（{}）：{e}", bin.display()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "无法读取 codexui-kb 输出".to_string())?;
    let stderr_task = child.stderr.take().map(spawn_stderr_reader);
    {
        let mut guard = job.child.lock().await;
        *guard = Some(child);
    }

    let mut summary: Option<IndexSummary> = None;
    let mut failure: Option<String> = None;
    let mut lines = BufReader::new(stdout).lines();
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|e| format!("读取 codexui-kb 输出失败：{e}"))?
    {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("progress") => {
                if let Ok(progress) = serde_json::from_value::<IndexProgress>(value) {
                    on_progress(progress);
                }
            }
            Some("result") => {
                let payload = value.get("summary").cloned().unwrap_or(Value::Null);
                summary = serde_json::from_value(payload).ok();
            }
            Some("error") => {
                failure = Some(
                    value
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("codexui-kb 建库失败")
                        .to_string(),
                );
            }
            _ => {}
        }
    }
    let status = {
        let mut guard = job.child.lock().await;
        match guard.take() {
            Some(mut child) => child
                .wait()
                .await
                .map_err(|e| format!("等待 codexui-kb 退出失败：{e}"))?,
            None => return Err("建库子进程句柄丢失".to_string()),
        }
    };
    let stderr = match stderr_task {
        Some(task) => task.await.unwrap_or_default(),
        None => String::new(),
    };

    if job.is_cancelled() {
        if let Some(summary) = summary {
            return Ok(summary);
        }
    }
    if let Some(message) = failure {
        return Err(with_stderr(message, &stderr));
    }
    match summary {
        Some(summary) => Ok(summary),
        None => Err(format!(
            "codexui-kb 未返回建库结果（退出码 {}）{}",
            status.code().unwrap_or(-1),
            stderr_tail(&stderr)
        )),
    }
}

/// 收集一次性命令（list/search/delete/version）的结果 JSON
async fn run_once(args: Vec<String>) -> Result<Value, String> {
    let bin = binary_path()?;
    let output = command(&bin, &args)
        .spawn()
        .map_err(|e| format!("无法启动 codexui-kb（{}）：{e}", bin.display()))?
        .wait_with_output()
        .await
        .map_err(|e| format!("codexui-kb 运行失败：{e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let mut result: Option<Value> = None;
    let mut failure: Option<String> = None;
    for line in stdout.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("result") => result = Some(value),
            Some("error") => {
                failure = Some(
                    value
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("codexui-kb 执行失败")
                        .to_string(),
                );
            }
            _ => {}
        }
    }
    if let Some(message) = failure {
        return Err(with_stderr(message, &stderr));
    }
    result.ok_or_else(|| {
        format!(
            "codexui-kb 未返回结果（退出码 {}）{}",
            output.status.code().unwrap_or(-1),
            stderr_tail(&stderr)
        )
    })
}

fn command(bin: &Path, args: &[String]) -> Command {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // 应用退出/任务被取消时顺带结束子进程，避免留下孤儿建库任务
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// 后台读取 stderr（避免管道写满阻塞子进程），返回采集到的文本
fn spawn_stderr_reader(stderr: tokio::process::ChildStderr) -> tokio::task::JoinHandle<String> {
    tokio::spawn(async move {
        let mut buf = String::new();
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            buf.push_str(&line);
            buf.push('\n');
        }
        buf
    })
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn stderr_tail(stderr: &str) -> String {
    let trimmed = stderr.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let skip = trimmed.chars().count().saturating_sub(STDERR_TAIL_LIMIT);
    let tail: String = trimmed.chars().skip(skip).collect();
    format!("\n诊断：{tail}")
}

fn with_stderr(message: String, stderr: &str) -> String {
    format!("{message}{}", stderr_tail(stderr))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 环境变量相关用例串行执行（`#[tokio::test]` 与普通测试共用同一进程）
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 定位刚构建好的 CLI（`target/debug/codexui-kb.exe`）；找不到则跳过用例
    fn dev_cli_bin() -> Option<PathBuf> {
        if let Some(raw) = std::env::var_os("CODEXUI_KB_BIN") {
            let path = PathBuf::from(raw);
            if path.is_file() {
                return Some(path);
            }
        }
        let exe = std::env::current_exe().ok()?;
        let dir = exe.parent()?.parent()?; // target/debug/deps → target/debug
        let candidate = dir.join(BIN_NAME);
        candidate.is_file().then_some(candidate)
    }

    /// 用真实 CLI 进程跑一段断言；CLI 未构建时返回 None（用例跳过）
    fn with_dev_cli<T>(f: impl FnOnce() -> T) -> Option<T> {
        let _guard = lock_env();
        let bin = dev_cli_bin()?;
        std::env::set_var("CODEXUI_KB_BIN", &bin);
        let out = f();
        std::env::remove_var("CODEXUI_KB_BIN");
        Some(out)
    }

    #[test]
    fn background_summary_describes_background_run() {
        let summary = IndexSummary::background_started("D:\\kb", vec!["D:\\kb".into()], 500);
        assert!(summary.background);
        assert_eq!(summary.total, 500);
        assert!(summary.text.contains("后台"));
        assert!(summary.text.contains("500"));
    }

    #[test]
    fn stderr_tail_is_bounded_and_labelled() {
        assert_eq!(stderr_tail("   "), "");
        let long = "错".repeat(STDERR_TAIL_LIMIT * 2);
        let tail = stderr_tail(&long);
        assert!(tail.starts_with("\n诊断："));
        assert!(tail.chars().count() <= STDERR_TAIL_LIMIT + 4);
    }

    #[test]
    fn binary_path_honours_env_override() {
        let dir = tempfile::TempDir::new().unwrap();
        let fake = dir.path().join(BIN_NAME);
        std::fs::write(&fake, b"stub").unwrap();
        // 环境变量是进程全局的：设置后立即断言并复位，避免影响其它用例
        let _guard = lock_env();
        std::env::set_var("CODEXUI_KB_BIN", &fake);
        let resolved = binary_path().unwrap();
        std::env::remove_var("CODEXUI_KB_BIN");
        assert_eq!(resolved, fake);
    }

    #[test]
    fn missing_binary_reports_actionable_error() {
        let _guard = lock_env();
        let missing = PathBuf::from("D:\\codexui\\no-such\\codexui-kb.exe");
        std::env::set_var("CODEXUI_KB_BIN", &missing);
        let err = binary_path().unwrap_err();
        std::env::remove_var("CODEXUI_KB_BIN");
        assert!(err.contains("不存在"), "{err}");
    }

    #[test]
    fn list_reads_empty_data_dir_through_cli() {
        let Some(result) = with_dev_cli(|| {
            let dir = tempfile::TempDir::new().unwrap();
            tauri::async_runtime::block_on(list(dir.path()))
        }) else {
            eprintln!("跳过：未构建 codexui-kb.exe（先 cargo build -p knowledge-cli）");
            return;
        };
        assert_eq!(result.unwrap().len(), 0);
    }

    #[test]
    fn search_without_kb_returns_empty_hits_through_cli() {
        let Some(result) = with_dev_cli(|| {
            let dir = tempfile::TempDir::new().unwrap();
            tauri::async_runtime::block_on(search(dir.path(), "D:\\没有这个工作目录", "错误码", 8))
        }) else {
            eprintln!("跳过：未构建 codexui-kb.exe（先 cargo build -p knowledge-cli）");
            return;
        };
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn delete_surfaces_cli_error_message() {
        let Some(result) = with_dev_cli(|| {
            let dir = tempfile::TempDir::new().unwrap();
            std::fs::create_dir_all(dir.path().join("kbs")).unwrap();
            tauri::async_runtime::block_on(delete(dir.path(), "../escape.sqlite"))
        }) else {
            eprintln!("跳过：未构建 codexui-kb.exe（先 cargo build -p knowledge-cli）");
            return;
        };
        let err = result.unwrap_err();
        assert!(err.contains("非法"), "{err}");
    }
}
