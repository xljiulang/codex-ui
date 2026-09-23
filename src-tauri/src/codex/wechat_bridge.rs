//! 微信 ClawBot 接入桥。
//!
//! 架构：本模块负责「会话 ↔ 微信账号」绑定（存入统一会话状态 sessions.json）、回合编排与回复路由；
//! 微信协议（ilink bot API）由纯 Rust 的 wechat_client 承担，事件经 mpsc 推送本模块
//! 消费（语义对齐原 Node sidecar 的 stdio 事件，前端 wechat/event 协议不变）。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, oneshot, Mutex, Notify};

use crate::codex::app_server::CodexServer;
use crate::codex::session_state::{SessionStateStore, WechatBinding};
use crate::codex::wechat_client::{WechatClient, WechatEvent};

/// 回复超长时按可见字符数切分发送。
const REPLY_CHUNK_CHARS: usize = 1800;
/// “正在输入”状态周期续发间隔（秒）。
const TYPING_INTERVAL_SECS: u64 = 8;
/// steer 撞上「回合尚未被服务端标记为可转向」窗口时的短重试次数与间隔
/// （与前端 steerTurn 同款：`turn/start` 响应早于 `turn/started`）。
const STEER_RETRY_ATTEMPTS: u32 = 5;
const STEER_RETRY_DELAY_MS: u64 = 400;
/// `turn/start` 响应未带回合 id（防御性分支）时，短等 `turn/started` 回填的次数与间隔。
const STEER_TURN_ID_WAIT_ATTEMPTS: u32 = 10;
const STEER_TURN_ID_WAIT_STEP_MS: u64 = 300;
/// 把回合产物整理为回复文本：无文本/非正常结束给出对应中文兜底提示。
fn normalize_turn_text(text: &str, status: &str) -> String {
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    match status {
        "interrupted" | "cancelled" | "canceled" => "⚠️ 回合已中断，本轮没有产生回复文本".into(),
        "completed" => "⚠️ Codex 本轮没有输出文本回复".into(),
        "" => "⚠️ 本轮回合未返回文本回复".into(),
        other => format!("⚠️ 执行失败：回合异常结束（{other}）"),
    }
}

/// 从 `thread/turns/list(itemsView=full)` 响应里取指定回合的最后一条非空 `agentMessage` 文本。
/// 用于弥补通知订阅漏抓 `item/completed`(agentMessage) 的兜底。
fn extract_turn_agent_text(resp: &Value, turn_id: &str) -> Option<String> {
    let turns = resp.pointer("/data").and_then(|d| d.as_array())?;
    let turn = turns
        .iter()
        .find(|t| t.get("id").and_then(|v| v.as_str()) == Some(turn_id))?;
    let items = turn.get("items")?.as_array()?;
    for item in items.iter().rev() {
        if item.get("type").and_then(|v| v.as_str()) == Some("agentMessage") {
            let text = item.get("text").and_then(|v| v.as_str()).unwrap_or("");
            if !text.trim().is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

/// 决定最终回复文本：已捕获文本优先；否则用回查取到的文本；均无则按状态兜底。
fn final_reply_text(captured: &str, status: &str, fetched: Option<String>) -> String {
    let trimmed = captured.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    if let Some(f) = fetched {
        let ft = f.trim();
        if !ft.is_empty() {
            return ft.to_string();
        }
    }
    normalize_turn_text(captured, status)
}
/// 前端事件名：状态每次变化全量推送快照。
pub const WECHAT_EVENT: &str = "wechat/event";

/// 一条已通过「谁扫谁白」门禁的入站消息（文本与/或图片，携带目标绑定信息）。
#[derive(Debug, Clone)]
struct InboundMessage {
    /// 消息所属微信账号（即绑定账号）。
    account_id: String,
    /// 绑定目标线程 id（消息将路由到该线程执行回合）。
    thread_id: String,
    from: String,
    /// 文本内容；纯图片消息为 None。
    text: Option<String>,
    /// 已下载解密落盘的图片绝对路径（按微信 item_list 顺序）。
    images: Vec<String>,
    /// 已下载解密落盘的文件/视频绝对路径（按微信 item_list 顺序）。
    files: Vec<String>,
}

// ---------------------------------------------------------------------------
// 纯函数工具（单元测试覆盖）
// ---------------------------------------------------------------------------

/// steer 失败分类：决定「另开新回合」还是「回失败提示」。
#[derive(Debug, PartialEq, Eq)]
enum SteerOutcome {
    /// 服务端没有可转向的活动回合（回合已结束 / 尚未标记为可转向）→ 另开新回合。
    NoActiveTurn,
    /// 当前回合类型不支持转向（review / 手动 compact 等）→ 回失败提示，不另开回合。
    NotSteerable,
    /// 其它错误（连接中断等）→ 回失败提示，不另开回合。
    Other,
}

/// 错误文案归类：`no active turn` 视为「回合不在」，`not steerable` 视为「不可转向」。
fn classify_steer_error(err: &str) -> SteerOutcome {
    let e = err.to_ascii_lowercase();
    if e.contains("no active turn") {
        SteerOutcome::NoActiveTurn
    } else if e.contains("not steerable") || e.contains("activeturnnotsteerable") {
        SteerOutcome::NotSteerable
    } else {
        SteerOutcome::Other
    }
}

/// 「thread not found」判定：codex 删除线程后 turn/start 的稳定错误文案。
/// 仅识别线程不存在，避免误判 model/file 等其它 not found。
fn is_thread_not_found(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("thread not found") || e.contains("thread does not exist")
}

/// 完成信号是否属于当前进行中的回合：线程 id 必须匹配；回合 id 仅在双方非空时精确匹配，
/// 允许 turn/start 响应缺 id 时按线程匹配兜底（避免永不完成）。
fn is_this_turn_completion(
    thread_id: &str,
    turn_id: &str,
    current_thread: &str,
    current_turn: &str,
) -> bool {
    if thread_id != current_thread {
        return false;
    }
    let exact_turn = !current_turn.is_empty() && !turn_id.is_empty();
    !exact_turn || turn_id == current_turn
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

/// 消息是否构成一次回合输入：有非空文本或至少一个附件（无载荷的媒体消息只是忽略）。
fn has_turn_payload(text: Option<&str>, images: &[String], files: &[String]) -> bool {
    text.is_some_and(|t| !t.trim().is_empty()) || !images.is_empty() || !files.is_empty()
}

/// 协议侧路径统一正斜杠：Windows 反斜杠路径 codex 侧读不到（与前端 mention.toProtocolPath 一致）。
fn proto_path(path: &str) -> String {
    path.replace('\\', "/")
}

/// 取路径的文件名（文件段里展示给 Codex 的名字）。
fn base_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_string()
}

/// 文件引用段标题（与前端 mention.ts / VS Code Codex 扩展一致）。
const FILE_MENTION_HEADING: &str = "# Files mentioned by the user:";
/// 用户请求分隔标记（同上）。
const MY_REQUEST_MARKER: &str = "## My request:";
/// 纯附件消息（无文字）时补的说明：避免 codex 收到空请求。
const BARE_ATTACHMENT_NOTE: &str = "（用户只发送了附件，没有文字说明，请查看附件内容）";

/// 文件引用文本段：`# Files mentioned by the user:` 下每行 `## 名字: 路径`（与前端 `fileMentionSection` 同款格式）。
/// codex 会丢弃独立的 `mention` 输入项，文件路径只能靠文本承载。
fn file_mention_section(files: &[String]) -> String {
    if files.is_empty() {
        return String::new();
    }
    let refs: String = files
        .iter()
        .map(|p| format!("\n## {}: {}\n", base_name(p), proto_path(p)))
        .collect();
    format!("\n{FILE_MENTION_HEADING}\n{refs}")
}

/// 组装 turn/start 的 input：图片（localImage）→ 文本项（用户文本，文件/视频以文件段承载）。
/// 纯图片消息不带文本项（codex 会自行包装 `<image …>`）。
fn build_turn_input(text: Option<&str>, images: &[String], files: &[String]) -> Vec<Value> {
    let mut input: Vec<Value> = images
        .iter()
        .map(|p| json!({ "type": "localImage", "path": proto_path(p) }))
        .collect();
    let section = file_mention_section(files);
    let body = match (text, section.is_empty()) {
        // 有文件：文件段 + `## My request:` + 用户文本（无文字时用占位说明）
        (Some(t), false) => Some(format!("{section}\n{MY_REQUEST_MARKER}\n{t}")),
        (None, false) => Some(format!("{section}\n{MY_REQUEST_MARKER}\n{BARE_ATTACHMENT_NOTE}")),
        // 无文件：有文本就原样发，纯图片不带文本项
        (Some(t), true) => Some(t.to_string()),
        (None, true) => None,
    };
    if let Some(body) = body {
        input.push(json!({ "type": "text", "text": body }));
    }
    input
}

/// turn/start 参数：完整能力 + 免审批（dangerFullAccess 沙箱策略）。
fn build_turn_params(
    thread_id: &str,
    text: Option<&str>,
    images: &[String],
    files: &[String],
    model: &str,
    effort: Option<&str>,
    client_message_id: &str,
) -> Value {
    json!({
        "threadId": thread_id,
        "input": build_turn_input(text, images, files),
        "clientUserMessageId": client_message_id,
        "approvalPolicy": "never",
        "sandboxPolicy": { "type": "dangerFullAccess" },
        "model": model,
        "effort": effort,
        "collaborationMode": {
            "mode": "default",
            "settings": {
                "model": model,
                "reasoning_effort": effort,
                "developer_instructions": null,
            },
        },
    })
}

/// turn/steer 参数：把一条消息并入指定回合（`expectedTurnId` 前置校验）。
/// 与前端 `steerTurn` 同形，输入构造复用 `build_turn_input`。
fn build_steer_params(
    thread_id: &str,
    text: Option<&str>,
    images: &[String],
    files: &[String],
    client_message_id: &str,
    expected_turn_id: &str,
) -> Value {
    json!({
        "threadId": thread_id,
        "input": build_turn_input(text, images, files),
        "clientUserMessageId": client_message_id,
        "expectedTurnId": expected_turn_id,
    })
}

// ---------------------------------------------------------------------------
// 桥本体状态
// ---------------------------------------------------------------------------

struct BridgeInner {
    /// 微信协议客户端是否已启用（决定快照 running 字段）。
    alive: bool,
    /// 连接状态：offline｜starting｜awaiting_qr｜connected｜session_expired｜error
    conn: &'static str,
    detail: Option<String>,
    qr_content: Option<String>,
    /// 登录中的绑定目标线程 id（模态框扫码绑定流程的待落盘状态）。
    pending_bind: Option<String>,
    /// 当前 pending 登录的关联 id（用于过滤被接管/取消后的陈旧 login_result）。
    pending_login_id: Option<String>,
    /// 当前生效的「会话 ↔ 微信账号」绑定（内存镜像，变更即落盘 bindings.json）。
    bindings: Vec<Value>,
    /// 已启动过接收器的账号（防止重复启动）。
    started_accounts: HashSet<String>,
    /// 各绑定账号的连接状态（offline/starting/connected/session_expired/error）。
    account_conn: HashMap<String, String>,
    /// 各线程进行中的 Codex 回合（键=threadId）；无条目表示该线程空闲。
    /// 线程之间并行，互不阻塞；同线程靠 `thread_locks` 串行化。
    active: HashMap<String, ActiveTurn>,
    /// 线程级串行锁：同线程的入站消息按到达顺序执行「转向或开新回合」。
    thread_locks: HashMap<String, Arc<Mutex<()>>>,
    /// 各线程最近完成的 agentMessage 文本（订阅任务收集，回合完成后取走）。
    pending_reply: HashMap<String, String>,
    /// 默认模型缓存：None 未解析过；Some(..) 已解析一次（成功/失败皆缓存）。
    default_model: Option<Result<String, String>>,
    /// 后台订阅（通知 / 连接状态）是否已启动（桥生命周期内仅启动一次）。
    pumps_started: bool,
    /// 已记过首次忽略日志的「账号|发送者」键（进程内去重，防日志刷屏）。
    ignored_logged: HashSet<String>,
}

impl Default for BridgeInner {
    fn default() -> Self {
        Self {
            alive: false,
            conn: "offline",
            detail: None,
            qr_content: None,
            pending_bind: None,
            pending_login_id: None,
            bindings: Vec::new(),
            started_accounts: HashSet::new(),
            account_conn: HashMap::new(),
            active: HashMap::new(),
            thread_locks: HashMap::new(),
            pending_reply: HashMap::new(),
            default_model: None,
            pumps_started: false,
            ignored_logged: HashSet::new(),
        }
    }
}

pub struct WeChatBridge {
    app: AppHandle,
    server: Arc<CodexServer>,
    /// 统一会话状态存储（微信绑定写入 wechat 子字段）。
    store: Arc<SessionStateStore>,
    /// 纯 Rust 微信协议客户端（登录/接收/发送/存储）。
    client: WechatClient,
    /// 当前扫码登录的取消信号（共享；notify_waiters 不残留 permit）。
    login_cancel: Arc<Notify>,
    inner: Mutex<BridgeInner>,
    seq: AtomicU64,
}

/// 一次已发起的 Codex 回合（键为 threadId）。
struct ActiveTurn {
    turn_id: String,
    /// 完成信号投递口：通知订阅在匹配的 `turn/completed` 到达时 take 并投递。
    /// 等待方永久等待（不设时间上限）；仅通道被丢弃（回合被清理）时才提前退出。
    done: Option<oneshot::Sender<Completion>>,
    /// 周期续发“正在输入”的后台任务；回合结束/失败/连接断开时 abort。
    typing_task: Option<tauri::async_runtime::JoinHandle<()>>,
}

/// 回合完成信号：`turn/completed` 的回合 id 与结束状态。
struct Completion {
    turn_id: String,
    status: String,
}

impl WeChatBridge {
    pub fn new(
        app: AppHandle,
        server: Arc<CodexServer>,
        app_dir: PathBuf,
        store: Arc<SessionStateStore>,
    ) -> Arc<Self> {
        let root = app_dir.join("wechat");
        let (wechat_tx, mut wechat_rx) = mpsc::unbounded_channel();
        let client = WechatClient::new(root.clone(), wechat_tx);
        let bridge = Arc::new(Self {
            app,
            server,
            store,
            client,
            login_cancel: Arc::new(Notify::new()),
            inner: Mutex::new(BridgeInner::default()),
            seq: AtomicU64::new(0),
        });
        // 协议事件循环：消费 wechat_client 的 mpsc 事件（语义对齐原 sidecar stdio 事件）。
        {
            let this = bridge.clone();
            tauri::async_runtime::spawn(async move {
                while let Some(ev) = wechat_rx.recv().await {
                    this.handle_wechat_event(ev).await;
                }
            });
        }
        bridge
    }

    /// 是否存在生效绑定（内存镜像；autostart / 绑定变更时刷新）。
    async fn has_bindings(&self) -> bool {
        !self.inner.lock().await.bindings.is_empty()
    }

    /// 从磁盘装载绑定到内存。
    async fn reload_bindings(self: &Arc<Self>) {
        let list = self.store.wechat_bindings();
        self.inner.lock().await.bindings = list;
        self.sync_client_bindings().await;
    }

    /// 把「账号 → 会话 id」映射推给协议客户端（决定附件落盘目录：未绑定账号不落盘）。
    async fn sync_client_bindings(&self) {
        let map: HashMap<String, String> = {
            let g = self.inner.lock().await;
            g.bindings
                .iter()
                .filter_map(|b| {
                    let account = b.get("accountId").and_then(|v| v.as_str())?;
                    let thread = b.get("threadId").and_then(|v| v.as_str())?;
                    Some((account.to_string(), thread.to_string()))
                })
                .collect()
        };
        self.client.set_bindings(map).await;
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
            "busy": !inner.active.is_empty(),
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

    async fn log(&self, level: &str, line: String) {
        self.server.push_log(level, format!("[wechat] {line}")).await;
    }

    fn next_message_id(&self) -> String {
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default();
        let n = self.seq.fetch_add(1, Ordering::Relaxed);
        format!("wechat-{ms}-{n}")
    }

    /// 启用微信协议客户端（客户端对象常驻，此方法仅翻转 running 标记并启动订阅）。
    async fn ensure_client(self: &Arc<Self>) {
        let pumps_first = {
            let mut g = self.inner.lock().await;
            g.alive = true;
            let pumps_first = !g.pumps_started;
            g.pumps_started = true;
            pumps_first
        };
        if pumps_first {
            self.spawn_notification_subscription();
            self.spawn_connection_watch();
        }
    }

    // -- 生命周期 -----------------------------------------------------------

    /// 应用启动自动恢复：存在绑定则启用客户端，并按账号快照启动接收器。
    pub async fn autostart(self: &Arc<Self>) {
        self.reload_bindings().await;
        if !self.has_bindings().await {
            return;
        }
        self.ensure_client().await;
        let accounts = self.client.accounts();
        self.handle_wechat_event(WechatEvent::Accounts(accounts)).await;
    }

    /// 停止全部接收器并清空运行态（退出前调用）。绑定持久化数据保留。
    pub async fn shutdown(&self) {
        self.client.shutdown().await;
        let mut g = self.inner.lock().await;
        g.alive = false;
        g.conn = "offline";
        g.detail = None;
        g.qr_content = None;
        g.pending_bind = None;
        g.pending_login_id = None;
        g.started_accounts.clear();
        g.account_conn.clear();
        for (_, t) in g.active.drain() {
            if let Some(task) = t.typing_task {
                task.abort();
            }
        }
        g.pending_reply.clear();
    }

    // -- 命令面（供 Tauri command 调用） ------------------------------------

    /// 当前状态快照。
    pub async fn state(&self) -> Value {
        self.snapshot().await
    }

    /// 绑定列表（供会话面板徽标与模态框消费）。
    pub async fn bindings(&self) -> Value {
        let snap = self.snapshot().await;
        json!({
            "running": snap.get("running"),
            "bindings": snap.get("bindings"),
        })
    }

    /// 对指定会话发起扫码绑定：校验未绑定 → 启用客户端 → 发起登录（二维码经事件回传）。
    pub async fn bind_login_start(self: &Arc<Self>, thread_id: &str) -> Result<(), String> {
        {
            let g = self.inner.lock().await;
            if g.bindings
                .iter()
                .any(|b| b.get("threadId").and_then(|v| v.as_str()) == Some(thread_id))
            {
                return Err("该会话已绑定微信".into());
            }
        }
        self.ensure_client().await;
        // 取消上一个在途登录（若有），避免残留 QR/结果干扰本次绑定。
        self.login_cancel.notify_waiters();
        let login_id = self.next_message_id();
        {
            let mut g = self.inner.lock().await;
            g.pending_bind = Some(thread_id.to_string());
            g.pending_login_id = Some(login_id.clone());
            g.qr_content = None;
            g.detail = None;
            g.conn = "starting";
        }
        self.emit_state().await;
        self.client.start_login(login_id, self.login_cancel.clone());
        Ok(())
    }

    /// 取消当前扫码绑定（弹窗关闭时调用）：取消在途登录并清待绑定目标（幂等）。
    pub async fn cancel_bind(self: &Arc<Self>) {
        let had_pending = {
            let g = self.inner.lock().await;
            g.pending_bind.is_some() || g.pending_login_id.is_some()
        };
        if !had_pending {
            return;
        }
        self.login_cancel.notify_waiters();
        {
            let mut g = self.inner.lock().await;
            g.pending_bind = None;
            g.pending_login_id = None;
            g.qr_content = None;
            g.detail = None;
        }
        self.emit_state().await;
    }

    /// 解除指定会话的微信绑定：清除本地凭据、删除绑定、停止该账号接收（幂等）。
    pub async fn unbind(self: &Arc<Self>, thread_id: &str) -> Result<(), String> {
        let target = self.binding_of_thread(thread_id).await;
        let Some(binding) = target else {
            return Ok(());
        };
        let account_id = binding
            .get("accountId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !account_id.is_empty() {
            self.client.logout(&account_id).await;
        }
        {
            let mut g = self.inner.lock().await;
            g.bindings
                .retain(|b| b.get("threadId").and_then(|v| v.as_str()) != Some(thread_id));
            if !account_id.is_empty() {
                g.started_accounts.remove(&account_id);
                g.account_conn.remove(&account_id);
            }
            if g.pending_bind.as_deref() == Some(thread_id) {
                g.pending_bind = None;
                g.pending_login_id = None;
            }
            if g.bindings.is_empty() {
                g.conn = "offline";
                g.detail = None;
                g.qr_content = None;
            }
        }
        if let Err(e) = self.store.set_wechat(thread_id, None) {
            self.log("warn", format!("落盘解除绑定失败: {e}")).await;
        }
        // 解绑后同步映射：该账号后续附件不再下载（媒体目录不再新增文件）。
        self.sync_client_bindings().await;
        self.log("info", format!("已解除会话 {thread_id} 的微信绑定")).await;
        self.emit_state().await;
        Ok(())
    }

    // -- 协议事件处理 --------------------------------------------------------

    async fn handle_wechat_event(self: &Arc<Self>, ev: WechatEvent) {
        match ev {
            WechatEvent::Qr(url) => {
                let mut g = self.inner.lock().await;
                g.qr_content = Some(url);
                g.conn = "awaiting_qr";
                g.detail = None;
                drop(g);
                self.emit_state().await;
            }
            WechatEvent::LoginResult {
                login_id,
                success,
                message,
                account_id,
                user_id,
            } => {
                // 仅处理当前 pending 登录的结果；被接管/取消的陈旧结果直接忽略。
                {
                    let g = self.inner.lock().await;
                    if g.pending_login_id.as_deref() != Some(login_id.as_str()) {
                        return;
                    }
                }
                if success {
                    // 注意：锁必须取到局部变量后立即释放，不能写进 match scrutinee——
                    // 临时锁会存活到整个 match 结束，分支内再次 lock 会自锁卡死事件循环。
                    let pending_bind = self.inner.lock().await.pending_bind.clone();
                    match (pending_bind, account_id, user_id) {
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
                                let stored_binding = WechatBinding {
                                    account_id: account.clone(),
                                    user_id: Some(user_id),
                                    name: None,
                                    bound_at: u64::try_from(bound_at).ok(),
                                };
                                {
                                    let mut g = self.inner.lock().await;
                                    g.bindings.push(binding);
                                    g.pending_bind = None;
                                    g.pending_login_id = None;
                                    g.qr_content = None;
                                    g.conn = "starting";
                                    g.detail = None;
                                    g.started_accounts.insert(start_account.clone());
                                    g.account_conn.insert(start_account.clone(), "starting".into());
                                }
                                if let Err(e) =
                                    self.store.set_wechat(&thread_id, Some(stored_binding))
                                {
                                    self.log("warn", format!("落盘绑定失败: {e}")).await;
                                }
                                self.log(
                                    "info",
                                    format!("会话 {thread_id} 已绑定微信账号 {start_account}"),
                                )
                                .await;
                                // 先同步映射再启动接收，保证绑定后立即收到的附件能落到该会话目录。
                                self.sync_client_bindings().await;
                                self.client.start_receiver(start_account).await;
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
                    g.pending_login_id = None;
                }
                self.emit_state().await;
            }
            WechatEvent::SessionStatus {
                account_id,
                status,
                error_code: _,
                error_message: _,
            } => {
                {
                    let mut g = self.inner.lock().await;
                    if g.bindings
                        .iter()
                        .any(|b| b.get("accountId").and_then(|v| v.as_str()) == Some(account_id.as_str()))
                    {
                        g.account_conn.insert(account_id.clone(), status.clone());
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
            WechatEvent::Message {
                account_id,
                from,
                to: _,
                timestamp: _,
                context_token: _,
                text,
                images,
                files,
                media_error,
            } => {
                if let Some(err) = media_error.as_deref() {
                    self.log("warn", format!("微信附件接收失败（{from}）: {err}")).await;
                }
                let thread_id = {
                    let g = self.inner.lock().await;
                    find_binding_for_message(&g.bindings, &account_id, &from)
                        .and_then(|b| b.get("threadId").and_then(|v| v.as_str()))
                        .map(str::to_string)
                };
                let Some(thread_id) = thread_id else {
                    // 「谁扫谁白」门禁：非绑定账号/非本人消息仅首次记一条日志，防刷屏。
                    let key = format!("{account_id}|{from}");
                    let first_ignore = {
                        let mut g = self.inner.lock().await;
                        g.ignored_logged.insert(key.clone())
                    };
                    if first_ignore {
                        self.log(
                            "info",
                            format!("忽略非绑定消息：{from}（账号 {account_id} 未绑定该会话）"),
                        )
                        .await;
                    }
                    return;
                };
                // 无文本无附件（如语音等无载荷媒体）：仅当附件接收失败时回一次提示，其余静默忽略。
                if !has_turn_payload(text.as_deref(), &images, &files) {
                    if let Some(err) = media_error {
                        self.send_reply_now(&account_id, &from, &format!("⚠️ 附件接收失败：{err}"))
                            .await;
                    }
                    return;
                }
                self.dispatch_inbound(InboundMessage {
                    account_id,
                    thread_id,
                    from,
                    text,
                    images,
                    files,
                });
            }
            WechatEvent::Accounts(list) => {
                // 重启复登：对所有已绑定且可启动的账号逐个恢复接收。
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
                    self.client.start_receiver(id).await;
                }
                self.emit_state().await;
            }
            WechatEvent::Error { message, kind } => {
                self.log("warn", format!("微信协议错误（{kind}）: {message}")).await;
                let mut g = self.inner.lock().await;
                g.detail = Some(message);
                // 扫码绑定流程中的错误（pending 非空）：清掉待绑定目标与二维码，
                // 让弹窗按钮恢复可用、可重新发起扫码；已绑定账号的运行时错误不受影响。
                if g.pending_bind.is_some() {
                    g.pending_bind = None;
                    g.pending_login_id = None;
                    g.qr_content = None;
                }
            }
        }
    }

    /// 入站消息分派：同线程串行、线程间并行（一个长回合不再挡住其它绑定会话）。
    /// 立即返回，实际处理在独立任务里排队等该线程的串行锁。
    fn dispatch_inbound(self: &Arc<Self>, msg: InboundMessage) {
        let this = Arc::clone(self);
        tauri::async_runtime::spawn(async move { this.handle_inbound(msg).await });
    }

    /// 取该线程的串行锁（懒创建）：同线程的「转向 / 开新回合」按到达顺序执行。
    async fn thread_lock(self: &Arc<Self>, thread_id: &str) -> Arc<Mutex<()>> {
        let mut g = self.inner.lock().await;
        g.thread_locks
            .entry(thread_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// 处理一条入站消息：该线程有进行中回合则 steer 并入，否则开新回合。
    async fn handle_inbound(self: &Arc<Self>, msg: InboundMessage) {
        if !self.inner.lock().await.alive {
            return; // 客户端未启用，静默丢弃（重启后新消息自然流入）
        }
        let lock = self.thread_lock(&msg.thread_id).await;
        let _guard = lock.lock().await;
        // 持锁期间该线程不会有人在建回合，因此取到的 turn_id 要么为空（无回合）、
        // 要么是已完成 turn/start 的真实 id。
        let active = self.active_turn_id(&msg.thread_id).await;
        // `turn/steer` 的 `expectedTurnId` 是协议必填项：id 未知时（turn/start 响应
        // 未带 id 的防御性分支）短等由 `turn/started` 回填。
        let active = match active {
            Some(None) => self.wait_turn_id(&msg.thread_id).await,
            other => other,
        };
        match active {
            Some(Some(id)) => self.steer_or_restart(&msg, &id).await,
            // 回合已在等待期间结束（或从未开始）：按普通消息另开新回合。
            None => {
                if let Err((account, peer, fail)) = self.begin_turn(msg).await {
                    self.send_reply_now(&account, &peer, &fail).await;
                }
            }
            // 仍在回合中但 id 始终未知：不另开回合（避免同线程并发两个回合），如实回提示。
            Some(None) => {
                self.send_reply_now(
                    &msg.account_id,
                    &msg.from,
                    "⚠️ 执行失败：无法确认当前回合，请稍后重试",
                )
                .await;
            }
        }
    }

    /// 该线程是否有进行中回合：`None` = 空闲；`Some(None)` = 有回合但 turn_id 未知；
    /// `Some(Some(id))` = 有回合且 id 已确定。
    async fn active_turn_id(&self, thread_id: &str) -> Option<Option<String>> {
        self.inner
            .lock()
            .await
            .active
            .get(thread_id)
            .map(|t| (!t.turn_id.is_empty()).then(|| t.turn_id.clone()))
    }

    /// 短等 turn_id 由 `turn/started` 回填；语义与 `active_turn_id` 一致。
    async fn wait_turn_id(&self, thread_id: &str) -> Option<Option<String>> {
        for _ in 0..STEER_TURN_ID_WAIT_ATTEMPTS {
            tokio::time::sleep(Duration::from_millis(STEER_TURN_ID_WAIT_STEP_MS)).await;
            match self.active_turn_id(thread_id).await {
                Some(Some(id)) => return Some(Some(id)),
                Some(None) => continue,
                None => return None, // 回合已结束/被清理：交给上层另开新回合
            }
        }
        Some(None)
    }

    /// 把消息并入该线程进行中的回合：steer 撞上「尚未可转向」窗口时短重试；
    /// 确认无活动回合则另开新回合；不可转向（review/compact）或其它错误回失败提示。
    async fn steer_or_restart(self: &Arc<Self>, msg: &InboundMessage, turn_id: &str) {
        let mut outcome = SteerOutcome::NoActiveTurn;
        let mut last_err = String::new();
        for attempt in 0..STEER_RETRY_ATTEMPTS {
            match self.steer_turn(msg, turn_id).await {
                Ok(()) => return, // 成功并入当前回合：静默，不回执
                Err((SteerOutcome::NoActiveTurn, _)) => {
                    outcome = SteerOutcome::NoActiveTurn;
                    last_err = "未找到可转向的活动回合".into();
                    // 服务端在 turn/start 响应与 turn/started 之间可能尚未把回合标记为
                    // 可转向；短暂重试几次，避免消息因竞态被判成「回合已结束」。
                    if attempt + 1 < STEER_RETRY_ATTEMPTS {
                        tokio::time::sleep(Duration::from_millis(STEER_RETRY_DELAY_MS)).await;
                        continue;
                    }
                }
                Err((kind, err)) => {
                    outcome = kind;
                    last_err = err;
                }
            }
            break;
        }
        match outcome {
            // 回合确已结束（或竞态未命中）：按普通消息另开新回合。
            SteerOutcome::NoActiveTurn => {
                if let Err((account, peer, fail)) = self.begin_turn(msg.clone()).await {
                    self.send_reply_now(&account, &peer, &fail).await;
                }
            }
            _ => {
                self.log(
                    "warn",
                    format!("转向失败（会话 {}）: {last_err}", msg.thread_id),
                )
                .await;
                self.send_reply_now(
                    &msg.account_id,
                    &msg.from,
                    &format!("⚠️ 执行失败：{last_err}"),
                )
                .await;
            }
        }
    }

    /// 调 `turn/steer` 把消息并入指定回合；失败按可恢复性分类返回。
    async fn steer_turn(
        self: &Arc<Self>,
        msg: &InboundMessage,
        expected_turn_id: &str,
    ) -> Result<(), (SteerOutcome, String)> {
        let params = build_steer_params(
            &msg.thread_id,
            msg.text.as_deref(),
            &msg.images,
            &msg.files,
            &self.next_message_id(),
            expected_turn_id,
        );
        match self
            .server
            .request("turn/steer", params, Some(Duration::from_secs(60)))
            .await
        {
            Ok(_) => Ok(()),
            Err(e) => Err((classify_steer_error(&e), e)),
        }
    }

    /// 即时系统回复（失败提示等文案，不经回合编排直接发出）。
    async fn send_reply_now(self: &Arc<Self>, account_id: &str, peer: &str, text: &str) {
        for chunk in chunk_text(text, REPLY_CHUNK_CHARS) {
            if let Err(e) = self.client.send_text(account_id, peer, &chunk).await {
                self.log("warn", format!("回复 {peer} 失败: {e}")).await;
            }
        }
    }

    /// 发送“正在输入”状态（best-effort：失败仅告警，绝不阻断回合）。
    async fn set_typing(&self, account_id: &str, peer: &str, status: i32) {
        if let Err(e) = self.client.send_typing(account_id, peer, status).await {
            self.log("warn", format!("发送 typing 状态给 {peer} 失败: {e}")).await;
        }
    }

    /// 启动周期续发“正在输入”的后台任务（首次发送 8s 后开始；靠 abort 停止）。
    fn spawn_typing_refresh(self: &Arc<Self>, account_id: &str, peer: &str) -> tauri::async_runtime::JoinHandle<()> {
        let this = Arc::clone(self);
        let account_id = account_id.to_string();
        let peer = peer.to_string();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(TYPING_INTERVAL_SECS)).await;
                this.set_typing(&account_id, &peer, 1).await;
            }
        })
    }

    // -- Codex 回合编排 ------------------------------------------------------

    /// 订阅 codex 服务器通知：收集 agentMessage 文本，并把 turn/completed 投递给
    /// 等待该线程回合的完成 waiter。不设时间上限——只有 waiter 本身被清理才会提前结束。
    fn spawn_notification_subscription(self: &Arc<Self>) {
        let mut rx = self.server.subscribe_notifications();
        let this = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok((method, params)) => this.on_server_notification(&method, &params).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        this.log("warn", format!("通知订阅滞后，跳过 {n} 条")).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    /// 单条服务器通知的处理：收集回复文本 / 投递回合完成信号。
    async fn on_server_notification(self: &Arc<Self>, method: &str, params: &Value) {
        match method {
            "item/completed" => {
                let is_agent =
                    params.pointer("/item/type").and_then(|t| t.as_str()) == Some("agentMessage");
                if !is_agent {
                    return;
                }
                if let (Some(tid), Some(text)) = (
                    params.get("threadId").and_then(|x| x.as_str()),
                    params.pointer("/item/text").and_then(|x| x.as_str()),
                ) {
                    if text.is_empty() {
                        return;
                    }
                    // 同回合可能多条 agentMessage：保留最后一条完整文本。
                    self.inner
                        .lock()
                        .await
                        .pending_reply
                        .insert(tid.to_string(), text.to_string());
                }
            }
            "turn/completed" => {
                let Some(tid) = params.get("threadId").and_then(|x| x.as_str()) else {
                    return;
                };
                let status = params
                    .pointer("/turn/status")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                // 取真实 turn_id：仅当线程与回合 id 都匹配才结束当前回合，
                // 避免同线程上残留的旧完成信号（后台压缩/重连遗留）误判新回合。
                let turn_id = params
                    .pointer("/turn/id")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let waiter = {
                    let mut g = self.inner.lock().await;
                    match g.active.get_mut(tid) {
                        Some(turn) if is_this_turn_completion(tid, &turn_id, tid, &turn.turn_id) => {
                            turn.done.take()
                        }
                        _ => None,
                    }
                };
                if let Some(tx) = waiter {
                    let _ = tx.send(Completion { turn_id, status });
                }
            }
            "turn/started" => {
                // turn/start 响应偶发未带 id 时，用本通知回填，保证后续 steer 可用。
                let (Some(tid), Some(turn_id)) = (
                    params.get("threadId").and_then(|x| x.as_str()),
                    params.pointer("/turn/id").and_then(|x| x.as_str()),
                ) else {
                    return;
                };
                let mut g = self.inner.lock().await;
                if let Some(turn) = g.active.get_mut(tid) {
                    if turn.turn_id.is_empty() {
                        turn.turn_id = turn_id.to_string();
                    }
                }
            }
            _ => {}
        }
    }

    /// 订阅 app-server 连接状态：断开瞬间停掉所有线程的「正在输入」续发
    /// （不再显示闪烁），但回合本身继续无限等待，不回复、不重试。
    fn spawn_connection_watch(self: &Arc<Self>) {
        let mut rx = self.server.subscribe_connection();
        let this = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(true) => {}
                    Ok(false) => this.abort_all_typing().await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // 滞后可能漏掉断开事件：按当前连接状态兜底判定一次。
                        let connected = this
                            .server
                            .status()
                            .await
                            .get("connected")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        if !connected {
                            this.abort_all_typing().await;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    /// 停掉所有进行中回合的「正在输入」续发任务（回合本身不受影响）。
    async fn abort_all_typing(&self) {
        let mut g = self.inner.lock().await;
        for turn in g.active.values_mut() {
            if let Some(task) = turn.typing_task.take() {
                task.abort();
            }
        }
    }

    /// 等待单个回合完成并回复：无限等（无超时兜底），仅在 waiter 被清理时退出。
    async fn await_turn_completion(
        self: Arc<Self>,
        thread_id: String,
        account_id: String,
        peer: String,
        done: oneshot::Receiver<Completion>,
    ) {
        let Ok(completion) = done.await else {
            // 完成通道被丢弃（应用退出 / 回合被清理）：不回复。
            return;
        };
        let (turn_id, task) = {
            let mut g = self.inner.lock().await;
            let removed = g.active.remove(&thread_id);
            let turn_id = removed
                .as_ref()
                .map(|t| t.turn_id.clone())
                .unwrap_or_else(|| completion.turn_id.clone());
            (turn_id, removed.and_then(|t| t.typing_task))
        };
        if let Some(t) = task {
            t.abort();
        }
        let captured = self
            .inner
            .lock()
            .await
            .pending_reply
            .remove(&thread_id)
            .unwrap_or_default();
        // 通知订阅可能漏抓 item/completed(agentMessage)：为空时回查线程取回复文本兜底。
        let final_text = if captured.trim().is_empty() {
            let fetched = self.fetch_turn_agent_text(&thread_id, &turn_id).await;
            let got = fetched.as_deref().is_some_and(|s| !s.trim().is_empty());
            if got {
                self.log(
                    "info",
                    format!("回合 {thread_id} 通知漏抓，已回查取到回复文本"),
                )
                .await;
            } else {
                self.log(
                    "warn",
                    format!(
                        "回合 {thread_id} 结束状态={} 未捕获到 agentMessage 文本",
                        completion.status
                    ),
                )
                .await;
            }
            final_reply_text(&captured, &completion.status, fetched)
        } else {
            final_reply_text(&captured, &completion.status, None)
        };
        self.send_reply_now(&account_id, &peer, &final_text).await;
        self.set_typing(&account_id, &peer, 2).await;
    }

    /// 回合开始：在绑定线程上发起 turn/start；失败时返回 (账号, 联系人, 失败文案)。
    async fn begin_turn(
        self: &Arc<Self>,
        job: InboundMessage,
    ) -> Result<(), (String, String, String)> {
        let peer = job.from.clone();
        let account = job.account_id.clone();
        let thread_id = job.thread_id.clone();
        match self
            .run_turn_on_thread(
                &account,
                &peer,
                &thread_id,
                job.text.as_deref(),
                &job.images,
                &job.files,
            )
            .await
        {
            Ok(()) => Ok(()),
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

    /// 在确认的线程上发起回合：恢复(激活)线程 → 解析默认模型 → 组装参数 → turn/start
    /// → 登记完成 waiter 并交给独立任务等待/回复。
    /// 失败返回 (peer, 已格式化文案)；该文案用于识别「thread not found」以触发解绑。
    async fn run_turn_on_thread(
        self: &Arc<Self>,
        account_id: &str,
        peer: &str,
        thread_id: &str,
        text: Option<&str>,
        images: &[String],
        files: &[String],
    ) -> Result<(), (String, String)> {
        // 纠偏：清除上一轮积存的回复文本，避免本轮无 agentMessage 时复用旧回复。
        self.inner.lock().await.pending_reply.remove(thread_id);
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
            return Err((peer.to_string(), format!("⚠️ 执行失败：{e}")));
        }
        // 与定时任务 execute 一致：优先使用会话保存的 model，缺省才回退默认模型。
        let session = self.store.get(thread_id);
        let saved_model = session.as_ref().and_then(|s| s.model.clone());
        let model = match saved_model {
            Some(m) if !m.is_empty() => m,
            _ => match self.resolve_default_model().await {
                Ok(m) => m,
                Err(e) => {
                    self.log("warn", e).await;
                    String::new()
                }
            },
        };
        if model.is_empty() {
            return Err((
                peer.to_string(),
                "⚠️ 执行失败：无法解析默认模型（检查 codex 登录与模型列表）".into(),
            ));
        }
        let effort = session.as_ref().and_then(|s| s.effort.as_deref());
        // 回合即将真正开始：点亮“正在输入”并周期续发。
        let typing_task = self.spawn_typing_refresh(account_id, peer);
        self.set_typing(account_id, peer, 1).await;
        let params = build_turn_params(
            thread_id,
            text,
            images,
            files,
            &model,
            effort,
            &self.next_message_id(),
        );
        let turn_id = match self
            .server
            .request("turn/start", params, Some(Duration::from_secs(60)))
            .await
        {
            Ok(resp) => resp
                .pointer("/turn/id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            Err(e) => {
                typing_task.abort();
                self.set_typing(account_id, peer, 2).await;
                return Err((peer.to_string(), format!("⚠️ 执行失败：{e}")));
            }
        };
        // 登记完成 waiter：turn/start 返回后已知真实 turn_id，此后到达的完成通知
        // 才会被 `on_server_notification` 投递（turn/start 响应必然早于该回合任何通知）。
        let (done_tx, done_rx) = oneshot::channel();
        let stale = self.inner.lock().await.active.insert(
            thread_id.to_string(),
            ActiveTurn {
                turn_id,
                done: Some(done_tx),
                typing_task: Some(typing_task),
            },
        );
        if let Some(stale) = stale {
            // 覆盖了一条旧回合（旧完成信号永不到达时的自愈路径）：停掉它遗留的
            // 续发任务、丢弃其完成口（旧 waiter 因此退出且不回复）。
            if let Some(t) = stale.typing_task {
                t.abort();
            }
        }
        tauri::async_runtime::spawn(Self::await_turn_completion(
            Arc::clone(self),
            thread_id.to_string(),
            account_id.to_string(),
            peer.to_string(),
            done_rx,
        ));
        Ok(())
    }

    /// 回查线程：取指定回合的最后一条非空 `agentMessage` 文本（best-effort，失败返回 None）。
    async fn fetch_turn_agent_text(&self, thread_id: &str, turn_id: &str) -> Option<String> {
        // turn/completed 刚下发时回查可能尚未落库，短重试 2 次（各 200ms）防误报无输出。
        for attempt in 0..2 {
            let resp = self
                .server
                .request(
                    "thread/turns/list",
                    json!({
                        "threadId": thread_id,
                        "limit": 5,
                        "sortDirection": "desc",
                        "itemsView": "full",
                    }),
                    Some(Duration::from_secs(5)),
                )
                .await
                .ok()?;
            if let Some(text) = extract_turn_agent_text(&resp, turn_id) {
                return Some(text);
            }
            if attempt == 0 {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
        None
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
        assert!(binding_conflict(list, "t-1", "bot-2"));
        assert!(binding_conflict(list, "t-2", "bot-1"));
        assert!(!binding_conflict(list, "t-2", "bot-2"));
    }

    #[test]
    fn find_binding_for_message_routes_by_account_and_owner() {
        let bindings = json!([
            { "threadId": "t-1", "accountId": "bot-1", "userId": "wx_me" },
            { "threadId": "t-2", "accountId": "bot-2", "userId": "wx_other" },
        ]);
        let list = bindings.as_array().unwrap();
        let hit = find_binding_for_message(list, "bot-1", " wx_me ");
        assert_eq!(
            hit.and_then(|b| b.get("threadId").and_then(|v| v.as_str())),
            Some("t-1")
        );
        assert!(find_binding_for_message(list, "bot-1", "wx_friend").is_none());
        assert!(find_binding_for_message(list, "bot-9", "wx_me").is_none());
    }

    #[test]
    fn steer_error_classification() {
        // 回合不在（尚未可转向 / 已结束）：应另开新回合
        assert_eq!(
            classify_steer_error("no active turn to steer"),
            SteerOutcome::NoActiveTurn
        );
        assert_eq!(
            classify_steer_error("No Active Turn"),
            SteerOutcome::NoActiveTurn
        );
        // 不可转向的回合类型（review / 手动 compact）：应回失败提示，不另开回合
        assert_eq!(
            classify_steer_error("active turn not steerable"),
            SteerOutcome::NotSteerable
        );
        assert_eq!(
            classify_steer_error("activeTurnNotSteerable { turnKind: review }"),
            SteerOutcome::NotSteerable
        );
        // 其它错误（连接中断等）：回失败提示
        assert_eq!(
            classify_steer_error("codex app-server 未连接，正在重连，请稍候"),
            SteerOutcome::Other
        );
        assert_eq!(classify_steer_error(""), SteerOutcome::Other);
    }

    #[test]
    fn steer_params_carry_expected_turn_and_input() {
        let images = vec!["C:\\tmp\\a.png".to_string()];
        let v = build_steer_params(
            "t-1",
            Some("补充要求"),
            &images,
            &[],
            "wechat-1-0",
            "turn-9",
        );
        assert_eq!(v["threadId"], "t-1");
        assert_eq!(v["expectedTurnId"], "turn-9");
        assert_eq!(v["clientUserMessageId"], "wechat-1-0");
        assert_eq!(v["input"][0]["type"], "localImage");
        assert_eq!(v["input"][0]["path"], "C:/tmp/a.png");
        assert_eq!(v["input"][1]["type"], "text");
        assert_eq!(v["input"][1]["text"], "补充要求");
        // steer 不携带权限/模型字段（沿用回合既有设置）
        assert!(v.get("sandboxPolicy").is_none());
        assert!(v.get("model").is_none());
    }

    #[test]
    fn chunk_text_splits_utf8_safely() {
        let text = "中文a🦞".repeat(700);
        let chunks = chunk_text(&text, REPLY_CHUNK_CHARS);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks.concat(), text);
        assert_eq!(chunk_text("", 10), vec![String::new()]);
        assert_eq!(chunk_text("hi", 10).len(), 1);
    }

    #[test]
    fn turn_params_carry_never_policy_and_default_collab() {
        let v = build_turn_params(
            "t-1",
            Some("hello"),
            &[],
            &[],
            "gpt-x",
            Some("high"),
            "wechat-1-0",
        );
        assert_eq!(v["threadId"], "t-1");
        assert_eq!(v["input"][0]["type"], "text");
        assert_eq!(v["input"][0]["text"], "hello");
        assert_eq!(v["clientUserMessageId"], "wechat-1-0");
        assert_eq!(v["approvalPolicy"], "never");
        assert_eq!(v["sandboxPolicy"]["type"], "dangerFullAccess");
        assert_eq!(v["model"], "gpt-x");
        assert_eq!(v["effort"], "high");
        assert_eq!(v["collaborationMode"]["mode"], "default");
        assert_eq!(v["collaborationMode"]["settings"]["model"], "gpt-x");
        assert_eq!(v["collaborationMode"]["settings"]["reasoning_effort"], "high");
    }

    #[test]
    fn turn_params_always_danger_full_access() {
        let v = build_turn_params("t-1", Some("hello"), &[], &[], "gpt-x", None, "wechat-1-0");
        assert_eq!(v["approvalPolicy"], "never");
        assert_eq!(v["sandboxPolicy"]["type"], "dangerFullAccess");
        assert!(
            v["sandboxPolicy"].get("writableRoots").is_none(),
            "danger-full-access 变体不应带 writableRoots"
        );
        assert!(
            v["sandboxPolicy"].get("networkAccess").is_none(),
            "danger-full-access 变体不应带 networkAccess"
        );
        assert_eq!(v["effort"], serde_json::Value::Null);
        assert_eq!(v["collaborationMode"]["settings"]["reasoning_effort"], serde_json::Value::Null);
    }

    #[test]
    fn turn_input_orders_images_and_carries_files_in_text_section() {
        let images = vec!["C:\\tmp\\wechat\\media\\t-1\\wechat-img-1.png".to_string()];
        let files = vec![
            "C:\\tmp\\wechat\\media\\t-1\\预算表.xlsx".to_string(),
            "C:\\tmp\\wechat\\media\\t-1\\wechat-video-1.mp4".to_string(),
        ];
        // 图片 + 文件 + 文本：图片在前，文件走文本段，路径转正斜杠
        let v = build_turn_params(
            "t-1",
            Some("看附件"),
            &images,
            &files,
            "gpt-x",
            None,
            "wechat-1-0",
        );
        let input = v["input"].as_array().unwrap();
        assert_eq!(input.len(), 2, "只应有 localImage + text 两项");
        assert_eq!(input[0]["type"], "localImage");
        assert_eq!(input[0]["path"], "C:/tmp/wechat/media/t-1/wechat-img-1.png");
        assert_eq!(input[1]["type"], "text");
        let body = input[1]["text"].as_str().unwrap();
        assert!(
            body.starts_with(&format!("\n{FILE_MENTION_HEADING}")),
            "文本应以文件段开头: {body}"
        );
        assert!(body.contains("## 预算表.xlsx: C:/tmp/wechat/media/t-1/预算表.xlsx"), "{body}");
        assert!(
            body.contains("## wechat-video-1.mp4: C:/tmp/wechat/media/t-1/wechat-video-1.mp4"),
            "{body}"
        );
        assert!(body.ends_with("看附件"), "用户文本应在末尾: {body}");
        assert!(body.contains(MY_REQUEST_MARKER), "{body}");

        // 纯附件（文件 + 视频，无文字）：带文件段 + 占位说明
        let v = build_turn_params("t-1", None, &[], &files, "gpt-x", None, "wechat-1-0");
        let input = v["input"].as_array().unwrap();
        assert_eq!(input.len(), 1, "只有 text 项");
        assert_eq!(input[0]["type"], "text");
        let body = input[0]["text"].as_str().unwrap();
        assert!(body.contains("## 预算表.xlsx: C:/tmp/wechat/media/t-1/预算表.xlsx"), "{body}");
        assert!(body.contains(MY_REQUEST_MARKER), "{body}");
        assert!(body.ends_with(BARE_ATTACHMENT_NOTE), "{body}");

        // 纯图片：只有 localImage 项，不带文本
        let v = build_turn_params("t-1", None, &images, &[], "gpt-x", None, "wechat-1-0");
        let input = v["input"].as_array().unwrap();
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["type"], "localImage");

        // 仅文本：原样发送
        let v = build_turn_params("t-1", Some("你好"), &[], &[], "gpt-x", None, "wechat-1-0");
        let input = v["input"].as_array().unwrap();
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["text"], "你好");
    }

    /// codex 会丢弃独立的 `mention` 项（曾导致微信文件回合变成空消息），确保不再产生该类型。
    #[test]
    fn turn_input_never_emits_mention_items() {
        let images = vec!["C:\\tmp\\a.png".to_string()];
        let files = vec!["C:\\tmp\\b.xlsx".to_string(), "C:\\tmp\\c.mp4".to_string()];
        for (text, imgs, fls) in [
            (Some("看附件"), images.clone(), files.clone()),
            (None, Vec::new(), files.clone()),
            (Some("只有文字"), Vec::new(), Vec::new()),
            (None, images.clone(), Vec::new()),
            (Some("图 + 文件"), images.clone(), files.clone()),
        ] {
            let v = build_turn_params("t-1", text, &imgs, &fls, "gpt-x", None, "wechat-1-0");
            let input = v["input"].as_array().unwrap();
            assert!(
                input.iter().all(|i| i["type"] != "mention"),
                "不应再产生 mention 项: {input:?}"
            );
            assert!(!input.is_empty(), "input 不应为空: text={text:?}");
        }
    }

    #[test]
    fn turn_payload_requires_text_or_attachment() {
        let img = vec!["D:/a.png".to_string()];
        let file = vec!["D:/b.xlsx".to_string()];
        assert!(has_turn_payload(None, &img, &[]));
        assert!(has_turn_payload(None, &[], &file));
        assert!(has_turn_payload(Some("你好"), &[], &[]));
        assert!(!has_turn_payload(None, &[], &[]));
        assert!(!has_turn_payload(Some("   "), &[], &[]));
    }

    #[test]
    fn normalize_turn_text_falls_back_for_empty_results() {
        assert_eq!(normalize_turn_text(" ok ", "completed"), "ok");
        assert!(normalize_turn_text("", "interrupted").contains("已中断"));
        assert!(!normalize_turn_text("", "completed").contains("执行失败"));
        assert!(normalize_turn_text("", "completed").contains("没有输出"));
        assert!(normalize_turn_text("", "").contains("未返回"));
        assert!(normalize_turn_text("", "failed").contains("failed"));
    }

    #[test]
    fn extract_turn_agent_text_returns_last_nonempty_for_turn() {
        let resp = json!({
            "data": [
                {
                    "id": "turn-a",
                    "items": [
                        { "type": "reasoning", "text": "思考" },
                        { "type": "agentMessage", "text": "" },
                        { "type": "agentMessage", "text": "  hello  " },
                        { "type": "agentMessage", "text": "" },
                    ],
                },
                { "id": "turn-b", "items": [ { "type": "agentMessage", "text": "other" } ] },
            ],
            "nextCursor": null,
        });
        assert_eq!(extract_turn_agent_text(&resp, "turn-a").as_deref(), Some("  hello  "));
        assert_eq!(extract_turn_agent_text(&resp, "turn-b").as_deref(), Some("other"));
        assert!(extract_turn_agent_text(&resp, "turn-none").is_none());
        assert!(extract_turn_agent_text(&json!(null), "x").is_none());
        let all_empty = json!({
            "data": [ { "id": "a", "items": [ { "type": "agentMessage", "text": "" } ] } ]
        });
        assert!(extract_turn_agent_text(&all_empty, "a").is_none());
    }

    #[test]
    fn final_reply_text_prefers_captured_then_fetched_else_fallback() {
        assert_eq!(final_reply_text(" hi ", "completed", Some("x".into())), "hi");
        assert_eq!(final_reply_text("", "completed", Some(" 回查文本 ".into())), "回查文本");
        assert!(!final_reply_text("", "completed", None).contains("执行失败"));
        assert!(final_reply_text("", "completed", Some(String::new())).contains("没有输出"));
        assert!(final_reply_text("", "failed", Some(String::new())).contains("failed"));
    }

    #[test]
    fn turn_completion_matches_thread_and_turn() {
        // 线程 + 回合全部匹配 → 命中
        assert!(is_this_turn_completion("t-1", "turn-2", "t-1", "turn-2"));
        // 线程不匹配 → 不命中（他线完成信号）
        assert!(!is_this_turn_completion("t-2", "turn-2", "t-1", "turn-2"));
        // 同线程但旧 turn 完成信号 → 不命中（关键：修复误判）
        assert!(!is_this_turn_completion("t-1", "turn-old", "t-1", "turn-2"));
    }

    #[test]
    fn turn_completion_falls_back_to_thread_when_turn_id_missing() {
        // 当前回合被记录的 turn_id 缺省 → 仅按线程匹配（不误判永不完成）
        assert!(is_this_turn_completion("t-1", "turn-x", "t-1", ""));
        // 完成信号缺 turn_id → 命中当前回合
        assert!(is_this_turn_completion("t-1", "", "t-1", "turn-2"));
        // 两者都非空且不相等 → 不命中
        assert!(!is_this_turn_completion("t-1", "turn-a", "t-1", "turn-b"));
    }

    #[test]
    fn wechat_bindings_persist_via_session_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::codex::session_state::SessionStateStore::new(dir.path()).unwrap();
        store
            .set_wechat(
                "t-a",
                Some(crate::codex::session_state::WechatBinding {
                    account_id: "bot-a".into(),
                    user_id: Some("u-a".into()),
                    name: None,
                    bound_at: Some(1),
                }),
            )
            .unwrap();
        store
            .set_wechat(
                "t-b",
                Some(crate::codex::session_state::WechatBinding {
                    account_id: "bot-b".into(),
                    user_id: Some("u-b".into()),
                    name: None,
                    bound_at: Some(2),
                }),
            )
            .unwrap();
        let b = store.wechat_bindings();
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].get("threadId").and_then(|v| v.as_str()), Some("t-a"));
        store.set_wechat("t-a", None).unwrap();
        assert_eq!(store.wechat_bindings().len(), 1);
    }

    #[test]
    fn thread_not_found_detection() {
        assert!(is_thread_not_found(
            "⚠️ 执行失败：thread not found: 01a04189-376d-7660-a4a3-94afbe4b9d5d"
        ));
        assert!(is_thread_not_found("thread not found: abc"));
        assert!(is_thread_not_found("Thread Not Found: abc"));
        assert!(is_thread_not_found("thread does not exist: abc"));
        assert!(!is_thread_not_found("⚠️ 执行失败：model not found"));
        assert!(!is_thread_not_found("⚠️ 执行失败：file not found: x"));
        assert!(!is_thread_not_found(""));
    }
}
