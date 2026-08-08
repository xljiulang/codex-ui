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
    pending: HashMap<u64, oneshot::Sender<Result<Value, String>>>,
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
        if let Some(id) = v.get("id").and_then(|x| x.as_u64()) {
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
                let msg = v["error"]
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("codex 请求失败");
                Err(msg.to_string())
            } else {
                Ok(v.get("result").cloned().unwrap_or(Value::Null))
            };
            let mut inner = self.shared.inner.lock().await;
            if let Some(tx) = inner.pending.remove(&id) {
                let _ = tx.send(result);
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
                return Err("codex app-server 未就绪，请稍后重试".into());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        {
            let mut inner = self.shared.inner.lock().await;
            if !inner.connected {
                return Err("codex app-server 未连接，正在重连，请稍候".into());
            }
            let stdin = inner
                .stdin
                .as_mut()
                .ok_or_else(|| "codex app-server stdin 不可用".to_string())?;
            let line = format!("{}\n", msg);
            stdin
                .write_all(line.as_bytes())
                .await
                .map_err(|e| format!("写入 app-server 失败: {e}"))?;
            inner.pending.insert(id, tx);
        }
        let t = timeout.unwrap_or(Duration::from_secs(30));
        tokio::time::timeout(t, rx)
            .await
            .map_err(|_| format!("请求 {method} 超时"))?
            .map_err(|_| "codex app-server 连接中断".to_string())?
    }

    pub async fn send_response(&self, id: u64, result: Value) -> Result<(), String> {
        let msg = json!({ "jsonrpc": "2.0", "id": id, "result": result });
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
            "workspace": self.workspace.to_string_lossy(),
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
            let _ = tx.send(Err("codex app-server 连接中断".into()));
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

/// Synchronous codex.exe discovery shared by the server and auth login.
pub fn find_codex_sync(settings: &AppSettings) -> Result<PathBuf, String> {
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
    if let Ok(p) = std::env::var("CODEX_BIN") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Ok(pb);
        }
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let cand = dir.join("codex.exe");
            if cand.is_file() {
                return Ok(cand);
            }
        }
    }
    let candidates = [
        std::env::var("APPDATA")
            .map(|d| PathBuf::from(d).join("npm").join("codex.exe"))
            .ok(),
        std::env::var("USERPROFILE")
            .map(|d| PathBuf::from(d).join(".local").join("bin").join("codex.exe"))
            .ok(),
        std::env::var("USERPROFILE")
            .map(|d| PathBuf::from(d).join("scoop").join("shims").join("codex.exe"))
            .ok(),
    ];
    for c in candidates.into_iter().flatten() {
        if c.is_file() {
            return Ok(c);
        }
    }
    Err("未找到 codex 可执行文件，请在设置中配置 codex 路径，或确认 codex 已加入 PATH".into())
}
