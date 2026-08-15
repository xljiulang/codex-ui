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
use tokio::sync::{Mutex, oneshot};

use crate::codex::settings::{self, AppSettings};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const MAX_LOG_LINES: usize = 500;

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
}

struct Shared {
    inner: Mutex<Inner>,
    stop: AtomicBool,
}

struct Inner {
    stdin: Option<ChildStdin>,
    child: Option<Child>,
    pending: HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>,
    connected: bool,
    ready: bool,
    logs: Vec<String>,
    codex_path: Option<PathBuf>,
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

impl CodexServer {
    pub fn new(app: AppHandle, workspace: PathBuf) -> Self {
        Self {
            app,
            shared: Arc::new(Shared {
                inner: Mutex::new(Inner::new()),
                stop: AtomicBool::new(false),
            }),
            next_id: AtomicU64::new(0),
            workspace,
            run_started: AtomicBool::new(false),
        }
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
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
    }

    async fn run(self: Arc<Self>) {
        while !self.shared.stop.load(Ordering::SeqCst) {
            if let Err(e) = self.spawn_and_read().await {
                self.push_log(format!("codex app-server 错误: {e}")).await;
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
                                this.push_log(format!("[stderr] {t}")).await;
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
                "clientInfo": { "name": "codex-ui", "title": "Codex UI", "version": "0.1.0" },
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
                        self.push_log(format!("initialize 失败: {}", v["error"]))
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
            inner.push_log_locked(format!(
                "已启动 codex app-server（{}），工作目录：{}",
                codex.display(),
                self.workspace.display()
            ));
        }
        self.emit_status().await;

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
                Err(e) => self
                    .push_log(format!("无法解析 app-server 消息: {e}"))
                    .await,
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

    /// 保留 JSON-RPC error code 的请求变体，供能力探测等需要区分错误类型的场景使用。
    pub async fn request_verbose(
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
    /// 探测用假线程 id：合法 UUID 格式，任何真实 codex 都会在字段校验后报 thread not found。
    const PROBE_THREAD_ID: &str = "00000000-0000-0000-0000-000000000000";
    /// 分区移动方法的探测顺序：0.146 及此前为 `threadSection/move`，
    /// 新版（0.147+）改名为 `thread/section/move`（`threadSection/list` 保留）。
    const SECTION_MOVE_METHODS: [&str; 2] = ["threadSection/move", "thread/section/move"];

    fn method_unavailable(e: &RpcError) -> bool {
        e.code == Some(-32601) || e.message.contains("unknown variant")
    }

    /// 探测当前 codex 的置顶协议能力（只读，不修改任何线程状态）。
    /// 返回 `{ protocol, pinnedSectionId, sectionMoveMethod? }`：
    /// - `section_move`：新版分区协议（`sectionMoveMethod` 为实际可用的移动方法名）；
    /// - `metadata_section`：分区时代 `thread/metadata/update { sectionId }`；
    /// - `metadata_is_pinned`：旧版 `thread/metadata/update { isPinned }`；
    /// - `unsupported`：完全不支持置顶。
    pub async fn pin_capability(&self) -> Result<Value, String> {
        match self
            .request_verbose("threadSection/list", json!({ "limit": 50 }), None)
            .await
        {
            Ok(resp) => {
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
                // 依次探测两个分区移动方法名；任一可用即采用（假线程 id 会报
                // thread not found，视为方法存在）。
                let mut move_method: Option<&'static str> = None;
                for m in Self::SECTION_MOVE_METHODS {
                    match self
                        .request_verbose(
                            m,
                            json!({ "threadId": Self::PROBE_THREAD_ID, "sectionId": null }),
                            None,
                        )
                        .await
                    {
                        Err(e) if Self::method_unavailable(&e) => continue,
                        Err(e) if e.code.is_none() => return Err(e.message),
                        _ => {
                            move_method = Some(m);
                            break;
                        }
                    }
                }
                match move_method {
                    Some(m) => Ok(json!({
                        "protocol": "section_move",
                        "pinnedSectionId": section_id,
                        "sectionMoveMethod": m,
                    })),
                    None => Ok(json!({
                        "protocol": "metadata_section",
                        "pinnedSectionId": section_id,
                    })),
                }
            }
            Err(e) if Self::method_unavailable(&e) => match self
                .request_verbose(
                    "thread/metadata/update",
                    json!({ "threadId": Self::PROBE_THREAD_ID, "isPinned": true }),
                    None,
                )
                .await
            {
                Err(e) if e.message.contains("must include at least one field") => Ok(json!({
                    "protocol": "unsupported",
                    "pinnedSectionId": null,
                })),
                Err(e) if e.code.is_none() => Err(e.message),
                _ => Ok(json!({
                    "protocol": "metadata_is_pinned",
                    "pinnedSectionId": null,
                })),
            },
            Err(e) => Err(e.message),
        }
    }

    /// 探测当前 codex 的“临时线程标题总结”能力（仿 VS Code：ephemeral 线程
    /// 总结首条消息，使用默认模型）。探测本身只创建一个内存线程并立即释放，不落盘。
    /// 返回 `{ experimentalApi, ephemeral }`：
    /// - `{ experimentalApi: true, ephemeral: true }`：支持临时线程 + 实验字段回退；
    /// - `{ experimentalApi: true, ephemeral: false }`：支持实验 API 但不支持临时线程；
    /// - `{ experimentalApi: false }`：不支持实验 API，标题总结整体跳过。
    pub async fn title_helper_capability(&self) -> Result<Value, String> {
        match self
            .request_verbose(
                "thread/start",
                json!({
                    "cwd": self.workspace,
                    "ephemeral": true,
                    "allowProviderModelFallback": true,
                    "approvalPolicy": "never",
                    "sandbox": "read-only",
                }),
                Some(Duration::from_secs(30)),
            )
            .await
        {
            Ok(resp) => {
                let thread_id = resp["thread"]["id"]
                    .as_str()
                    .map(|s| s.to_string());
                if let Some(tid) = thread_id {
                    let _ = self
                        .request("thread/unsubscribe", json!({ "threadId": tid }), None)
                        .await;
                }
                Ok(json!({ "experimentalApi": true, "ephemeral": true }))
            }
            Err(e) if e.code.is_none() => Err(e.message),
            Err(e) => {
                let msg = e.message.to_lowercase();
                if msg.contains("experimentalapi") {
                    Ok(json!({ "experimentalApi": false, "ephemeral": false }))
                } else if msg.contains("unknown field") || msg.contains("ephemeral") {
                    Ok(json!({ "experimentalApi": true, "ephemeral": false }))
                } else {
                    // 其它错误（如字段被拒/参数不识别）：保守按不支持处理
                    Ok(json!({ "experimentalApi": false, "ephemeral": false }))
                }
            }
        }
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
            "startupWorkspace": self.workspace.to_string_lossy(),
            "codexPath": inner.codex_path.as_ref().map(|p| p.to_string_lossy().to_string()),
            "logs": inner.logs.clone(),
        })
    }

    pub async fn push_log(&self, line: String) {
        let mut inner = self.shared.inner.lock().await;
        inner.push_log_locked(line);
    }

    async fn mark_disconnected(&self) {
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
        inner.push_log_locked("codex app-server 已断开，正在重连…".into());
    }

    async fn emit_status(&self) {
        let status = self.status().await;
        let _ = self.app.emit("server/status", status);
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

/// codex 配置文件路径：CODEX_HOME（若设置）否则 %USERPROFILE%\.codex\config.toml。
/// CODEX_HOME 设置时完全重定向，不回退到 USERPROFILE。
fn codex_config_path() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("CODEX_HOME") {
        let p = PathBuf::from(home).join("config.toml");
        return p.is_file().then_some(p);
    }
    let p = std::env::var("USERPROFILE")
        .map(|d| PathBuf::from(d).join(".codex").join("config.toml"))
        .ok()?;
    p.is_file().then_some(p)
}

/// 读取 config.toml 中 [mcp_servers.node_repl.env] 的 CODEX_CLI_PATH，值指向的文件存在才返回
fn read_codex_cli_path(config: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(config).ok()?;
    let doc: toml::Value = toml::from_str(&text).ok()?;
    let v = doc
        .get("mcp_servers")?
        .get("node_repl")?
        .get("env")?
        .get("CODEX_CLI_PATH")?
        .as_str()?;
    let pb = PathBuf::from(v);
    pb.is_file().then_some(pb)
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

/// Synchronous codex.exe discovery shared by the server and auth login.
/// 顺序：settings 路径 → config.toml CODEX_CLI_PATH → CODEX_BIN →
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
    // 2. config.toml 的 CODEX_CLI_PATH
    if let Some(cfg) = codex_config_path() {
        if let Some(pb) = read_codex_cli_path(&cfg) {
            return Ok(pb);
        }
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

    fn write_config(cli_path: &Path, dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        let toml = format!(
            "[mcp_servers.node_repl.env]\nCODEX_CLI_PATH = '{}'\n",
            cli_path.display()
        );
        std::fs::write(dir.join("config.toml"), toml).unwrap();
    }

    #[test]
    fn read_codex_cli_path_valid_and_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("codex.exe");
        std::fs::write(&exe, b"MZ").unwrap();
        write_config(&exe, tmp.path());

        assert_eq!(
            read_codex_cli_path(&tmp.path().join("config.toml")),
            Some(exe)
        );

        let bad = tmp.path().join("bad.toml");
        std::fs::write(
            &bad,
            "[mcp_servers.node_repl.env]\nCODEX_CLI_PATH = 'C:\\nope\\x.exe'\n",
        )
        .unwrap();
        assert_eq!(read_codex_cli_path(&bad), None);

        let nokey = tmp.path().join("nokey.toml");
        std::fs::write(&nokey, "[other]\n").unwrap();
        assert_eq!(read_codex_cli_path(&nokey), None);
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

        // 2. config.toml（CODEX_HOME 指向临时目录）
        let home = tempfile::tempdir().unwrap();
        let cfg_exe = home.path().join("cfg.exe");
        std::fs::write(&cfg_exe, b"MZ").unwrap();
        write_config(&cfg_exe, home.path());
        settings.codex_path = None;
        let home_s = home.path().to_string_lossy().into_owned();
        with_envs(&[("CODEX_HOME", Some(&home_s)), ("CODEX_BIN", None)], || {
            assert_eq!(find_codex_sync(&settings).unwrap(), cfg_exe);
        });

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
