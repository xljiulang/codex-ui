//! 微信 ClawBot 接入桥。
//!
//! 架构：本模块负责 Node sidecar（wechat-channel 包装进程）的生命周期、
//! 「会话 ↔ 微信账号」绑定（bindings.json）、回合编排与回复路由；微信协议细节
//! 全部由 sidecar 内的 wechat-channel 库承担。两端通过 stdio 换行分隔 JSON 通信
//! （与驱动 codex app-server 的模式一致）。

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, Mutex};
use tokio::time::timeout;

use crate::codex::app_server::CodexServer;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// 排队入站消息上限；溢出立即回忙碌提示且该条丢弃。
const QUEUE_LIMIT: usize = 16;
/// 回复超长时按可见字符数切分发送。
const REPLY_CHUNK_CHARS: usize = 1800;
/// 把回合产物整理为回复文本：无文本/非正常结束给出对应中文兜底提示。
fn normalize_turn_text(text: &str, status: &str) -> String {
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    match status {
        "interrupted" | "cancelled" | "canceled" => "⚠️ 回合已中断，本轮没有产生回复文本".into(),
        "" => "⚠️ 执行失败：回合结束但未收到文本回复".into(),
        other => format!("⚠️ 执行失败：回合异常结束（{other}）"),
    }
}
/// 单个 Codex 回合最长等待时间；超时按失败回复，避免整条队列卡死。
const TURN_TIMEOUT_SECS: u64 = 600;
/// 前端事件名：状态每次变化全量推送快照。
pub const WECHAT_EVENT: &str = "wechat/event";

/// 一条已通过「谁扫谁白」门禁的入站文本消息（携带目标绑定信息）。
#[derive(Debug, Clone)]
struct InboundMessage {
    /// 消息所属微信账号（即绑定账号）。
    account_id: String,
    /// 绑定目标线程 id（消息将路由到该线程执行回合）。
    thread_id: String,
    from: String,
    text: String,
}

// ---------------------------------------------------------------------------
// 纯函数工具（单元测试覆盖）
// ---------------------------------------------------------------------------

/// 溢出判定：仅当「已有回合执行中 且 队列达到上限」时丢弃新消息。
fn queue_overflow(active_busy: bool, queued_len: usize) -> bool {
    active_busy && queued_len >= QUEUE_LIMIT
}

/// 「thread not found」判定：codex 删除线程后 turn/start 的稳定错误文案。
/// 仅识别线程不存在，避免误判 model/file 等其它 not found。
fn is_thread_not_found(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("thread not found") || e.contains("thread does not exist")
}

/// accounts 快照中的账号是否可自动启动接收：已配置且未过期（session_expired 跳过）。
fn account_is_startable(a: &Value) -> bool {
    a.get("configured").and_then(|c| c.as_bool()) == Some(true)
        && a.get("status").and_then(|s| s.as_str()) != Some("session_expired")
}

/// 绑定是否冲突：同一线程或同一微信账号已被占用（双向唯一）。
fn binding_conflict(bindings: &[Value], thread_id: &str, account_id: &str) -> bool {
    bindings.iter().any(|b| {
        b.get("threadId").and_then(|v| v.as_str()) == Some(thread_id)
            || b.get("accountId").and_then(|v| v.as_str()) == Some(account_id)
    })
}

/// 按「账号 + 本人」路由：消息所属账号命中绑定，且发送者就是绑定账号本人（谁扫谁白）。
fn find_binding_for_message<'a>(
    bindings: &'a [Value],
    account_id: &str,
    from: &str,
) -> Option<&'a Value> {
    bindings.iter().find(|b| {
        b.get("accountId").and_then(|v| v.as_str()) == Some(account_id)
            && b.get("userId")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .is_some_and(|uid| uid == from.trim())
    })
}

/// 超长文本切分：按可见字符（含换行）计数，UTF-8 安全。
fn chunk_text(text: &str, max_chars: usize) -> Vec<String> {
    if max_chars == 0 || text.chars().count() <= max_chars {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut n = 0usize;
    for ch in text.chars() {
        cur.push(ch);
        n += 1;
        if n >= max_chars {
            out.push(std::mem::take(&mut cur));
            n = 0;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// 从单行事件 JSON 提取二维码内容（event=qr）。
fn extract_qr(v: &Value) -> Option<String> {
    if v.get("event")?.as_str()? != "qr" {
        return None;
    }
    v.get("content")
        .and_then(|c| c.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 从单行事件 JSON 提取入站文本消息（event=message 且带非空 text）。
/// 返回 (accountId, from, text)；媒体等无文本消息返回 None 由调用方忽略。
fn extract_message(v: &Value) -> Option<(String, String, String)> {
    if v.get("event")?.as_str()? != "message" {
        return None;
    }
    let msg = v.get("message")?;
    let account = msg.get("accountId").and_then(|x| x.as_str())?.to_string();
    let from = msg.get("from").and_then(|x| x.as_str())?.to_string();
    let text = msg
        .get("text")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|t| !t.is_empty())?
        .to_string();
    Some((account, from, text))
}

/// turn/start 参数：完整能力 + 免审批（dangerFullAccess 沙箱策略）。
fn build_turn_params(thread_id: &str, text: &str, model: &str, client_message_id: &str) -> Value {
    json!({
        "threadId": thread_id,
        "input": [{ "type": "text", "text": text }],
        "clientUserMessageId": client_message_id,
        "approvalPolicy": "never",
        "sandboxPolicy": { "type": "dangerFullAccess" },
        "model": null,
        "effort": null,
        "collaborationMode": {
            "mode": "default",
            "settings": {
                "model": model,
                "reasoning_effort": null,
                "developer_instructions": null,
            },
        },
    })
}

/// 绑定文件路径：<root>/bindings.json。
fn bindings_path(root: &Path) -> PathBuf {
    root.join("bindings.json")
}

/// 原子写绑定表（临时文件 + 重命名，风格对齐 settings.rs）。
async fn save_bindings(root: &Path, list: &[Value]) -> Result<(), String> {
    let p = bindings_path(root);
    let tmp = p.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(list).map_err(|e| format!("序列化绑定失败: {e}"))?;
    tokio::fs::write(&tmp, body)
        .await
        .map_err(|e| format!("写入绑定失败: {e}"))?;
    if p.exists() {
        tokio::fs::remove_file(&p)
            .await
            .map_err(|e| format!("替换绑定失败: {e}"))?;
    }
    tokio::fs::rename(&tmp, &p)
        .await
        .map_err(|e| format!("落盘绑定失败: {e}"))
}

/// 读绑定表；缺失视为空表。
async fn load_bindings(root: &Path) -> Vec<Value> {
    let Ok(text) = tokio::fs::read_to_string(bindings_path(root)).await else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// sidecar 脚本定位顺序：
/// ① 环境变量 CODEX_UI_WECHAT_SIDECAR 显式指定；
/// ② 打包资源 <exe>/resources/wechat-sidecar/wechat-sidecar.mjs；
/// ③ 开发构建产物 <workspace>/sidecar-dist/wechat-sidecar.mjs；
/// ④ 开发源码直跑 <workspace>/sidecar/wechat-sidecar.mjs（ESM 邻近解析依赖）。
fn resolve_sidecar_script() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CODEX_UI_WECHAT_SIDECAR") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidates = [
        exe_dir.join("resources").join("wechat-sidecar").join("wechat-sidecar.mjs"),
        manifest.join("..").join("sidecar-dist").join("wechat-sidecar.mjs"),
        manifest.join("..").join("sidecar").join("wechat-sidecar.mjs"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

// ---------------------------------------------------------------------------
// 桥本体状态
// ---------------------------------------------------------------------------

struct BridgeInner {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    /// sidecar 进程是否存活。
    alive: bool,
    /// 连接状态：offline｜starting｜awaiting_qr｜connected｜session_expired｜error
    conn: &'static str,
    detail: Option<String>,
    qr_content: Option<String>,
    /// 登录中的绑定目标线程 id（模态框扫码绑定流程的待落盘状态）。
    pending_bind: Option<String>,
    /// 当前生效的「会话 ↔ 微信账号」绑定（内存镜像，变更即落盘 bindings.json）。
    bindings: Vec<Value>,
    /// 本代际 sidecar 中已下发过 start 的账号（防止重复启动接收器）。
    started_accounts: HashSet<String>,
    /// 各绑定账号的连接状态（offline/starting/connected/session_expired/error）。
    account_conn: HashMap<String, String>,
    queue: VecDeque<InboundMessage>,
    /// 同一发送者连续溢出只回一次忙碌提示的去重集合（收到其新消息时解除）。
    overflow_notified: HashSet<String>,
    /// 当前正在执行 Codex 回合的线程 id；None 表示空闲。
    active_thread: Option<String>,
    /// 各线程最近完成的 agentMessage 文本（订阅任务收集，回合完成后取走）。
    pending_reply: HashMap<String, String>,
    /// 默认模型缓存：None 未解析过；Some(..) 已解析一次（成功/失败皆缓存）。
    default_model: Option<Result<String, String>>,
    /// 后台循环（泵/订阅）是否已启动（桥生命周期内仅启动一次）。
    pumps_started: bool,
    /// 出队唤醒信号接收端（首启被泵任务取走）。
    job_rx: Option<mpsc::UnboundedReceiver<()>>,
    /// 回合完成信号接收端（首启被泵任务取走）。
    done_rx: Option<mpsc::UnboundedReceiver<(String, String)>>,
    /// sidecar 退出信号接收端（首启被监督任务取走）。
    exit_rx: Option<mpsc::UnboundedReceiver<()>>,
    /// 进程意外退出后的自动重启次数（v1 仅尝试一次）。
    restarts: u32,
    /// 已记过首次忽略日志的「账号|发送者」键（进程内去重，防日志刷屏）。
    ignored_logged: HashSet<String>,
    /// 退出监督任务是否已启动。
    supervisor_started: bool,
}

impl Default for BridgeInner {
    fn default() -> Self {
        Self {
            child: None,
            stdin: None,
            alive: false,
            conn: "offline",
            detail: None,
            qr_content: None,
            pending_bind: None,
            bindings: Vec::new(),
            started_accounts: HashSet::new(),
            account_conn: HashMap::new(),
            queue: VecDeque::new(),
            overflow_notified: HashSet::new(),
            active_thread: None,
            pending_reply: HashMap::new(),
            default_model: None,
            pumps_started: false,
            job_rx: None,
            done_rx: None,
            exit_rx: None,
            restarts: 0,
            ignored_logged: HashSet::new(),
            supervisor_started: false,
        }
    }
}

pub struct WeChatBridge {
    app: AppHandle,
    server: Arc<CodexServer>,
    /// 微信数据根目录：%APPDATA%/<identifier>/wechat/
    root: PathBuf,
    inner: Mutex<BridgeInner>,
    job_tx: mpsc::UnboundedSender<()>,
    done_tx: mpsc::UnboundedSender<(String, String)>,
    exit_tx: mpsc::UnboundedSender<()>,
    seq: AtomicU64,
}

struct ActiveTurn {
    thread_id: String,
    account_id: String,
    peer: String,
    deadline: std::time::Instant,
}

impl WeChatBridge {
    pub fn new(app: AppHandle, server: Arc<CodexServer>, app_dir: PathBuf) -> Arc<Self> {
        let root = app_dir.join("wechat");
        let (job_tx, job_rx) = mpsc::unbounded_channel();
        let (done_tx, done_rx) = mpsc::unbounded_channel();
        let (exit_tx, exit_rx) = mpsc::unbounded_channel();
        Arc::new(Self {
            app,
            server,
            root,
            inner: Mutex::new(BridgeInner {
                job_rx: Some(job_rx),
                done_rx: Some(done_rx),
                exit_rx: Some(exit_rx),
                ..Default::default()
            }),
            job_tx,
            done_tx,
            exit_tx,
            seq: AtomicU64::new(0),
        })
    }

    /// 是否存在生效绑定（内存镜像；autostart / 绑定变更时刷新）。
    async fn has_bindings(&self) -> bool {
        !self.inner.lock().await.bindings.is_empty()
    }

    /// 从磁盘装载绑定到内存（应用启动与绑定文件外部变化时调用）。
    async fn reload_bindings(self: &Arc<Self>) {
        let list = load_bindings(&self.root).await;
        self.inner.lock().await.bindings = list;
    }

    /// 取某线程的绑定（复制一份，避免跨 await 持锁）。
    async fn binding_of_thread(&self, thread_id: &str) -> Option<Value> {
        self.inner
            .lock()
            .await
            .bindings
            .iter()
            .find(|b| b.get("threadId").and_then(|v| v.as_str()) == Some(thread_id))
            .cloned()
    }

    async fn log(&self, level: &str, line: String) {
        self.server.push_log(level, format!("[wechat] {line}")).await;
    }

    async fn snapshot(&self) -> Value {
        let inner = self.inner.lock().await;
        let bindings: Vec<Value> = inner
            .bindings
            .iter()
            .map(|b| {
                let account_id = b.get("accountId").and_then(|v| v.as_str()).unwrap_or("");
                json!({
                    "threadId": b.get("threadId").and_then(|v| v.as_str()).unwrap_or(""),
                    "accountId": account_id,
                    "name": b.get("name").and_then(|v| v.as_str()),
                    "connection": inner
                        .account_conn
                        .get(account_id)
                        .map(String::as_str)
                        .unwrap_or("offline"),
                })
            })
            .collect();
        json!({
            "running": inner.alive,
            "connection": inner.conn,
            "detail": inner.detail,
            "qrContent": inner.qr_content,
            "pendingThreadId": inner.pending_bind,
            "queued": inner.queue.len(),
            "busy": inner.active_thread.is_some(),
            "bindings": bindings,
        })
    }

    /// 状态变化后全量推送快照给前端（失败静默）。
    async fn emit_state(self: &Arc<Self>) {
        let snap = self.snapshot().await;
        let _ = self.app.emit(WECHAT_EVENT, snap);
    }

    async fn set_error(self: &Arc<Self>, detail: String) {
        {
            let mut g = self.inner.lock().await;
            g.conn = "error";
            g.detail = Some(detail.clone());
        }
        self.log("warn", detail).await;
        self.emit_state().await;
    }

    /// 向 sidecar 写一行指令（串行单写者；未运行时报错）。
    async fn send_cmd(&self, cmd: Value) -> Result<(), String> {
        let mut g = self.inner.lock().await;
        let stdin = g
            .stdin
            .as_mut()
            .ok_or_else(|| "微信桥未启动".to_string())?;
        let line = serde_json::to_string(&cmd).map_err(|e| e.to_string())?;
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("写微信桥指令失败: {e}"))?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(|e| format!("写微信桥指令失败: {e}"))?;
        stdin
            .flush()
            .await
            .map_err(|e| format!("写微信桥指令失败: {e}"))?;
        Ok(())
    }

    fn next_message_id(&self) -> String {
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default();
        let n = self.seq.fetch_add(1, Ordering::Relaxed);
        format!("wechat-{ms}-{n}")
    }

    /// 常驻 sidecar 退出监督：仅在首个外部入口调用一次。绝不放进 start_service，
    /// 否则 reader→exit→start_service 构成 future 类型自环，编译期无法证明 Send。
    async fn ensure_supervisor(self: &Arc<Self>) {
        let first = {
            let mut g = self.inner.lock().await;
            let first = !g.supervisor_started;
            g.supervisor_started = true;
            (first, g.exit_rx.take())
        };
        if let (true, Some(exit_rx)) = first {
            tauri::async_runtime::spawn(Self::supervise_exits(self.clone(), exit_rx));
        }
    }

    // -- 生命周期 -----------------------------------------------------------

    /// 应用启动自动恢复：存在绑定则拉起 sidecar，accounts 快照到达后逐个恢复接收。
    pub async fn autostart(self: &Arc<Self>) {
        self.ensure_supervisor().await;
        self.reload_bindings().await;
        if !self.has_bindings().await {
            return;
        }
        if let Err(e) = self.start_service().await {
            self.set_error(format!("微信接入自动启动失败: {e}")).await;
        }
    }

    /// 拉起 sidecar 进程与后台循环（幂等：已存活直接返回 Ok）。
    pub async fn start_service(self: &Arc<Self>) -> Result<(), String> {
        if self.inner.lock().await.alive {
            return Ok(());
        }
        let script = resolve_sidecar_script()
            .ok_or_else(|| "未找到 wechat-sidecar 脚本（请先执行 npm run build:sidecar）".to_string())?;
        let mut cmd =
            Command::new(std::env::var("CODEX_UI_NODE").unwrap_or_else(|_| "node".into()));
        cmd.arg(&script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| {
            format!("启动 Node 微信桥失败（确认本机已安装 node 且在 PATH 中）: {e}")
        })?;
        let stdin = child.stdin.take().ok_or("无法获取微信桥 stdin")?;
        let stdout = child.stdout.take().ok_or("无法获取微信桥 stdout")?;
        let stderr = child.stderr.take().ok_or("无法获取微信桥 stderr")?;

        // 后台循环（泵/通知订阅）桥生命周期内只起一次；通道接收端只能取一次，
        // 后续进程重启不再重建（事件通道与具体子进程解耦）。
        {
            let mut g = self.inner.lock().await;
            g.child = Some(child);
            g.stdin = Some(stdin);
            g.alive = true;
            g.conn = "starting";
            g.detail = None;
            g.qr_content = None;
            // 新进程代际：接收器全部未启动，等待 accounts 快照按绑定逐个触发。
            g.started_accounts.clear();
            let pumps_first = !g.pumps_started;
            g.pumps_started = true;
            if pumps_first {
                let job_rx = g.job_rx.take();
                let done_rx = g.done_rx.take();
                self.spawn_notification_subscription();
                if let (Some(job_rx), Some(done_rx)) = (job_rx, done_rx) {
                    tauri::async_runtime::spawn(Self::pump_loop(self.clone(), job_rx, done_rx));
                }
            }
        }

        // stdout 读循环：每次拉起新进程都重建一个。
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<Value>(trimmed) {
                            Ok(v) => this.handle_sidecar_event(v).await,
                            Err(e) => {
                                this.log("warn", format!("无法解析 sidecar 输出: {e}")).await
                            }
                        }
                    }
                }
            }
            // 退出后续处理交由常驻监督任务，避免 reader future 递归引用 start_service。
            let _ = this.exit_tx.send(());
        });
        // stderr 仅记日志，便于排查库内部报错。
        let srv = self.server.clone();
        tauri::async_runtime::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let t = line.trim_end().to_string();
                        if !t.is_empty() {
                            srv.push_log("info", format!("[wechat][stderr] {t}")).await;
                        }
                    }
                }
            }
        });

        // 初始化指令：stateDir 指向应用数据目录，避免默认落到 ~/.wechannel。
        let state_dir = self.root.join("wechannel-data");
        self.send_cmd(json!({ "cmd": "init", "stateDir": state_dir.to_string_lossy() }))
            .await?;
        self.log("info", format!("sidecar 已启动：{}", script.display()))
            .await;
        self.emit_state().await;
        Ok(())
    }

    /// 停止 sidecar 并清空运行态（退出前 / 无绑定兜底调用）。绑定持久化数据保留。
    pub async fn stop_service(&self) {
        let mut g = self.inner.lock().await;
        if let Some(mut child) = g.child.take() {
            let _ = child.kill().await;
        }
        g.stdin = None;
        g.alive = false;
        g.conn = "offline";
        g.detail = None;
        g.qr_content = None;
        g.pending_bind = None;
        g.started_accounts.clear();
        g.account_conn.clear();
        g.queue.clear();
        g.overflow_notified.clear();
        g.active_thread = None;
        g.restarts = 0;
    }

}

impl WeChatBridge {
    /// sidecar 退出监督：存在绑定时静默重启一次。
    async fn supervise_exits(self: Arc<Self>, mut rx: mpsc::UnboundedReceiver<()>) {
        while rx.recv().await.is_some() {
            let allow_restart = {
                let mut g = self.inner.lock().await;
                g.alive = false;
                g.queue.clear();
                g.overflow_notified.clear();
                g.active_thread = None;
                let allow = g.restarts < 1 && !g.bindings.is_empty();
                if allow {
                    g.restarts += 1;
                    g.conn = "starting";
                    g.detail = Some("进程退出，3 秒后尝试重启".into());
                } else {
                    g.conn = "offline";
                    g.detail = None;
                }
                allow
            };
            self.emit_state().await;
            if !allow_restart {
                continue;
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
            if !self.has_bindings().await {
                continue;
            }
            if let Err(e) = self.start_service().await {
                self.set_error(format!("微信桥重启失败: {e}")).await;
                continue;
            }
        }
    }
}

impl WeChatBridge {
    // -- 命令面（供 Tauri command 调用） ------------------------------------

    /// 当前状态快照。
    pub async fn state(&self) -> Value {
        self.snapshot().await
    }

    /// 绑定列表（供历史面板徽标与模态框消费）。
    pub async fn bindings(&self) -> Value {
        let snap = self.snapshot().await;
        json!({
            "running": snap.get("running"),
            "bindings": snap.get("bindings"),
        })
    }

    /// 对指定会话发起扫码绑定：校验未绑定 → 起 sidecar → 下发 login，二维码经事件回传。
    pub async fn bind_login_start(self: &Arc<Self>, thread_id: &str) -> Result<(), String> {
        self.ensure_supervisor().await;
        {
            let g = self.inner.lock().await;
            if g.bindings
                .iter()
                .any(|b| b.get("threadId").and_then(|v| v.as_str()) == Some(thread_id))
            {
                return Err("该会话已绑定微信".into());
            }
        }
        self.start_service().await?;
        {
            let mut g = self.inner.lock().await;
            g.pending_bind = Some(thread_id.to_string());
            g.qr_content = None;
            g.detail = None;
            g.conn = "starting";
        }
        self.emit_state().await;
        self.send_cmd(json!({ "cmd": "login", "timeoutMs": 480_000 }))
            .await
    }

    /// 解除指定会话的微信绑定：清除本地凭据、删除绑定、停止该账号接收（幂等）。
    pub async fn unbind(self: &Arc<Self>, thread_id: &str) -> Result<(), String> {
        let target = self.binding_of_thread(thread_id).await;
        let Some(binding) = target else {
            return Ok(());
        };
        let account_id = binding.get("accountId").and_then(|v| v.as_str()).unwrap_or("").to_string();
        // 尽力清除 sidecar 侧凭据与接收器（未运行/失败不影响解绑）。
        if self.inner.lock().await.alive {
            if !account_id.is_empty() {
                let _ = self.send_cmd(json!({ "cmd": "logout", "accountId": account_id })).await;
                let _ = self.send_cmd(json!({ "cmd": "stop", "accountId": account_id })).await;
            }
        }
        {
            let mut g = self.inner.lock().await;
            g.bindings.retain(|b| {
                b.get("threadId").and_then(|v| v.as_str()) != Some(thread_id)
            });
            if !account_id.is_empty() {
                g.started_accounts.remove(&account_id);
                g.account_conn.remove(&account_id);
            }
            if g.pending_bind.as_deref() == Some(thread_id) {
                g.pending_bind = None;
            }
            if g.bindings.is_empty() {
                g.conn = "offline";
                g.detail = None;
                g.qr_content = None;
            }
        }
        let _ = save_bindings(&self.root, &self.inner.lock().await.bindings.clone()).await;
        self.log("info", format!("已解除会话 {thread_id} 的微信绑定")).await;
        self.emit_state().await;
        Ok(())
    }

    // -- sidecar 事件分发 ----------------------------------------------------

    async fn handle_sidecar_event(self: &Arc<Self>, v: Value) {
        let ev = v.get("event").and_then(|x| x.as_str()).unwrap_or("");
        match ev {
            "ready" => {
                {
                    let mut g = self.inner.lock().await;
                    g.detail = None;
                }
                self.emit_state().await;
            }
            "qr" => {
                if let Some(content) = extract_qr(&v) {
                    {
                        let mut g = self.inner.lock().await;
                        g.qr_content = Some(content);
                        g.conn = "awaiting_qr";
                        g.detail = None;
                    }
                    self.emit_state().await;
                }
            }
            "login_result" => {
                let success = v.get("success").and_then(|x| x.as_bool()).unwrap_or(false);
                let message = v
                    .get("message")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let account =
                    v.get("accountId").and_then(|x| x.as_str()).map(str::to_string);
                let user_id = v.get("userId").and_then(|x| x.as_str()).map(str::to_string);
                let pending = self.inner.lock().await.pending_bind.clone();
                if success {
                    match (pending, account, user_id) {
                        (Some(thread_id), Some(account), Some(user_id)) => {
                            let start_account = account.clone();
                            let conflict = {
                                let g = self.inner.lock().await;
                                binding_conflict(&g.bindings, &thread_id, &account)
                            };
                            if conflict {
                                self.set_error("绑定冲突：该会话或该微信账号已被绑定".into())
                                    .await;
                            } else {
                                let bound_at = SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .map(|d| d.as_millis())
                                    .unwrap_or(0);
                                let binding = json!({
                                    "threadId": thread_id,
                                    "accountId": account,
                                    "userId": user_id,
                                    "name": null,
                                    "boundAt": bound_at,
                                });
                                {
                                    let mut g = self.inner.lock().await;
                                    g.bindings.push(binding);
                                    g.pending_bind = None;
                                    g.qr_content = None;
                                    g.conn = "starting";
                                    g.detail = None;
                                    g.started_accounts.insert(start_account.clone());
                                    g.account_conn.insert(start_account.clone(), "starting".into());
                                }
                                let _ = save_bindings(
                                    &self.root,
                                    &self.inner.lock().await.bindings.clone(),
                                )
                                .await;
                                self.log(
                                    "info",
                                    format!("会话 {thread_id} 已绑定微信账号 {start_account}"),
                                )
                                .await;
                                if let Err(e) = self
                                    .send_cmd(json!({ "cmd": "start", "accountId": start_account }))
                                    .await
                                {
                                    self.set_error(format!("启动消息接收失败: {e}")).await;
                                }
                            }
                        }
                        _ => {
                            self.log(
                                "warn",
                                "收到未预期的登录成功（缺少待绑定会话或账号信息）".into(),
                            )
                            .await;
                            self.emit_state().await;
                        }
                    }
                } else {
                    let mut g = self.inner.lock().await;
                    g.conn = "error";
                    g.detail = Some(if message.is_empty() {
                        "扫码登录未完成或已超时".into()
                    } else {
                        message
                    });
                    // 失败后清掉待绑定目标，允许用户重新发起扫码。
                    g.pending_bind = None;
                }
                self.emit_state().await;
            }
            "session_status" => {
                let status = v
                    .pointer("/status/status")
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                let account = v
                    .pointer("/status/accountId")
                    .and_then(|x| x.as_str())
                    .map(str::to_string);
                {
                    let mut g = self.inner.lock().await;
                    if let Some(id) = account.as_deref() {
                        if g.bindings
                            .iter()
                            .any(|b| b.get("accountId").and_then(|v| v.as_str()) == Some(id))
                        {
                            g.account_conn.insert(id.to_string(), status.to_string());
                        }
                    }
                    // 桥级连接取「最优」状态：任一 connected → connected；否则有过期 →
                    // session_expired；否则有 disconnected → offline；否则维持原值。
                    let any_connected = g.account_conn.values().any(|s| s == "connected");
                    let any_expired = g.account_conn.values().any(|s| s == "session_expired");
                    let any_disconnected = g.account_conn.values().any(|s| s == "disconnected");
                    if any_connected {
                        g.conn = "connected";
                        g.detail = None;
                    } else if any_expired {
                        g.conn = "session_expired";
                        g.detail = Some("微信会话过期，请重新扫码".into());
                    } else if any_disconnected && g.conn != "starting" {
                        g.conn = "offline";
                        g.detail = Some("连接断开，等待重连".into());
                    }
                }
                self.emit_state().await;
            }
            "message" => {
                if let Some((account, from, text)) = extract_message(&v) {
                    let thread_id = {
                        let g = self.inner.lock().await;
                        find_binding_for_message(&g.bindings, &account, &from)
                            .and_then(|b| b.get("threadId").and_then(|v| v.as_str()))
                            .map(str::to_string)
                    };
                    if let Some(thread_id) = thread_id {
                        self.enqueue_inbound(InboundMessage {
                            account_id: account,
                            thread_id,
                            from,
                            text,
                        })
                        .await;
                    } else {
                        // 「谁扫谁白」门禁：非绑定账号/非本人消息仅首次记一条日志，防刷屏。
                        let key = format!("{account}|{from}");
                        let first_ignore = {
                            let mut g = self.inner.lock().await;
                            g.ignored_logged.insert(key.clone())
                        };
                        if first_ignore {
                            self.log(
                                "info",
                                format!("忽略非绑定消息：{from}（账号 {account} 未绑定该会话）"),
                            )
                            .await;
                        }
                    }
                }
                // 无文本（图片/语音等）按方案直接忽略。
            }
            "accounts" => {
                // 重启复登：对所有已绑定且可启动的账号逐个恢复接收（每进程代际一次）。
                if let Some(list) = v.get("accounts").and_then(|x| x.as_array()).cloned() {
                    let mut started: Vec<String> = Vec::new();
                    {
                        let mut g = self.inner.lock().await;
                        let bound: Vec<String> = g
                            .bindings
                            .iter()
                            .filter_map(|b| {
                                b.get("accountId").and_then(|v| v.as_str()).map(str::to_string)
                            })
                            .collect();
                        for acct in &list {
                            let id = acct.get("id").and_then(|x| x.as_str()).unwrap_or("");
                            if bound.iter().any(|b| b == id)
                                && account_is_startable(acct)
                                && !g.started_accounts.contains(id)
                            {
                                g.started_accounts.insert(id.to_string());
                                g.account_conn.insert(id.to_string(), "starting".into());
                                started.push(id.to_string());
                            }
                        }
                    }
                    for id in started {
                        self.log("info", format!("已自动恢复消息接收（{id}）")).await;
                        if let Err(e) =
                            self.send_cmd(json!({ "cmd": "start", "accountId": id })).await
                        {
                            self.set_error(format!("自动恢复消息接收失败: {e}")).await;
                        }
                    }
                    self.emit_state().await;
                }
            }
            "error" => {
                let message = v
                    .get("message")
                    .and_then(|x| x.as_str())
                    .unwrap_or("未知错误")
                    .to_string();
                self.log("warn", format!("sidecar 错误: {message}")).await;
                let mut g = self.inner.lock().await;
                g.detail = Some(message);
                // 扫码绑定流程中的错误（pending 非空）：清掉待绑定目标与二维码，
                // 让弹窗按钮恢复可用、可重新发起扫码；已绑定账号的运行时错误不受影响。
                if g.pending_bind.is_some() {
                    g.pending_bind = None;
                    g.qr_content = None;
                }
            }
            _ => {}
        }
    }

    /// 入站文本消息入队（绑定路由已通过）；满队时立即回一次忙碌提示并丢弃该条。
    async fn enqueue_inbound(self: &Arc<Self>, msg: InboundMessage) {
        let mut g = self.inner.lock().await;
        if !g.alive {
            return; // 进程不在，静默丢弃（重启后新消息自然流入）
        }
        g.overflow_notified.remove(&msg.from);
        let full = queue_overflow(g.active_thread.is_some(), g.queue.len());
        if full {
            let peer = msg.from.clone();
            let account = msg.account_id.clone();
            if g.overflow_notified.insert(peer.clone()) {
                drop(g);
                self.send_reply_now(&account, &peer, "处理队列已满，稍后再试。")
                    .await;
            }
            return;
        }
        g.queue.push_back(msg);
        drop(g);
        let _ = self.job_tx.send(());
    }

    /// 绕过队列的即时回复（忙碌提示 / 失败提示等系统文案）。
    async fn send_reply_now(self: &Arc<Self>, account_id: &str, peer: &str, text: &str) {
        for chunk in chunk_text(text, REPLY_CHUNK_CHARS) {
            if let Err(e) = self
                .send_cmd(json!({
                    "cmd": "send_text",
                    "accountId": account_id,
                    "toUserId": peer,
                    "text": chunk,
                }))
                .await
            {
                self.log("warn", format!("回复 {peer} 失败: {e}")).await;
            }
        }
    }
}

impl WeChatBridge {
    // -- Codex 回合编排 ------------------------------------------------------

    /// 订阅 codex 服务器通知：收集 agentMessage 文本并在 turn/completed 时唤醒泵。
    fn spawn_notification_subscription(self: &Arc<Self>) {
        let mut rx = self.server.subscribe_notifications();
        let this = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok((method, params)) => {
                        match method.as_str() {
                            "item/completed" => {
                                let is_agent = params.pointer("/item/type").and_then(|t| t.as_str()) == Some("agentMessage");
                                if !is_agent {
                                    continue;
                                }
                                if let (Some(tid), Some(text)) = (
                                    params.get("threadId").and_then(|x| x.as_str()),
                                    params.pointer("/item/text").and_then(|x| x.as_str()),
                                ) {
                                    if text.is_empty() {
                                        continue;
                                    }
                                    // 同回合可能多条 agentMessage：保留最后一条完整文本。
                                    this.inner
                                        .lock()
                                        .await
                                        .pending_reply
                                        .insert(tid.to_string(), text.to_string());
                                }
                            }
                            "turn/completed" => {
                                if let Some(tid) = params.get("threadId").and_then(|x| x.as_str()) {
                                    let status = params
                                        .pointer("/turn/status")
                                        .and_then(|x| x.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let _ = this.done_tx.send((tid.to_string(), status));
                                }
                            }
                            _ => {}
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        this.log("warn", format!("通知订阅滞后，跳过 {n} 条")).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    /// 核心串行泵：出队 → 在绑定线程上发起回合 → 等完成 → 回复 → 下一条。
    async fn pump_loop(
        self: Arc<Self>,
        mut wake: mpsc::UnboundedReceiver<()>,
        mut done: mpsc::UnboundedReceiver<(String, String)>,
    ) {
        let mut current: Option<ActiveTurn> = None;
        loop {
            // 空闲则先尝试取队首。
            if current.is_none() {
                let job = self.inner.lock().await.queue.pop_front();
                if let Some(job) = job {
                    match self.begin_turn(job).await {
                        Ok(t) => current = Some(t),
                        Err((account, peer, fail_text)) => {
                            self.send_reply_now(&account, &peer, &fail_text).await;
                        }
                    }
                    continue;
                }
            }
            if let Some(c) = current.take() {
                let remaining = c
                    .deadline
                    .checked_duration_since(std::time::Instant::now())
                    .unwrap_or_default();
                match timeout(remaining, done.recv()).await {
                    Err(_elapsed) => {
                        // 超时：清理槽位并回复失败提示。
                        self.inner.lock().await.pending_reply.remove(&c.thread_id);
                        self.finish_active(&c.account_id, &c.peer, "⚠️ 执行失败：等待 Codex 回合超时")
                            .await;
                    }
                    Ok(None) => break, // 完成通道关闭（应用退出）
                    Ok(Some((tid, status))) => {
                        if tid != c.thread_id {
                            current = Some(c); // 他线完成信号：放回继续等本线程
                            continue;
                        }
                        let text = self.inner.lock().await.pending_reply.remove(&c.thread_id).unwrap_or_default();
                        let final_text = normalize_turn_text(&text, &status);
                        self.finish_active(&c.account_id, &c.peer, &final_text).await;
                    }
                }
            } else {
                // 队列空且无进行中回合：停到下一条入站唤醒。
                if wake.recv().await.is_none() {
                    break;
                }
            }
        }
    }

    /// 回合开始：在绑定线程上发起 turn/start；失败时返回 (账号, 联系人, 失败文案)。
    async fn begin_turn(self: &Arc<Self>, job: InboundMessage) -> Result<ActiveTurn, (String, String, String)> {
        let peer = job.from.clone();
        let account = job.account_id.clone();
        let thread_id = job.thread_id.clone();
        match self.run_turn_on_thread(&account, &peer, &thread_id, &job.text).await {
            Ok(t) => Ok(t),
            Err((p, fail)) => {
                // 绑定线程已被删除（外部删除/清理）：自动解除该会话的微信绑定并停止接收，
                // 不回自动重建（绑定语义下新线程不具原身份）。
                if is_thread_not_found(&fail) {
                    self.log("info", format!("会话 {thread_id} 已不存在，自动解除微信绑定")).await;
                    let _ = self.unbind(&thread_id).await;
                    Err((
                        account,
                        peer,
                        "⚠️ 该会话已被删除，微信绑定已解除，请重新绑定会话".into(),
                    ))
                } else {
                    Err((account, p, fail))
                }
            }
        }
    }

    /// 在确认的线程上发起回合：设置活动线程 → 恢复(激活)线程 → 解析默认模型 → 组装参数 → turn/start。
    /// 失败返回 (peer, 已格式化文案)；该文案用于识别「thread not found」以触发解绑。
    async fn run_turn_on_thread(
        self: &Arc<Self>,
        account_id: &str,
        peer: &str,
        thread_id: &str,
        text: &str,
    ) -> Result<ActiveTurn, (String, String)> {
        {
            let mut g = self.inner.lock().await;
            g.active_thread = Some(thread_id.to_string());
        }
        // 先 thread/resume 激活线程：codex 的 turn/start 只对已 resume 的线程可寻址，
        // 否则会对存在于会话库的线程误报 thread not found（前端发送前也是先 resume）。
        // 仅当 resume 也报 thread not found 时才判定线程确实缺失。
        if let Err(e) = self
            .server
            .request(
                "thread/resume",
                json!({ "threadId": thread_id }),
                Some(Duration::from_secs(30)),
            )
            .await
        {
            self.clear_active(thread_id).await;
            return Err((peer.to_string(), format!("⚠️ 执行失败：{e}")));
        }
        let model = match self.resolve_default_model().await {
            Ok(m) => m,
            Err(e) => {
                self.log("warn", e).await;
                String::new()
            }
        };
        if model.is_empty() {
            self.clear_active(thread_id).await;
            return Err((
                peer.to_string(),
                "⚠️ 执行失败：无法解析默认模型（检查 codex 登录与模型列表）".into(),
            ));
        }
        let params = build_turn_params(thread_id, text, &model, &self.next_message_id());
        if let Err(e) = self
            .server
            .request("turn/start", params, Some(Duration::from_secs(60)))
            .await
        {
            self.clear_active(thread_id).await;
            return Err((peer.to_string(), format!("⚠️ 执行失败：{e}")));
        }
        Ok(ActiveTurn {
            thread_id: thread_id.to_string(),
            account_id: account_id.to_string(),
            peer: peer.to_string(),
            deadline: std::time::Instant::now() + Duration::from_secs(TURN_TIMEOUT_SECS),
        })
    }

    async fn clear_active(&self, thread_id: &str) {
        let mut g = self.inner.lock().await;
        g.pending_reply.remove(thread_id);
        if g.active_thread.as_deref() == Some(thread_id) {
            g.active_thread = None;
            g.overflow_notified.clear();
        }
    }

    /// 回复联系人并把状态推进到空闲（触发下一条处理）。
    async fn finish_active(self: &Arc<Self>, account_id: &str, peer: &str, text: &str) {
        {
            let mut g = self.inner.lock().await;
            g.active_thread = None;
            g.overflow_notified.clear();
        }
        self.send_reply_now(account_id, peer, text).await;
        let _ = self.job_tx.send(());
    }

    /// 默认模型解析（进程内缓存）：isDefault 优先，其次首个非 hidden。
    async fn resolve_default_model(self: &Arc<Self>) -> Result<String, String> {
        {
            let cached = self.inner.lock().await.default_model.clone();
            if let Some(res) = cached {
                return res;
            }
        }
        let res = (|| async {
            let resp = self
                .server
                .request("model/list", json!({}), Some(Duration::from_secs(30)))
                .await?;
            let list = resp.get("data").and_then(|d| d.as_array()).cloned().unwrap_or_default();
            let pick = list.iter().find(|m| {
                m.get("isDefault").and_then(|x| x.as_bool()).unwrap_or(false)
                    && !m.get("hidden").and_then(|x| x.as_bool()).unwrap_or(false)
            });
            let pick = pick.or_else(|| {
                list.iter().find(|m| {
                    !m.get("hidden").and_then(|x| x.as_bool()).unwrap_or(true)
                })
            });
            pick.and_then(|m| m.get("model").and_then(|x| x.as_str()))
                .map(str::to_string)
                .ok_or_else(|| "模型列表为空".to_string())
        })()
        .await;
        self.inner.lock().await.default_model = Some(res.clone());
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_is_startable_matrix() {
        // 已配置且未过期 → 可启动
        assert!(account_is_startable(&json!({ "configured": true })));
        assert!(account_is_startable(&json!({ "configured": true, "status": "connected" })));
        assert!(account_is_startable(&json!({ "configured": true, "status": "disconnected" })));
        // session_expired 或未配置 → 不可启动
        assert!(!account_is_startable(&json!({ "configured": true, "status": "session_expired" })));
        assert!(!account_is_startable(&json!({ "configured": false, "status": "connected" })));
    }

    #[test]
    fn binding_conflict_matrix() {
        let bindings = json!([
            { "threadId": "t-1", "accountId": "bot-1", "userId": "u-1" },
        ]);
        let list = bindings.as_array().unwrap();
        // 同线程冲突
        assert!(binding_conflict(list, "t-1", "bot-2"));
        // 同账号冲突
        assert!(binding_conflict(list, "t-2", "bot-1"));
        // 均不冲突
        assert!(!binding_conflict(list, "t-2", "bot-2"));
    }

    #[test]
    fn find_binding_for_message_routes_by_account_and_owner() {
        let bindings = json!([
            { "threadId": "t-1", "accountId": "bot-1", "userId": "wx_me" },
            { "threadId": "t-2", "accountId": "bot-2", "userId": "wx_other" },
        ]);
        let list = bindings.as_array().unwrap();
        // 账号 + 本人命中；两侧 trim 后匹配
        let hit = find_binding_for_message(list, "bot-1", " wx_me ");
        assert_eq!(
            hit.and_then(|b| b.get("threadId").and_then(|v| v.as_str())),
            Some("t-1")
        );
        // 非本人 / 非绑定账号不命中
        assert!(find_binding_for_message(list, "bot-1", "wx_friend").is_none());
        assert!(find_binding_for_message(list, "bot-9", "wx_me").is_none());
    }

    #[test]
    fn queue_overflow_only_when_busy_at_limit() {
        // 空闲时不判满（泵会立即取走队首）；忙线且到达上限才丢弃。
        assert!(!queue_overflow(false, QUEUE_LIMIT));
        assert!(!queue_overflow(true, QUEUE_LIMIT - 1));
        assert!(queue_overflow(true, QUEUE_LIMIT));
    }

    #[test]
    fn chunk_text_splits_utf8_safely() {
        let text = "中文a🦞".repeat(700); // 2800 个可见字符，含多字节 emoji
        let chunks = chunk_text(&text, REPLY_CHUNK_CHARS);
        assert_eq!(chunks.len(), 2);
        let joined: String = chunks.concat();
        assert_eq!(joined, text);
        // 空文本与短文本不分段
        assert_eq!(chunk_text("", 10), vec![String::new()]);
        assert_eq!(chunk_text("hi", 10).len(), 1);
    }

    #[test]
    fn extract_qr_and_message_shape() {
        let qr = json!({ "event": "qr", "content": "http://weixin/abc" });
        assert_eq!(
            extract_qr(&qr).as_deref(),
            Some("http://weixin/abc")
        );
        assert!(extract_qr(&json!({ "event": "qr" })).is_none());
        assert!(extract_qr(&json!({ "event": "other" })).is_none());

        let msg = json!({
            "event": "message",
            "message": {
                "accountId": "bot-1",
                "from": "wx_alice",
                "text": " 1 + 1 = ? ",
                "image": { "path": "x.jpg" },
            },
        });
        let (account, from, text) = extract_message(&msg).unwrap();
        assert_eq!(account, "bot-1");
        assert_eq!(from, "wx_alice");
        assert_eq!(text, "1 + 1 = ?");

        // 媒体消息无文本：返回 None 由调用方忽略
        let media = json!({
            "event": "message",
            "message": { "accountId": "b", "from": "p", "image": {} },
        });
        assert!(extract_message(&media).is_none());
    }

    #[test]
    fn turn_params_carry_never_policy_and_default_collab() {
        let v = build_turn_params("t-1", "hello", "gpt-x", "wechat-1-0");
        assert_eq!(v["threadId"], "t-1");
        assert_eq!(v["input"][0]["type"], "text");
        assert_eq!(v["input"][0]["text"], "hello");
        assert_eq!(v["clientUserMessageId"], "wechat-1-0");
        assert_eq!(v["approvalPolicy"], "never");
        assert_eq!(v["sandboxPolicy"]["type"], "dangerFullAccess");
        assert_eq!(v["collaborationMode"]["mode"], "default");
        assert_eq!(v["collaborationMode"]["settings"]["model"], "gpt-x");
    }

    #[test]
    fn normalize_turn_text_falls_back_for_empty_results() {
        assert_eq!(normalize_turn_text(" ok ", "completed"), "ok");
        assert!(normalize_turn_text("", "interrupted").contains("已中断"));
        assert!(normalize_turn_text("", "").contains("未收到"));
        assert!(normalize_turn_text("", "failed").contains("failed"));
    }

    #[tokio::test]
    async fn bindings_roundtrip_atomic() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(load_bindings(root).await.is_empty());
        let list = json!([
            { "threadId": "t-a", "accountId": "bot-a", "userId": "u-a", "boundAt": 1 },
            { "threadId": "t-b", "accountId": "bot-b", "userId": "u-b", "boundAt": 2 },
        ]);
        let arr = list.as_array().unwrap();
        save_bindings(root, arr).await.unwrap();
        let loaded = load_bindings(root).await;
        assert_eq!(loaded.len(), 2);
        assert_eq!(
            loaded[0].get("threadId").and_then(|v| v.as_str()),
            Some("t-a")
        );
        // 覆盖写不残留临时文件
        let list2 = json!([{ "threadId": "t-c", "accountId": "bot-c", "userId": "u-c" }]);
        save_bindings(root, list2.as_array().unwrap()).await.unwrap();
        assert_eq!(load_bindings(root).await.len(), 1);
        assert!(!root.join("bindings.json.tmp").exists());
    }

    #[test]
    fn thread_not_found_detection() {
        // 正例：codex 删除线程后 turn/start 的典型报错。
        assert!(is_thread_not_found(
            "⚠️ 执行失败：thread not found: 01a04189-376d-7660-a4a3-94afbe4b9d5d"
        ));
        assert!(is_thread_not_found("thread not found: abc"));
        assert!(is_thread_not_found("Thread Not Found: abc")); // 大小写不敏感
        assert!(is_thread_not_found("thread does not exist: abc"));
        // 反例：不误判其它 not found。
        assert!(!is_thread_not_found("⚠️ 执行失败：model not found"));
        assert!(!is_thread_not_found("⚠️ 执行失败：file not found: x"));
        assert!(!is_thread_not_found(""));
    }

}
