use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, Notify, broadcast, oneshot};

use crate::codex::bundled;
use crate::codex::env_flags;
use crate::codex::path_util::clean_path;
use crate::codex::model_config;
use crate::codex::logs_guard;
use crate::codex::settings::{self, AppSettings};
use crate::codex::session_log::SessionLog;
use crate::codex::zen_proxy::{self, ZenProxyHandle};
use crate::codex::zen_trace::{TraceSink, ZenTrace};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const MAX_LOG_LINES: usize = 500;
/// 内置插件市场名（保留名）；来源为 codex 认可的 canonical 位置，由后台 `bundled::bootstrap`
/// 解压到该位置，Rust 侧只注册（`marketplace/add`）、不复制。
pub(crate) const BUNDLED_MARKETPLACES: [&str; 2] = ["openai-bundled", "openai-primary-runtime"];

/// JSON-RPC 错误；`code` 为 `None` 时表示本地传输层错误（未就绪/超时/断连）。
#[derive(Debug, Clone)]
pub struct RpcError {
    pub code: Option<i64>,
    pub message: String,
}

pub struct CodexServer {
    app: AppHandle,
    shared: Arc<Shared>,
    next_id: AtomicU64,
    workspace: PathBuf,
    run_started: AtomicBool,
    log: Option<Arc<SessionLog>>,
    /// codex app-server 的 error/warning 消息日志（`codex-YYYY-MM-DD.log`）。
    codex_log: Option<Arc<SessionLog>>,
    /// Zen 代理内容诊断日志（`logs/zen/`，常开）；无日志目录时为 None。
    zen_trace: Option<Arc<ZenTrace>>,
}

struct Shared {
    inner: Mutex<Inner>,
    stop: AtomicBool,
    bundled: DoneState,
    runtime: DoneState,
    /// 服务器通知广播口：除转发前端外，微信桥等 Rust 内部订阅者据此感知
    /// thread/turn 事件（容量有限，滞后订阅者按 Lagged 自行容错）。
    tap: broadcast::Sender<(String, Value)>,
}

/// 后台完成信号：`wait` 会阻塞到 `mark_done` 被调用，避免市场登记在
/// 解压/下载完成前即因「来源未就绪」被跳过。
struct DoneState {
    done: AtomicBool,
    notify: Notify,
}

impl DoneState {
    fn new() -> Self {
        Self {
            done: AtomicBool::new(false),
            notify: Notify::new(),
        }
    }

    fn mark_done(&self) {
        self.done.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    async fn wait(&self) {
        while !self.done.load(Ordering::SeqCst) {
            self.notify.notified().await;
        }
    }
}

struct Inner {
    stdin: Option<ChildStdin>,
    child: Option<Child>,
    pending: HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>,
    connected: bool,
    ready: bool,
    logs: Vec<String>,
    codex_path: Option<PathBuf>,
    /// 启动后探测到的 codex 版本号（`codex --version` 输出）；未探测为 None。
    codex_version: Option<String>,
    /// 版本是否低于 0.149.0（仅低版本警告）；未探测为 None。
    version_too_old: Option<bool>,
    /// Zen 本地代理运行句柄（设置开启且启动成功时为 Some）。
    zen_proxy: Option<ZenProxyHandle>,
}

impl Inner {
    fn new() -> Self {
        Self {
            stdin: None,
            child: None,
            pending: HashMap::new(),
            connected: false,
            ready: false,
            logs: Vec::new(),
            codex_path: None,
            codex_version: None,
            version_too_old: None,
            zen_proxy: None,
        }
    }

    fn push_log_locked(&mut self, line: String) {
        self.logs.push(line);
        if self.logs.len() > MAX_LOG_LINES {
            let excess = self.logs.len() - MAX_LOG_LINES;
            self.logs.drain(0..excess);
        }
    }
}

/// Zen 代理内容诊断日志句柄：开关关闭（默认）或没有日志目录时不构造——
/// 既不建目录也不写入，更不会清理已有文件（用户需要时手动删或重新开启）。
fn zen_trace_from(dir: Option<PathBuf>, enabled: bool) -> Option<Arc<ZenTrace>> {
    if !enabled {
        return None;
    }
    dir.map(|d| Arc::new(ZenTrace::new(d)))
}

impl CodexServer {
    pub fn new(app: AppHandle, workspace: PathBuf) -> Self {
        let logs_dir = app
            .path()
            .app_data_dir()
            .ok()
            .map(|d| d.join("logs"));
        let log = logs_dir
            .as_ref()
            .map(|d| Arc::new(SessionLog::new(d.clone())));
        let codex_log = logs_dir
            .as_ref()
            .map(|d| Arc::new(SessionLog::with_prefix(d.clone(), "codex-")));
        // Zen 代理内容诊断日志（独立子目录，便于单独清理与打包分析）：默认关闭，
        // 需要排查时设 `CODEXUI_ZEN_TRACE=1` 并重启 codex-ui（环境变量只在启动时读取一次）。
        if let (Some(raw), Some(log)) = (env_flags::invalid_value("ZEN_TRACE"), log.as_ref()) {
            log.write(
                "warn",
                None,
                "env.flag_invalid",
                &[
                    ("name".to_string(), env_flags::full_name("ZEN_TRACE")),
                    ("value".to_string(), raw),
                    (
                        "detail".to_string(),
                        "取值无法识别，按关闭处理；真值为 1/true/on/yes".to_string(),
                    ),
                ],
            );
        }
        let zen_trace = zen_trace_from(
            logs_dir.as_ref().map(|d| d.join("zen")),
            env_flags::flag("ZEN_TRACE"),
        );
        Self {
            app,
            shared: Arc::new(Shared {
                inner: Mutex::new(Inner::new()),
                stop: AtomicBool::new(false),
                bundled: DoneState::new(),
                runtime: DoneState::new(),
                tap: broadcast::channel(256).0,
            }),
            next_id: AtomicU64::new(0),
            workspace,
            run_started: AtomicBool::new(false),
            log,
            codex_log,
            zen_trace,
        }
    }

    /// 订阅 app-server 的全部服务器通知（method, params）。
    /// 当前仅微信桥使用；迟到导致 Lagged 时由订阅方自行跳过补齐。
    pub fn subscribe_notifications(&self) -> broadcast::Receiver<(String, Value)> {
        self.shared.tap.subscribe()
    }

    /// 后台解压完成信号：向等待方广播（供 `bundled::bootstrap` 调用）。
    pub(crate) fn bundled_mark_done(&self) {
        self.shared.bundled.mark_done();
    }

    /// 等待内置插件市场后台解压结束。
    pub(crate) async fn bundled_wait(&self) {
        self.shared.bundled.wait().await;
    }

    /// 运行时按需下载/升级完成信号：向等待方广播（供 `bundled::bootstrap` 调用）。
    pub(crate) fn runtime_mark_done(&self) {
        self.shared.runtime.mark_done();
    }

    /// 等待运行时下载/升级结束。
    pub(crate) async fn runtime_wait(&self) {
        self.shared.runtime.wait().await;
    }

    /// Spawns the background lifecycle task exactly once.
    pub fn ensure_running(self: &Arc<Self>) {
        if !self.run_started.swap(true, Ordering::SeqCst) {
            let this = self.clone();
            tauri::async_runtime::spawn(async move { this.run().await });
        }
    }

    pub fn shutdown(&self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        let mut inner = self.shared.inner.blocking_lock();
        if let Some(child) = inner.child.as_mut() {
            let _ = child.start_kill();
        }
        if let Some(h) = inner.zen_proxy.take() {
            h.stop();
        }
    }

    /// 写一条日志文件条目；日志目录不可用/写盘失败时静默忽略。
    fn log_file(&self, level: &str, thread: Option<&str>, event: &str, kv: &[(String, String)]) {
        if let Some(log) = &self.log {
            log.write(level, thread, event, kv);
        }
    }

    /// 记录带会话归属的 RPC 事件；跳过高频流式增量事件（如 textDelta）避免日志被淹没。
    fn log_event(&self, event: &str, method: &str, params: &Value) {
        if is_streaming_delta(event) {
            return;
        }
        let mut kv = SessionLog::summarize_params(method, params);
        let thread = take_thread(&mut kv);
        self.log_file("info", thread.as_deref(), event, &kv);
    }

    /// 前端用户动作日志（仅允许安全字段；调用方负责不传敏感内容）。
    pub fn session_log(
        &self,
        level: String,
        thread_id: Option<String>,
        event: String,
        detail: Option<String>,
    ) {
        let level = if matches!(level.as_str(), "info" | "warn" | "error") {
            level
        } else {
            "info".into()
        };
        let kv = detail
            .map(|d| vec![("msg".to_string(), d)])
            .unwrap_or_default();
        self.log_file(&level, thread_id.as_deref(), &event, &kv);
    }

    /// 记录并转发 codex 的 error/warning 通知：
    /// 两类消息都写 `codex-YYYY-MM-DD.log`；DEBUG 构建向前端发送全部，
    /// Release 构建只发送 error（warning 仅落盘）。
    fn log_codex_message(&self, method: &str, params: &Value) {
        let Some(level) = codex_message_level(method) else {
            return;
        };
        let message = codex_message_text(method, params);
        let thread = params.get("threadId").and_then(|v| v.as_str());
        let turn_id = params.get("turnId").and_then(|v| v.as_str());
        let error_info = params.pointer("/error/codexErrorInfo").cloned();
        let mut kv = vec![("message".to_string(), message.clone())];
        if let Some(turn_id) = turn_id {
            kv.push(("turnId".to_string(), turn_id.to_string()));
        }
        if let Some(info) = &error_info {
            kv.push(("codexErrorInfo".to_string(), json_value_text(info)));
        }
        if let Some(log) = &self.codex_log {
            log.write(level, thread, method, &kv);
        }
        if !should_emit_codex_message(level, cfg!(debug_assertions)) {
            return;
        }
        let mut payload = json!({
            "level": level,
            "method": method,
            "message": message,
        });
        if let Some(thread) = thread {
            payload["threadId"] = json!(thread);
        }
        if let Some(turn_id) = turn_id {
            payload["turnId"] = json!(turn_id);
        }
        if let Some(info) = error_info {
            payload["codexErrorInfo"] = info;
        }
        let _ = self.app.emit("codex/message", payload);
    }

    async fn run(self: Arc<Self>) {
        // 后台一次性解压内置插件市场（仅当完成标记缺失且归档存在时），结束后通知登记逻辑。
        let boot = self.clone();
        tauri::async_runtime::spawn(async move {
            bundled::bootstrap(boot).await;
        });
        // 按保存设置启动 Zen 本地代理（app-server 解码前确保端口可用；失败仅记日志不阻断）。
        let app_dir = self
            .app
            .path()
            .app_data_dir()
            .unwrap_or_default();
        let settings = settings::load(&app_dir);
        let status = self
            .apply_zen_proxy(
                settings.zen_proxy_enabled,
                settings.zen_proxy_port,
                settings.zen_proxy_base_url,
            )
            .await;
        if !status.running {
            if let Some(e) = status.error {
                self.push_log("warn", format!("Zen 本地代理未启动：{e}")).await;
            }
        } else {
            self.push_log(
                "info",
                format!("Zen 本地代理已启动，端口 {}", status.port),
            )
            .await;
        }
        while !self.shared.stop.load(Ordering::SeqCst) {
            if let Err(e) = self.spawn_and_read().await {
                self.push_log("error", format!("codex app-server 错误: {e}")).await;
            }
            self.mark_disconnected().await;
            self.emit_status().await;
            for _ in 0..10 {
                if self.shared.stop.load(Ordering::SeqCst) {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }

    /// 应用 Zen 代理开关、端口与上游地址：按需启动/停止/重启，返回最新状态。
    pub(crate) async fn apply_zen_proxy(
        &self,
        enabled: bool,
        port: u16,
        base_url: String,
    ) -> zen_proxy::ZenProxyStatus {
        let mut inner = self.shared.inner.lock().await;
        let trace = match self.zen_trace.as_ref() {
            Some(trace) => TraceSink::new(trace.clone()),
            None => TraceSink::disabled(),
        };
        zen_proxy::apply(
            &mut inner.zen_proxy,
            enabled,
            port,
            base_url,
            self.log.clone(),
            trace,
        )
        .await
    }

    /// Zen 代理当前状态（运行中返回端口；未开启或未启动返回默认端口）。
    pub(crate) async fn zen_proxy_status(&self) -> zen_proxy::ZenProxyStatus {
        let inner = self.shared.inner.lock().await;
        if let Some(h) = &inner.zen_proxy {
            zen_proxy::ZenProxyStatus {
                running: true,
                port: h.port,
                error: None,
            }
        } else {
            zen_proxy::ZenProxyStatus::stopped()
        }
    }

    async fn spawn_and_read(self: &Arc<Self>) -> Result<(), String> {
        let codex = self.resolve_codex().await?;
        let mut cmd = Command::new(&codex);
        cmd.args(["app-server", "--stdio"]);
        cmd.current_dir(&self.workspace);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        apply_codex_env(cmd.as_std_mut());
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("启动 codex app-server 失败: {e}"))?;
        let stdin = child.stdin.take().ok_or_else(|| "无法获取 stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "无法获取 stdout".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "无法获取 stderr".to_string())?;

        let err_logger = {
            let this = Arc::clone(self);
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
                                this.push_log("info", format!("[stderr] {t}")).await;
                            }
                        }
                    }
                }
            })
        };

        // 握手：initialize → 等待响应 → initialized 通知
        let handshake_id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let init_msg = json!({
            "jsonrpc": "2.0",
            "id": handshake_id,
            "method": "initialize",
            "params": {
                "clientInfo": { "name": "codex-ui", "title": "Codex UI", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": { "experimentalApi": true, "requestAttestation": false }
            }
        });
        let mut reader = BufReader::new(stdout);
        {
            let mut stdin = stdin;
            let line = format!("{init_msg}\n");
            stdin
                .write_all(line.as_bytes())
                .await
                .map_err(|e| format!("写入 initialize 失败: {e}"))?;

            let mut init_ok = false;
            let mut line = String::new();
            loop {
                line.clear();
                let n = reader
                    .read_line(&mut line)
                    .await
                    .map_err(|e| format!("读取 initialize 响应失败: {e}"))?;
                if n == 0 {
                    break;
                }
                let t = line.trim();
                if t.is_empty() {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<Value>(t) else {
                    continue;
                };
                if v.get("id").and_then(|x| x.as_u64()) == Some(handshake_id) {
                    if v.get("result").is_some() {
                        init_ok = true;
                    } else {
                        self.push_log("error", format!("initialize 失败: {}", v["error"]))
                            .await;
                    }
                    break;
                }
                // 握手期间到达的通知照常转发
                self.handle_message(v).await;
            }
            if !init_ok {
                return Err("codex app-server initialize 失败".into());
            }
            stdin
                .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"initialized\"}\n")
                .await
                .map_err(|e| format!("发送 initialized 失败: {e}"))?;

            let mut inner = self.shared.inner.lock().await;
            inner.stdin = Some(stdin);
            inner.child = Some(child);
            inner.connected = true;
            inner.ready = true;
            inner.codex_path = Some(codex.clone());
        }
        self.push_log(
            "info",
            format!(
                "已启动 codex app-server（{}），工作目录：{}",
                codex.display(),
                self.workspace.display()
            ),
        )
        .await;
        self.emit_status().await;

        // 后台探测 codex 版本：仅低于 0.149 时警告，不阻断主流程
        let server = self.clone();
        tauri::async_runtime::spawn(async move {
            server.probe_codex_version().await;
        });

        // 连接成功后为 codex 日志库应用阻断触发器（best-effort），
        // 防止 logs_2.sqlite 及其 WAL 因持续写入 TRACE 日志而无限膨胀。
        let server = self.clone();
        tauri::async_runtime::spawn(async move {
            server.apply_logs_guard().await;
        });

        // 初始化成功后隐式尝试添加内置插件市场。codex-cli 0.149 把
        // openai-bundled / openai-primary-runtime 视为保留市场名，marketplace/add
        // 只接受 codex 自己管理位置的来源（而非应用自带副本），因此按固定市场名
        // 解析 canonical 来源并注册；内置市场归档由后台 `bundled::bootstrap` 解压，
        // 来源目录未就绪（归档未随包提供或解压失败）则跳过。Rust 侧只登记、不复制。
        for name in BUNDLED_MARKETPLACES {
            let server = self.clone();
            tauri::async_runtime::spawn(async move {
                // openai-bundled 等随包归档解压；openai-primary-runtime 等运行时下载/升级完成。
                if name == "openai-primary-runtime" {
                    server.runtime_wait().await;
                } else {
                    server.bundled_wait().await;
                }
                let source = match resolve_bundled_marketplace_source(name) {
                    Ok(src) => src,
                    Err(msg) => {
                        server
                            .push_log("warn", format!("跳过内置插件市场 {name}：{msg}"))
                            .await;
                        return;
                    }
                };
                let source_display = source.clone();
                let re_add_source = source.clone();
                match server
                    .request(
                        "marketplace/add",
                        json!({ "source": source }),
                        Some(Duration::from_secs(10)),
                    )
                    .await
                {
                    Ok(_) => {
                        server
                            .push_log(
                                "info",
                                format!("已注册内置插件市场 {name}（来源 {source_display}）"),
                            )
                            .await;
                    }
                    Err(e) => {
                        if is_conflicting_marketplace_source_error(&e) {
                            // config.toml 里同名市场已登记到别的来源（如旧版 codex-ui
                            // 写入的应用目录路径），导致以 canonical 来源注册被拒；
                            // 先移除旧登记，再重试添加。
                            if server
                                .request(
                                    "marketplace/remove",
                                    json!({ "marketplaceName": name }),
                                    Some(Duration::from_secs(10)),
                                )
                                .await
                                .is_ok()
                            {
                                if let Err(e2) = server
                                    .request(
                                        "marketplace/add",
                                        json!({ "source": re_add_source }),
                                        Some(Duration::from_secs(10)),
                                    )
                                    .await
                                {
                                    server
                                        .push_log(
                                            "warn",
                                            format!(
                                                "添加内置插件市场 {name}（移除旧来源后仍失败）：{e2}"
                                            ),
                                        )
                                        .await;
                                } else {
                                    server
                                        .push_log(
                                            "info",
                                            format!("已重新注册内置插件市场 {name}（来源 {source_display}）"),
                                        )
                                        .await;
                                }
                            } else {
                                server
                                    .push_log(
                                        "warn",
                                        format!("移除内置插件市场 {name} 的旧来源登记失败"),
                                    )
                                    .await;
                            }
                        } else {
                            server
                                .push_log(
                                    "warn",
                                    format!("添加内置插件市场 {name} 失败：{e}"),
                                )
                                .await;
                        }
                    }
                }
            });
        }

        let mut line = String::new();
        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .await
                .map_err(|e| format!("读取 app-server 输出失败: {e}"))?;
            if n == 0 {
                break;
            }
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            match serde_json::from_str::<Value>(t) {
                Ok(v) => self.handle_message(v).await,
                Err(e) => {
                    self.push_log("error", format!("无法解析 app-server 消息: {e}"))
                        .await
                }
            }
        }
        err_logger.abort();
        Ok(())
    }

    async fn handle_message(&self, v: Value) {
        // 协议 RequestId 为 string | number：服务端反向请求可能带字符串 id，
        // 必须原样保留（我们的请求 id 恒为 u64 数字，应答按数字匹配 pending）。
        let id = v.get("id").filter(|x| !x.is_null()).cloned();
        if id.is_some() {
            if v.get("method").and_then(|m| m.as_str()).is_some() {
                // server -> client request (approval / user input / elicitation)
                let method = v["method"].as_str().unwrap_or("unknown");
                let params = v.get("params").cloned().unwrap_or(Value::Null);
                self.log_event("server-request", method, &params);
                let _ = self.app.emit(
                    "interaction:request",
                    json!({ "requestId": id, "method": method, "params": params }),
                );
                return;
            }
            // response to one of our requests
            let result = if v.get("error").is_some() {
                let code = v["error"].get("code").and_then(|c| c.as_i64());
                let message = v["error"]
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("codex 请求失败")
                    .to_string();
                Err(RpcError { code, message })
            } else {
                Ok(v.get("result").cloned().unwrap_or(Value::Null))
            };
            if let Some(numeric_id) = id.and_then(|x| x.as_u64()) {
                let mut inner = self.shared.inner.lock().await;
                if let Some(tx) = inner.pending.remove(&numeric_id) {
                    let _ = tx.send(result);
                }
            }
            return;
        }
        if let Some(method) = v.get("method").and_then(|m| m.as_str()) {
            let params = v.get("params").cloned().unwrap_or(Value::Null);
            self.log_event(method, method, &params);
            self.log_codex_message(method, &params);
            let _ = self.shared.tap.send((method.to_string(), params.clone()));
            let _ = self.app.emit(method, params);
        }
    }

    pub async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Option<Duration>,
    ) -> Result<Value, String> {
        self.request_verbose(method, params, timeout)
            .await
            .map_err(|e| e.message)
    }

    /// 保留 JSON-RPC error code 的请求变体，供需要区分错误类型的场景使用。
    /// 外层记录出站请求与响应（含耗时/错误码），内层实现真实收发。
    pub async fn request_verbose(
        &self,
        method: &str,
        params: Value,
        timeout: Option<Duration>,
    ) -> Result<Value, RpcError> {
        let mut kv = SessionLog::summarize_params(method, &params);
        let thread = take_thread(&mut kv);
        kv.insert(0, ("method".to_string(), method.to_string()));
        self.log_file("info", thread.as_deref(), "rpc-request", &kv);
        let start = Instant::now();
        let res = self.request_verbose_inner(method, params, timeout).await;
        let dur_ms = start.elapsed().as_millis().to_string();
        let mut resp_kv = vec![
            ("method".to_string(), method.to_string()),
            ("durMs".to_string(), dur_ms),
        ];
        let level = match &res {
            Ok(_) => {
                resp_kv.push(("ok".to_string(), "true".into()));
                "info"
            }
            Err(e) => {
                resp_kv.push(("ok".to_string(), "false".into()));
                resp_kv.push((
                    "code".to_string(),
                    e.code
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "transport".into()),
                ));
                resp_kv.push(("msg".to_string(), truncate(&e.message, 200)));
                "warn"
            }
        };
        self.log_file(level, thread.as_deref(), "rpc-response", &resp_kv);
        res
    }

    async fn request_verbose_inner(
        &self,
        method: &str,
        params: Value,
        timeout: Option<Duration>,
    ) -> Result<Value, RpcError> {
        // 等待握手完成（初始化失败/断线重连时最多等 10 秒）
        let ready_deadline = Instant::now() + Duration::from_secs(10);
        loop {
            {
                let inner = self.shared.inner.lock().await;
                if inner.ready && inner.connected {
                    break;
                }
            }
            if Instant::now() > ready_deadline {
                return Err(RpcError {
                    code: None,
                    message: "codex app-server 未就绪，请稍后重试".into(),
                });
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        // 服务端入口饱和（-32001）时请求尚未被处理，按协议建议指数退避 + 抖动重试。
        const MAX_OVERLOAD_RETRIES: u32 = 3;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let t = timeout.unwrap_or(Duration::from_secs(30));
        let mut attempt: u32 = 0;
        loop {
            let (tx, rx) = oneshot::channel();
            {
                let mut inner = self.shared.inner.lock().await;
                if !inner.connected {
                    return Err(RpcError {
                        code: None,
                        message: "codex app-server 未连接，正在重连，请稍候".into(),
                    });
                }
                let stdin = inner
                    .stdin
                    .as_mut()
                    .ok_or_else(|| RpcError {
                        code: None,
                        message: "codex app-server stdin 不可用".to_string(),
                    })?;
                let line = format!("{}\n", msg);
                stdin
                    .write_all(line.as_bytes())
                    .await
                    .map_err(|e| RpcError {
                        code: None,
                        message: format!("写入 app-server 失败: {e}"),
                    })?;
                inner.pending.insert(id, tx);
            }
            let err = match tokio::time::timeout(t, rx).await {
                Ok(Ok(Ok(v))) => return Ok(v),
                Ok(Ok(Err(e))) => e,
                Ok(Err(_)) => {
                    // 超时：摘除 pending，避免残留（迟到应答的 tx.send 已被丢弃）
                    self.shared.inner.lock().await.pending.remove(&id);
                    RpcError {
                        code: None,
                        message: format!("请求 {method} 超时"),
                    }
                }
                Err(_) => RpcError {
                    code: None,
                    message: "codex app-server 连接中断".to_string(),
                },
            };
            if err.code == Some(-32001) && attempt < MAX_OVERLOAD_RETRIES {
                attempt += 1;
                let backoff_ms = 200u64 * (1 << attempt);
                let jitter_ms = (id % 100) as u64;
                tokio::time::sleep(Duration::from_millis(backoff_ms + jitter_ms)).await;
                continue;
            }
            return Err(err);
        }
    }

    /// 稳定内置 Pinned 分区 id（与 codex 源码常量一致，与 CODEX_HOME 无关）。
    const PINNED_SECTION_ID: &str = "01984de2-8f74-7c91-a3b2-5c5e937cf318";
    /// 只读获取当前 codex 的内置 Pinned 分区 id（不修改任何线程状态）。
    /// 置顶协议已固定为 `thread/section/move`（0.149+），此处仅负责定位
    /// Pinned 分区：优先按名称匹配，其次按内置常量 id 匹配，都找不到时
    /// 回退常量本身；`threadSection/list` 失败同样回退常量，不报错。
    pub async fn pinned_section_id(&self) -> Result<String, String> {
        let resp = self
            .request("threadSection/list", json!({ "limit": 50 }), None)
            .await?;
        let section_id = resp["data"]
            .as_array()
            .and_then(|arr| {
                arr.iter()
                    .find(|s| s["name"].as_str() == Some("Pinned"))
                    .or_else(|| {
                        arr.iter()
                            .find(|s| s["id"].as_str() == Some(Self::PINNED_SECTION_ID))
                    })
                    .and_then(|s| s["id"].as_str().map(|x| x.to_string()))
            })
            .unwrap_or_else(|| Self::PINNED_SECTION_ID.to_string());
        Ok(section_id)
    }

    /// 应答服务端反向请求（approval / user input / elicitation）。
    /// request id 按协议可为 number 或 string，原样回显。
    pub async fn send_response(&self, request_id: &Value, result: Value) -> Result<(), String> {
        if !request_id.is_number() && !request_id.is_string() {
            return Err("request id 必须是 number 或 string".to_string());
        }
        let msg = json!({ "jsonrpc": "2.0", "id": request_id, "result": result });
        let mut inner = self.shared.inner.lock().await;
        let stdin = inner
            .stdin
            .as_mut()
            .ok_or_else(|| "codex app-server stdin 不可用".to_string())?;
        let line = format!("{}\n", msg);
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn status(&self) -> Value {
        let inner = self.shared.inner.lock().await;
        json!({
            "connected": inner.connected,
            "codexPath": inner.codex_path.as_ref().map(|p| clean_path(Path::new(p))),
            "codexVersion": inner.codex_version,
            "versionTooOld": inner.version_too_old,
            "logs": inner.logs.clone(),
        })
    }

    pub async fn push_log(&self, level: &str, line: String) {
        let mut inner = self.shared.inner.lock().await;
        inner.push_log_locked(line.clone());
        drop(inner);
        self.log_file(level, None, "server-log", &[("msg".to_string(), line)]);
    }

    /// 连接成功后为 codex 日志库 `logs_2.sqlite` 应用阻断触发器（best-effort）。
    /// 先短暂轮询等待库文件出现（codex 启动早期可能尚未创建），再执行幂等触发器；
    /// 任何失败/缺失都只记日志，不阻断主流程、不向用户抛错。
    async fn apply_logs_guard(&self) {
        const DB_NAME: &str = "logs_2.sqlite";

        // 等待库文件出现：最多 WAIT_STEPS 步，每步 WAIT_STEP 毫秒。
        let mut waited: u32 = 0;
        loop {
            let existing = model_config::codex_home()
                .map(|home| home.join(DB_NAME).is_file())
                .unwrap_or(false);
            if existing {
                break;
            }
            if waited >= logs_guard::WAIT_STEPS {
                self.push_log("info", format!("{DB_NAME} 尚未创建，跳过日志库防护"))
                    .await;
                return;
            }
            waited += 1;
            tokio::time::sleep(logs_guard::WAIT_STEP).await;
        }

        match logs_guard::apply_at_codex_home(DB_NAME) {
            Ok(Some(db)) => {
                self.push_log(
                    "info",
                    format!("已为 codex 日志库 {DB_NAME} 应用阻断触发器：{}", db.display()),
                )
                .await;
            }
            Ok(None) => {
                self.push_log("info", format!("{DB_NAME} 尚不存在，跳过日志库防护"))
                    .await;
            }
            Err(e) => {
                self.push_log("warn", format!("应用日志库阻断触发器失败：{e}"))
                    .await;
            }
        }
    }

    async fn mark_disconnected(&self) {
        {
            let mut inner = self.shared.inner.lock().await;
            inner.connected = false;
            inner.ready = false;
            inner.stdin = None;
            if let Some(mut child) = inner.child.take() {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
            let pending = std::mem::take(&mut inner.pending);
            for (_, tx) in pending {
                let _ = tx.send(Err(RpcError {
                    code: None,
                    message: "codex app-server 连接中断".into(),
                }));
            }
        }
        self.push_log("warn", "codex app-server 已断开，正在重连…".into())
            .await;
    }

    async fn emit_status(&self) {
        let status = self.status().await;
        let _ = self.app.emit("server/status", status);
    }

    /// 执行 `codex --version` 探测版本（非阻断）：解析 major.minor，
    /// 仅低于 0.149.0 时记 warn 日志并标记 `versionTooOld`，其余记 info；
    /// 探测失败/无法解析只记日志，不警告。
    async fn probe_codex_version(self: &Arc<Self>) {
        let codex = match self.resolve_codex().await {
            Ok(p) => p,
            Err(e) => {
                self.push_log("warn", format!("探测 codex 版本失败：{e}")).await;
                return;
            }
        };
        let mut cmd = Command::new(&codex);
        cmd.arg("--version");
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        apply_codex_env(cmd.as_std_mut());
        let output = match cmd.spawn() {
            Ok(child) => match tokio::time::timeout(
                Duration::from_secs(5),
                child.wait_with_output(),
            )
            .await
            {
                Ok(Ok(o)) => o,
                Ok(Err(e)) => {
                    self.push_log("warn", format!("执行 codex --version 失败: {e}"))
                        .await;
                    return;
                }
                Err(_) => {
                    self.push_log("warn", "探测 codex 版本超时".into()).await;
                    return;
                }
            },
            Err(e) => {
                self.push_log("warn", format!("启动 codex --version 失败: {e}"))
                    .await;
                return;
            }
        };
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        match parse_codex_version(&text) {
            Some((major, minor)) => {
                let too_old = (major, minor) < (0, 149);
                {
                    let mut inner = self.shared.inner.lock().await;
                    inner.codex_version = Some(text.clone());
                    inner.version_too_old = Some(too_old);
                }
                if too_old {
                    self.push_log(
                        "warn",
                        format!("当前 codex 版本 {text} 低于 0.149.0，仅支持 0.149.x，部分功能可能异常"),
                    )
                    .await;
                } else {
                    self.push_log("info", format!("codex 版本: {text}")).await;
                }
                self.emit_status().await;
            }
            None => {
                self.push_log("info", format!("无法解析 codex 版本输出: {text:?}"))
                    .await;
            }
        }
    }

    async fn resolve_codex(&self) -> Result<PathBuf, String> {
        let settings = self
            .app
            .path()
            .app_data_dir()
            .ok()
            .map(|d| settings::load(&d))
            .unwrap_or_default();
        find_codex_sync(&settings)
    }
}

/// 从摘要中取出 threadId 并移除该键（文件行头部单独输出 `thread=` 列，避免重复）。
fn take_thread(kv: &mut Vec<(String, String)>) -> Option<String> {
    kv.iter()
        .position(|(k, _)| k == "threadId")
        .map(|i| kv.remove(i).1)
}

/// 截断长文本，防止错误消息把日志行撑爆。
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

/// codex 诊断通知级别：当前只处理 error/warning 两类。
fn codex_message_level(method: &str) -> Option<&'static str> {
    match method {
        "error" => Some("error"),
        "warning" => Some("warn"),
        _ => None,
    }
}

/// 提取 codex error/warning 的消息正文；缺失时回退为截断后的紧凑 JSON。
fn codex_message_text(method: &str, params: &Value) -> String {
    let raw = match method {
        "error" => params
            .pointer("/error/message")
            .and_then(|v| v.as_str())
            .or_else(|| params.get("message").and_then(|v| v.as_str())),
        "warning" => params.get("message").and_then(|v| v.as_str()),
        _ => None,
    };
    if let Some(text) = raw.filter(|s| !s.trim().is_empty()) {
        return truncate(text, 500);
    }
    truncate(&params.to_string(), 500)
}

/// DEBUG 构建发送全部诊断消息；Release 只发送 error。
fn should_emit_codex_message(level: &str, debug: bool) -> bool {
    level == "error" || debug
}

/// JSON 值转日志文本：字符串原样，其它类型序列化。
fn json_value_text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

/// 判断通知是否为流式增量事件：事件名最后一段包含 delta（大小写不敏感），
/// 覆盖 `/delta`、`textDelta`、`outputDelta` 等变体，避免日志被高频增量刷屏。
fn is_streaming_delta(event: &str) -> bool {
    event
        .rsplit('/')
        .next()
        .map(|seg| seg.to_ascii_lowercase().contains("delta"))
        .unwrap_or(false)
}

/// 在 npm 前缀目录下按嵌套/扁平布局找 @openai 平台包里的真实 codex.exe（x64/arm64）
fn npm_codex_exe(prefix: &Path) -> Option<PathBuf> {
    const LAYOUTS: [&[&str]; 2] = [
        &["node_modules", "@openai", "codex", "node_modules", "@openai"],
        &["node_modules", "@openai"],
    ];
    const TARGETS: [&[&str]; 2] = [
        &[
            "codex-win32-x64",
            "vendor",
            "x86_64-pc-windows-msvc",
            "bin",
            "codex.exe",
        ],
        &[
            "codex-win32-arm64",
            "vendor",
            "aarch64-pc-windows-msvc",
            "bin",
            "codex.exe",
        ],
    ];
    for layout in LAYOUTS {
        for target in TARGETS {
            let mut p = PathBuf::from(prefix);
            for seg in layout.iter().chain(target.iter()) {
                p.push(seg);
            }
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// 应用自身目录：运行中可执行文件（current_exe）所在目录。
pub(crate) fn app_exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

/// codex-cli 0.149 保留名市场的、codex 认可的 canonical 来源路径（纯函数，便于测试）。
/// - openai-bundled -> <CODEX_HOME>/.tmp/bundled-marketplaces/openai-bundled
/// - openai-primary-runtime -> %USERPROFILE%\.cache\codex-runtimes\codex-primary-runtime\plugins\openai-primary-runtime
/// 其它名字返回 None（调用方不应传非内置名）。
fn bundled_marketplace_source_in(
    codex_home: &Path,
    userprofile: &Path,
    name: &str,
) -> Option<PathBuf> {
    match name {
        "openai-bundled" => Some(
            codex_home
                .join(".tmp")
                .join("bundled-marketplaces")
                .join("openai-bundled"),
        ),
        "openai-primary-runtime" => Some(
            userprofile
                .join(".cache")
                .join("codex-runtimes")
                .join("codex-primary-runtime")
                .join("plugins")
                .join("openai-primary-runtime"),
        ),
        _ => None,
    }
}

/// 依据进程环境返回内置市场的 canonical 来源；无法定位 CODEX_HOME/%USERPROFILE% 时返回 None。
fn bundled_marketplace_source(name: &str) -> Option<PathBuf> {
    let home = model_config::codex_home().ok()?;
    let userprofile = std::env::var_os("USERPROFILE").map(PathBuf::from)?;
    bundled_marketplace_source_in(&home, &userprofile, name)
}

/// 解析内置市场应使用的 marketplace/add 来源。
/// 保留名返回 codex 认可的 canonical 路径；目录未就绪（归档未随包提供或解压失败）则返回 Err 让其跳过。
/// Rust 侧只登记、不复制、不扫描目录；catalog 由后台 `bundled::bootstrap` 解压到该 canonical 位置。
fn resolve_bundled_marketplace_source(name: &str) -> Result<String, String> {
    let Some(target) = bundled_marketplace_source(name) else {
        return Err(format!("未知内置市场名：{name}"));
    };
    if !target.is_dir() {
        return Err(format!("内置市场 {name} 的来源未就绪（归档未随包提供或解压失败），跳过"));
    }
    Ok(clean_path(&target))
}

/// 判断 marketplace/add 是否因“同名市场已从不同来源登记”而失败；
/// 此类需先移除旧登记再以新来源重加。
fn is_conflicting_marketplace_source_error(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("already added")
        && (m.contains("different source") || m.contains("remove it before"))
}

/// 应用自身目录下的 bin/codex.exe：current_exe 所在目录的 bin 子目录，
/// 文件存在才返回；不是工作目录。
fn bundled_codex_exe() -> Option<PathBuf> {
    let dir = app_exe_dir()?;
    bundled_codex_exe_in(&dir)
}

/// <应用目录>/bin/codex.exe，存在才返回（纯函数，便于测试）
fn bundled_codex_exe_in(dir: &Path) -> Option<PathBuf> {
    let p = dir.join("bin").join("codex.exe");
    p.is_file().then_some(p)
}

/// 为 codex.exe 子进程设置启动环境变量：
/// - PATH 前插应用自身目录的 bin（bin 存在时）与 codex-runtimes 的 node/python
///   依赖目录（无条件预置，运行期才下载解压），让 codex 能调用 CLI 工具与
///   node/python；
/// - 不显式设置 CODEX_HOME：子进程继承 codex-ui 的环境变量，未设置时 codex
///   默认使用 `%USERPROFILE%\.codex`（与设置页模型配置目录一致）。
pub fn apply_codex_env(cmd: &mut std::process::Command) {
    if let Some(dir) = app_exe_dir() {
        let userprofile = std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_default();
        apply_codex_env_in(cmd, &dir, &userprofile);
    }
}

/// codex-runtimes 依赖目录（`%USERPROFILE%\.cache\codex-runtimes\
/// codex-primary-runtime\dependencies`）下的 node/python 前缀目录。
/// 固定返回三条、不检查是否已存在：运行时由后台任务下载解压，PATH 需在
/// 安装完成前就预置，避免先启动的 codex/终端进程缺失这些条目。
pub(crate) fn runtime_dependency_dirs(userprofile: &Path) -> Vec<PathBuf> {
    if userprofile.as_os_str().is_empty() {
        return Vec::new();
    }
    let deps = userprofile
        .join(".cache")
        .join("codex-runtimes")
        .join("codex-primary-runtime")
        .join("dependencies");
    vec![
        deps.join("node").join("bin"),
        deps.join("python"),
        deps.join("python").join("Scripts"),
    ]
}

/// 把 <app_dir>/bin（存在时）与 codex-runtimes 依赖目录前插到现有 PATH；
/// 依赖目录即使尚未下载也一并加入。无任何可前插项时返回 None（保持原 PATH）。
/// codex 与终端子进程统一使用本函数注入 PATH（唯一的 PATH 前插实现）。
pub(crate) fn prepend_bin_and_runtime_path(
    existing: &str,
    app_dir: &Path,
    userprofile: &Path,
) -> Option<std::ffi::OsString> {
    let mut parts: Vec<PathBuf> = Vec::new();
    let bin = app_dir.join("bin");
    if bin.is_dir() {
        parts.push(bin);
    }
    parts.extend(runtime_dependency_dirs(userprofile));
    if parts.is_empty() {
        return None;
    }
    parts.extend(std::env::split_paths(existing));
    std::env::join_paths(parts).ok()
}

/// apply_codex_env 的纯函数变体：app_dir/userprofile 显式传入，便于测试。
fn apply_codex_env_in(
    cmd: &mut std::process::Command,
    app_dir: &Path,
    userprofile: &Path,
) {
    let existing = std::env::var("PATH").unwrap_or_default();
    if let Some(joined) = prepend_bin_and_runtime_path(&existing, app_dir, userprofile) {
        cmd.env("PATH", joined);
    }
}

/// 官方安装：glob %LOCALAPPDATA%\OpenAI\Codex\bin\*\codex.exe，按修改时间取最新
fn official_codex_exe() -> Option<PathBuf> {
    let root = std::env::var("LOCALAPPDATA")
        .map(|d| PathBuf::from(d).join("OpenAI").join("Codex").join("bin"))
        .ok()?;
    let mut best: Option<(PathBuf, std::time::SystemTime)> = None;
    for entry in std::fs::read_dir(&root).ok()?.flatten() {
        let exe = entry.path().join("codex.exe");
        if exe.is_file() {
            let mtime = std::fs::metadata(&exe)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            if best.as_ref().map(|(_, t)| mtime > *t).unwrap_or(true) {
                best = Some((exe, mtime));
            }
        }
    }
    best.map(|(p, _)| p)
}

/// VS Code Codex 扩展捆绑的 codex：
/// %USERPROFILE%\.vscode\extensions\openai.chatgpt-*\bin\{windows-x86_64|win32-x64}\codex.exe（取最新）
fn vscode_codex_exe() -> Option<PathBuf> {
    let root = std::env::var("USERPROFILE")
        .map(|d| PathBuf::from(d).join(".vscode").join("extensions"))
        .ok()?;
    let mut best: Option<(PathBuf, std::time::SystemTime)> = None;
    for entry in std::fs::read_dir(&root).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("openai.chatgpt-") {
            continue;
        }
        for sub in ["windows-x86_64", "win32-x64"] {
            let exe = entry.path().join("bin").join(sub).join("codex.exe");
            if exe.is_file() {
                let mtime = std::fs::metadata(&exe)
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                if best.as_ref().map(|(_, t)| mtime > *t).unwrap_or(true) {
                    best = Some((exe, mtime));
                }
            }
        }
    }
    best.map(|(p, _)| p)
}

/// 静态扫描候选：PATH 每目录（codex.exe 直接命中；codex.cmd shim 则解析真实 exe）
/// → %APPDATA%\npm → nvm-windows（%LOCALAPPDATA%\nvm\*\node_modules 同布局）。
/// 全部为文件存在性检查，零子进程。
fn static_path_candidates() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(path) = std::env::var("PATH") {
        roots.extend(std::env::split_paths(&path));
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        roots.push(PathBuf::from(appdata).join("npm"));
    }

    let mut out: Vec<PathBuf> = Vec::new();
    for root in roots {
        let exe = root.join("codex.exe");
        if exe.is_file() {
            out.push(exe);
            continue;
        }
        if root.join("codex.cmd").is_file() {
            if let Some(real) = npm_codex_exe(&root) {
                out.push(real);
            }
        }
    }

    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        if let Ok(entries) = std::fs::read_dir(PathBuf::from(local).join("nvm")) {
            for e in entries.flatten() {
                if let Some(real) = npm_codex_exe(&e.path()) {
                    out.push(real);
                }
            }
        }
    }
    out
}

/// 从 `codex --version` 输出中解析 major.minor（如 "codex-cli 0.149.0" → (0, 149)）。
/// 取首个形如 `<数字>.<数字>` 的片段；解析不到返回 None。
fn parse_codex_version(s: &str) -> Option<(u64, u64)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if start == i {
            i += 1;
            continue;
        }
        let major_str = &s[start..i];
        if i >= bytes.len() || bytes[i] != b'.' {
            continue;
        }
        i += 1;
        let minor_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if minor_start == i {
            continue;
        }
        let minor_str = &s[minor_start..i];
        let major = major_str.parse::<u64>().ok()?;
        let minor = minor_str.parse::<u64>().ok()?;
        return Some((major, minor));
    }
    None
}

/// Synchronous codex.exe discovery shared by the server and auth login.
/// 顺序：settings 路径 → 应用自身目录 bin → CODEX_BIN →
/// 官方安装（最新）→ PATH/APPDATA npm/nvm-windows 静态布局 → 报错。
pub fn find_codex_sync(settings: &AppSettings) -> Result<PathBuf, String> {
    // 1. 应用设置
    if let Some(p) = settings
        .codex_path
        .as_ref()
        .filter(|p| !p.trim().is_empty())
    {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Ok(pb);
        }
    }
    // 2. 应用自身目录 bin（current_exe 所在目录）
    if let Some(pb) = bundled_codex_exe() {
        return Ok(pb);
    }
    // 3. CODEX_BIN
    if let Ok(p) = std::env::var("CODEX_BIN") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Ok(pb);
        }
    }
    // 4. 官方安装
    if let Some(pb) = official_codex_exe() {
        return Ok(pb);
    }
    // 5. VS Code 扩展
    if let Some(pb) = vscode_codex_exe() {
        return Ok(pb);
    }
    // 6. 静态兜底
    for pb in static_path_candidates() {
        if pb.is_file() {
            return Ok(pb);
        }
    }
    Err("未找到 codex 可执行文件，请在设置中配置 codex 路径，或确认 codex 已加入 PATH".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    static ENV_LOCK: StdMutex<()> = StdMutex::new(());

    /// `CODEXUI_ZEN_TRACE` 关闭（默认）时不构造内容日志句柄，也不创建目录。
    #[test]
    fn zen_trace_handle_respects_switch() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("zen");
        assert!(zen_trace_from(Some(dir.clone()), false).is_none());
        assert!(!dir.exists(), "开关关闭时不应创建内容日志目录");
        assert!(zen_trace_from(Some(dir.clone()), true).is_some());
        assert!(zen_trace_from(None, true).is_none());
    }

    #[test]
    fn parse_codex_version_variants() {
        assert_eq!(parse_codex_version("codex-cli 0.149.0"), Some((0, 149)));
        assert_eq!(parse_codex_version("0.149.0"), Some((0, 149)));
        assert_eq!(
            parse_codex_version("codex-cli 0.149.0-alpha.9.2"),
            Some((0, 149))
        );
        assert_eq!(parse_codex_version("codex-cli 0.148.2"), Some((0, 148)));
        assert_eq!(parse_codex_version("codex-cli 0.150.0"), Some((0, 150)));
        assert_eq!(parse_codex_version("version 1.2.3"), Some((1, 2)));
        assert_eq!(parse_codex_version(""), None);
        assert_eq!(parse_codex_version("codex"), None);
        assert_eq!(parse_codex_version("0."), None);
        assert_eq!(parse_codex_version("0..1"), None);
    }

    /// 串行设置/恢复多个环境变量（避免并行测试互相干扰；Mutex 不可重入，勿嵌套）
    fn with_envs(pairs: &[(&str, Option<&str>)], f: impl FnOnce()) {
        let _guard = ENV_LOCK.lock().unwrap();
        let old: Vec<(&str, Option<std::ffi::OsString>)> = pairs
            .iter()
            .map(|(k, _)| (*k, std::env::var_os(k)))
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

    fn empty_settings() -> AppSettings {
        AppSettings::default()
    }

    #[test]
    fn streaming_delta_filter_matches_delta_variants() {
        for e in [
            "item/reasoning/textDelta",
            "item/commandExecution/outputDelta",
            "item/agentMessage/delta",
            "item/agentMessage/Delta",
        ] {
            assert!(is_streaming_delta(e), "{e} 应视为流式事件");
        }
    }

    #[test]
    fn streaming_delta_filter_keeps_state_events() {
        for e in [
            "turn/started",
            "turn/completed",
            "item/started",
            "item/completed",
            "thread/tokenUsage/updated",
            "mcpServer/startupStatus/updated",
            "remoteControl/status/changed",
            "server-request",
        ] {
            assert!(!is_streaming_delta(e), "{e} 不应视为流式事件");
        }
    }

    #[test]
    fn codex_message_level_only_maps_error_and_warning() {
        assert_eq!(codex_message_level("error"), Some("error"));
        assert_eq!(codex_message_level("warning"), Some("warn"));
        assert_eq!(codex_message_level("configWarning"), None);
        assert_eq!(codex_message_level("turn/started"), None);
    }

    #[test]
    fn codex_message_text_extracts_error_and_warning() {
        let error = serde_json::json!({
            "error": { "message": "boom", "codexErrorInfo": "serverOverloaded" }
        });
        assert_eq!(codex_message_text("error", &error), "boom");
        let warning = serde_json::json!({ "message": "careful" });
        assert_eq!(codex_message_text("warning", &warning), "careful");
        // 缺正文时回退为紧凑 JSON，且空字符串不会被当作有效消息
        let fallback = serde_json::json!({ "error": { "message": "  " }, "threadId": "t1" });
        assert!(codex_message_text("error", &fallback).contains("threadId"));
    }

    #[test]
    fn should_emit_codex_message_filters_warning_in_release() {
        assert!(should_emit_codex_message("error", false));
        assert!(should_emit_codex_message("error", true));
        assert!(should_emit_codex_message("warn", true));
        assert!(!should_emit_codex_message("warn", false));
    }

    #[test]
    fn bundled_codex_exe_in_resolves_bin_next_to_app_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let bin_exe = tmp.path().join("bin").join("codex.exe");
        std::fs::create_dir_all(bin_exe.parent().unwrap()).unwrap();
        std::fs::write(&bin_exe, b"MZ").unwrap();
        assert_eq!(bundled_codex_exe_in(tmp.path()), Some(bin_exe.clone()));

        // 有 bin 目录但没有 codex.exe
        let tmp2 = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp2.path().join("bin")).unwrap();
        assert_eq!(bundled_codex_exe_in(tmp2.path()), None);

        // 无 bin 目录
        let tmp3 = tempfile::tempdir().unwrap();
        assert_eq!(bundled_codex_exe_in(tmp3.path()), None);
    }

    #[test]
    fn apply_codex_env_in_prepends_bin_and_runtime_paths() {
        use std::ffi::OsStr;

        // bin 与 userprofile 都有：PATH 前插 bin 与三条 codex-runtimes 依赖目录，
        // 且保留原 PATH；不设置 CODEX_HOME
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("bin")).unwrap();
        let old = tmp.path().join("old").join("bin").to_string_lossy().into_owned();
        with_envs(&[("PATH", Some(&old))], || {
            let mut cmd = std::process::Command::new("codex");
            apply_codex_env_in(&mut cmd, tmp.path(), tmp.path());
            let envs: Vec<(std::ffi::OsString, Option<std::ffi::OsString>)> = cmd
                .get_envs()
                .map(|(k, v)| (k.to_os_string(), v.map(|s| s.to_os_string())))
                .collect();

            let path = envs
                .iter()
                .find_map(|(k, v)| (k == OsStr::new("PATH")).then(|| v.clone().unwrap()))
                .unwrap();
            let parts: Vec<_> = std::env::split_paths(&path).collect();
            assert_eq!(parts.first().unwrap(), &tmp.path().join("bin"));
            assert_eq!(
                &parts[1..4],
                runtime_dependency_dirs(tmp.path()).as_slice(),
                "依赖目录即使尚未下载也应收录在 bin 之后"
            );
            assert!(parts.contains(&tmp.path().join("old").join("bin")));

            assert!(
                envs.iter().all(|(k, _)| k != OsStr::new("CODEX_HOME")),
                "不应设置 CODEX_HOME"
            );
        });

        // bin 缺失且 userprofile 为空：PATH 不改写，也不设置 CODEX_HOME
        let tmp2 = tempfile::tempdir().unwrap();
        let dir2_s = tmp2.path().to_string_lossy().into_owned();
        with_envs(&[("PATH", Some(&dir2_s))], || {
            let mut cmd = std::process::Command::new("codex");
            apply_codex_env_in(&mut cmd, tmp2.path(), Path::new(""));
            let envs: Vec<(std::ffi::OsString, Option<std::ffi::OsString>)> = cmd
                .get_envs()
                .map(|(k, v)| (k.to_os_string(), v.map(|s| s.to_os_string())))
                .collect();

            assert!(
                envs.iter().all(|(k, _)| k != OsStr::new("PATH")),
                "bin 缺失时不应改写 PATH"
            );
            assert!(
                envs.iter().all(|(k, _)| k != OsStr::new("CODEX_HOME")),
                "不应设置 CODEX_HOME"
            );
        });
    }

    #[test]
    fn bundled_marketplace_source_in_maps_reserved_names() {
        let home = Path::new("C:\\home\\codex");
        let user = Path::new("C:\\Users\\u");
        assert_eq!(
            bundled_marketplace_source_in(home, user, "openai-bundled").unwrap(),
            Path::new("C:\\home\\codex\\.tmp\\bundled-marketplaces\\openai-bundled")
        );
        assert_eq!(
            bundled_marketplace_source_in(home, user, "openai-primary-runtime").unwrap(),
            Path::new(
                "C:\\Users\\u\\.cache\\codex-runtimes\\codex-primary-runtime\\plugins\\openai-primary-runtime"
            )
        );
        assert_eq!(bundled_marketplace_source_in(home, user, "other"), None);
    }

    #[test]
    fn bundled_marketplaces_const_all_resolve_to_canonical_source() {
        let home = Path::new("C:\\home\\codex");
        let user = Path::new("C:\\Users\\u");
        for name in BUNDLED_MARKETPLACES {
            assert!(
                bundled_marketplace_source_in(home, user, name).is_some(),
                "{name} 应能解析到 canonical 来源"
            );
        }
    }

    #[test]
    fn done_state_wait_blocks_until_mark_done() {
        let state = Arc::new(DoneState::new());
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let waiter = state.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let handle = rt.spawn(async move {
            started_tx.send(()).unwrap();
            waiter.wait().await;
            true
        });
        // 等待任务真正进入 wait，再通知唤醒，验证“先等待后通知”能解除阻塞。
        started_rx.recv().unwrap();
        state.mark_done();
        assert!(rt.block_on(handle).unwrap());
    }

    #[test]
    fn done_state_wait_returns_immediately_after_mark_done() {
        let state = Arc::new(DoneState::new());
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        state.mark_done();
        let waiter = state.clone();
        // 已通知后再等待应立刻返回。
        rt.block_on(async move { waiter.wait().await });
    }

    #[test]
    fn conflicting_marketplace_source_error_matches() {
        assert!(is_conflicting_marketplace_source_error(
            "marketplace 'openai-bundled' is already added from a different source; remove it before adding this source"
        ));
        assert!(is_conflicting_marketplace_source_error(
            "marketplace 'openai-bundled' is already added from a different source"
        ));
        assert!(!is_conflicting_marketplace_source_error(
            "marketplace `openai-primary-runtime` is reserved and cannot be added from this source"
        ));
        assert!(!is_conflicting_marketplace_source_error("unknown error"));
    }

    #[test]
    fn runtime_dependency_dirs_fixed_order_and_empty_profile() {
        let user = Path::new("C:\\Users\\u");
        let dirs = runtime_dependency_dirs(user);
        assert_eq!(
            dirs,
            vec![
                PathBuf::from(
                    "C:\\Users\\u\\.cache\\codex-runtimes\\codex-primary-runtime\
                     \\dependencies\\node\\bin"
                ),
                PathBuf::from(
                    "C:\\Users\\u\\.cache\\codex-runtimes\\codex-primary-runtime\
                     \\dependencies\\python"
                ),
                PathBuf::from(
                    "C:\\Users\\u\\.cache\\codex-runtimes\\codex-primary-runtime\
                     \\dependencies\\python\\Scripts"
                ),
            ]
        );
        // 不要求目录已存在（运行期才下载解压）：无需创建任何目录
        assert!(runtime_dependency_dirs(Path::new("")).is_empty());
    }

    #[test]
    fn prepend_bin_and_runtime_path_prepends_bin_then_runtime() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("bin")).unwrap();
        let existing = std::env::join_paths([
            tmp.path().join("a"),
            tmp.path().join("b"),
        ])
        .unwrap();

        // bin 在最前，三条 runtime 目录随后，原 PATH 保留
        let joined =
            prepend_bin_and_runtime_path(&existing.to_string_lossy(), tmp.path(), tmp.path())
                .unwrap();
        let parts: Vec<_> = std::env::split_paths(&joined).collect();
        assert_eq!(parts.first().unwrap(), &tmp.path().join("bin"));
        assert_eq!(&parts[1..4], runtime_dependency_dirs(tmp.path()).as_slice());
        assert!(parts.contains(&tmp.path().join("a")));
        assert!(parts.contains(&tmp.path().join("b")));

        // bin 缺失且 userprofile 为空：无可前插项 → None（保持原 PATH）
        let tmp2 = tempfile::tempdir().unwrap();
        assert!(
            prepend_bin_and_runtime_path(
                &existing.to_string_lossy(),
                tmp2.path(),
                Path::new(""),
            )
            .is_none()
        );

        // bin 缺失但 userprofile 非空：仍前插三条依赖目录（不检查存在性）
        let joined2 =
            prepend_bin_and_runtime_path(&existing.to_string_lossy(), tmp2.path(), tmp.path())
                .unwrap();
        let dirs = runtime_dependency_dirs(tmp.path());
        let parts2: Vec<_> = std::env::split_paths(&joined2).collect();
        assert_eq!(&parts2[..3], dirs.as_slice());
        assert!(parts2.contains(&tmp.path().join("a")));
        assert!(parts2.contains(&tmp.path().join("b")));
    }

    #[test]
    fn npm_codex_exe_nested_and_flat() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp
            .path()
            .join("node_modules")
            .join("@openai")
            .join("codex")
            .join("node_modules")
            .join("@openai")
            .join("codex-win32-x64")
            .join("vendor")
            .join("x86_64-pc-windows-msvc")
            .join("bin")
            .join("codex.exe");
        std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
        std::fs::write(&nested, b"MZ").unwrap();
        assert_eq!(npm_codex_exe(tmp.path()), Some(nested.clone()));

        let tmp2 = tempfile::tempdir().unwrap();
        let flat = tmp2
            .path()
            .join("node_modules")
            .join("@openai")
            .join("codex-win32-x64")
            .join("vendor")
            .join("x86_64-pc-windows-msvc")
            .join("bin")
            .join("codex.exe");
        std::fs::create_dir_all(flat.parent().unwrap()).unwrap();
        std::fs::write(&flat, b"MZ").unwrap();
        assert_eq!(npm_codex_exe(tmp2.path()), Some(flat));

        let tmp3 = tempfile::tempdir().unwrap();
        assert_eq!(npm_codex_exe(tmp3.path()), None);
    }

    #[test]
    fn official_codex_exe_picks_newest() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("OpenAI").join("Codex").join("bin");
        let old_dir = bin.join("oldhash");
        let new_dir = bin.join("newhash");
        std::fs::create_dir_all(&old_dir).unwrap();
        std::fs::create_dir_all(&new_dir).unwrap();
        let old = old_dir.join("codex.exe");
        let new = new_dir.join("codex.exe");
        std::fs::write(&old, b"MZ").unwrap();
        std::fs::write(&new, b"MZ").unwrap();
        let future = std::time::SystemTime::now() + std::time::Duration::from_secs(10);
        set_modified(&new, future).unwrap();
        let local = tmp.path().to_string_lossy().into_owned();
        with_envs(&[("LOCALAPPDATA", Some(&local))], || {
            assert_eq!(official_codex_exe(), Some(new.clone()));
        });
    }

    #[test]
    fn vscode_codex_exe_picks_newest() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_root = tmp.path().join(".vscode").join("extensions");
        let old_dir = ext_root.join("openai.chatgpt-1.0.0-win32-x64");
        let new_dir = ext_root.join("openai.chatgpt-2.0.0");
        let old = old_dir
            .join("bin")
            .join("windows-x86_64")
            .join("codex.exe");
        let new = new_dir.join("bin").join("win32-x64").join("codex.exe");
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        std::fs::create_dir_all(new.parent().unwrap()).unwrap();
        std::fs::write(&old, b"MZ").unwrap();
        std::fs::write(&new, b"MZ").unwrap();
        let future = std::time::SystemTime::now() + std::time::Duration::from_secs(10);
        set_modified(&new, future).unwrap();
        let user = tmp.path().to_string_lossy().into_owned();
        with_envs(&[("USERPROFILE", Some(&user))], || {
            assert_eq!(vscode_codex_exe(), Some(new.clone()));
        });
    }

    #[cfg(windows)]
    fn set_modified(p: &Path, t: std::time::SystemTime) -> std::io::Result<()> {
        let f = std::fs::File::options().write(true).open(p)?;
        f.set_times(std::fs::FileTimes::new().set_modified(t))?;
        Ok(())
    }

    #[cfg(not(windows))]
    fn set_modified(p: &Path, t: std::time::SystemTime) -> std::io::Result<()> {
        let f = std::fs::File::options().write(true).open(p)?;
        f.set_times(std::fs::FileTimes::new().set_modified(t))?;
        Ok(())
    }

    fn nested_codex_exe(root: &Path) -> PathBuf {
        root.join("node_modules")
            .join("@openai")
            .join("codex")
            .join("node_modules")
            .join("@openai")
            .join("codex-win32-x64")
            .join("vendor")
            .join("x86_64-pc-windows-msvc")
            .join("bin")
            .join("codex.exe")
    }

    #[test]
    fn find_codex_sync_full_chain() {
        let tmp = tempfile::tempdir().unwrap();
        let settings_exe = tmp.path().join("settings.exe");
        std::fs::write(&settings_exe, b"MZ").unwrap();
        let mut settings = empty_settings();
        settings.codex_path = Some(settings_exe.to_string_lossy().into_owned());
        assert_eq!(find_codex_sync(&settings).unwrap(), settings_exe);

        settings.codex_path = None;

        // 3. CODEX_BIN
        let bin = tmp.path().join("env.exe");
        std::fs::write(&bin, b"MZ").unwrap();
        let home2 = tempfile::tempdir().unwrap();
        let home2_s = home2.path().to_string_lossy().into_owned();
        let bin_s = bin.to_string_lossy().into_owned();
        with_envs(
            &[
                ("CODEX_HOME", Some(&home2_s)),
                ("CODEX_BIN", Some(&bin_s)),
            ],
            || {
                assert_eq!(find_codex_sync(&settings).unwrap(), bin);
            },
        );

        // 4. VS Code 扩展（官方之后、静态之前）
        let user = tempfile::tempdir().unwrap();
        let vscode_exe = user
            .path()
            .join(".vscode")
            .join("extensions")
            .join("openai.chatgpt-26.803.61601-win32-x64")
            .join("bin")
            .join("windows-x86_64")
            .join("codex.exe");
        std::fs::create_dir_all(vscode_exe.parent().unwrap()).unwrap();
        std::fs::write(&vscode_exe, b"MZ").unwrap();
        let user_s = user.path().to_string_lossy().into_owned();
        let local_empty = tempfile::tempdir().unwrap();
        let local_empty_s = local_empty.path().to_string_lossy().into_owned();
        with_envs(
            &[
                ("CODEX_HOME", Some(&home2_s)),
                ("CODEX_BIN", None),
                ("PATH", None),
                ("LOCALAPPDATA", Some(&local_empty_s)),
                ("APPDATA", None),
                ("USERPROFILE", Some(&user_s)),
            ],
            || {
                assert_eq!(find_codex_sync(&settings).unwrap(), vscode_exe);
            },
        );

        // 5. PATH 目录 codex.exe
        let dir = tempfile::tempdir().unwrap();
        let path_exe = dir.path().join("codex.exe");
        std::fs::write(&path_exe, b"MZ").unwrap();
        let dir_s = dir.path().to_string_lossy().into_owned();
        with_envs(
            &[
                ("CODEX_HOME", Some(&home2_s)),
                ("CODEX_BIN", None),
                ("PATH", Some(&dir_s)),
                ("LOCALAPPDATA", None),
                ("APPDATA", None),
                ("USERPROFILE", None),
            ],
            || {
                assert_eq!(find_codex_sync(&settings).unwrap(), path_exe);
            },
        );

        // 6. PATH 目录 codex.cmd + 嵌套布局 → 返回真实 exe
        let dir2 = tempfile::tempdir().unwrap();
        std::fs::write(dir2.path().join("codex.cmd"), b"@echo off").unwrap();
        let real = nested_codex_exe(dir2.path());
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::write(&real, b"MZ").unwrap();
        let dir2_s = dir2.path().to_string_lossy().into_owned();
        with_envs(
            &[
                ("CODEX_HOME", Some(&home2_s)),
                ("CODEX_BIN", None),
                ("PATH", Some(&dir2_s)),
                ("LOCALAPPDATA", None),
                ("APPDATA", None),
                ("USERPROFILE", None),
            ],
            || {
                assert_eq!(find_codex_sync(&settings).unwrap(), real);
            },
        );

        // 7. nvm-windows 布局
        let local = tempfile::tempdir().unwrap();
        let nvm_exe = nested_codex_exe(&local.path().join("nvm").join("v25.1.0"));
        std::fs::create_dir_all(nvm_exe.parent().unwrap()).unwrap();
        std::fs::write(&nvm_exe, b"MZ").unwrap();
        let local_s = local.path().to_string_lossy().into_owned();
        with_envs(
            &[
                ("CODEX_HOME", Some(&home2_s)),
                ("CODEX_BIN", None),
                ("PATH", None),
                ("LOCALAPPDATA", Some(&local_s)),
                ("APPDATA", None),
                ("USERPROFILE", None),
            ],
            || {
                assert_eq!(find_codex_sync(&settings).unwrap(), nvm_exe);
            },
        );

        // 8. 全部为空 → 报错
        with_envs(
            &[
                ("CODEX_HOME", Some(&home2_s)),
                ("CODEX_BIN", None),
                ("PATH", None),
                ("LOCALAPPDATA", None),
                ("APPDATA", None),
                ("USERPROFILE", None),
            ],
            || {
                assert!(find_codex_sync(&settings).is_err());
            },
        );
    }
}
