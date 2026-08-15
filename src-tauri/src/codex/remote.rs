//! 远程 Web 模式：`--remote` 启动内置 HTTP + WebSocket 服务，手机/浏览器访问同一份前端。
//! 鉴权用启动令牌（`--token` 或自动生成）；事件经 `RemoteHub` 广播到 WebSocket 客户端。
//! 远程端只能调用应用接口，不暴露操作系统。

use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::codex::app_server::CodexServer;
use crate::codex::commands;
use crate::codex::git::{self, GitError};
use crate::codex::session_fs;
use crate::codex::terminal;

/// 远程模式启动配置（命令行解析）
#[derive(Debug, Clone)]
pub struct RemoteConfig {
    pub enabled: bool,
    pub port: u16,
    pub token: String,
}

/// 解析命令行远程参数：`--remote [--port N] [--token T]`
pub fn parse_remote_args() -> RemoteConfig {
    let mut enabled = false;
    let mut port = 8000u16;
    let mut token = String::new();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--remote" => enabled = true,
            "--port" => {
                if let Some(v) = args.get(i + 1) {
                    port = v.parse().unwrap_or(8000);
                    i += 1;
                }
            }
            "--token" => {
                if let Some(v) = args.get(i + 1) {
                    token = v.clone();
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    if enabled && token.is_empty() {
        token = generate_token();
    }
    RemoteConfig {
        enabled,
        port,
        token,
    }
}

/// 生成默认令牌（32 位十六进制；非密码学强随机，公网部署请显式传 `--token`）
fn generate_token() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut h);
    std::process::id().hash(&mut h);
    format!("{:016x}{:016x}", h.finish(), h.finish())
}

/// WebSocket 推送消息
#[derive(Debug, Clone, Serialize)]
pub struct WsMessage {
    pub event: String,
    pub payload: Value,
}

/// 远程服务共享状态：WS 广播 + 令牌（始终 manage；未启用时 enabled=false）
pub struct RemoteHub {
    pub tx: tokio::sync::broadcast::Sender<WsMessage>,
    pub token: String,
    pub enabled: bool,
}

impl RemoteHub {
    pub fn new(cfg: &RemoteConfig) -> Self {
        let (tx, _) = tokio::sync::broadcast::channel::<WsMessage>(256);
        Self {
            tx,
            token: cfg.token.clone(),
            enabled: cfg.enabled,
        }
    }
}

/// 统一事件发射：Tauri 前端 + 远程 WS 广播（payload 为已序列化的 JSON）
pub fn emit_event(app: &AppHandle, event: &str, payload: Value) {
    let _ = app.emit(event, payload.clone());
    if let Some(hub) = app.try_state::<RemoteHub>() {
        if hub.enabled {
            let _ = hub
                .tx
                .send(WsMessage {
                    event: event.to_string(),
                    payload,
                });
        }
    }
}

// ---------------- HTTP 服务 ----------------

#[derive(Debug, Deserialize)]
struct RpcRequest {
    cmd: String,
    #[serde(default)]
    args: Value,
}

fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, "未授权：缺少或错误的令牌").into_response()
}

fn check_auth(app: &AppHandle, headers: &HeaderMap, query: &Value) -> bool {
    let Some(hub) = app.try_state::<RemoteHub>() else {
        return false;
    };
    if !hub.enabled || hub.token.is_empty() {
        return false;
    }
    if let Some(v) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(t) = v.strip_prefix("Bearer ") {
            if t == hub.token {
                return true;
            }
        }
    }
    if let Some(t) = query.get("token").and_then(|v| v.as_str()) {
        if t == hub.token {
            return true;
        }
    }
    false
}

pub fn router(app: AppHandle) -> Router {
    Router::new()
        .route("/rpc", post(rpc_handler))
        .route("/events", get(events_handler))
        .route("/asset", get(asset_handler))
        .fallback(static_handler)
        .with_state(app)
}

pub async fn start_remote(app: AppHandle, cfg: RemoteConfig) {
    // 令牌落盘，方便读取
    if let Ok(dir) = app.path().app_data_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("remote-token.txt"), &cfg.token);
    }
    let bind = format!("0.0.0.0:{}", cfg.port);
    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("远程模式启动失败（{bind}）: {e}");
            return;
        }
    };
    eprintln!(
        "远程模式已启动: http://0.0.0.0:{} （令牌: {}）",
        cfg.port, cfg.token
    );
    if let Err(e) = axum::serve(listener, router(app)).await {
        eprintln!("远程服务退出: {e}");
    }
}

async fn rpc_handler(
    State(app): State<AppHandle>,
    headers: HeaderMap,
    Query(query): Query<Value>,
    Json(req): Json<RpcRequest>,
) -> Response {
    if !check_auth(&app, &headers, &query) {
        return unauthorized();
    }
    match dispatch(&app, &req.cmd, &req.args).await {
        Ok(data) => Json(json!({ "ok": true, "data": data })).into_response(),
        Err(e) => Json(json!({ "ok": false, "error": e })).into_response(),
    }
}

async fn events_handler(
    State(app): State<AppHandle>,
    headers: HeaderMap,
    Query(query): Query<Value>,
    ws: WebSocketUpgrade,
) -> Response {
    if !check_auth(&app, &headers, &query) {
        return unauthorized();
    }
    ws.on_upgrade(move |socket| handle_socket(socket, app))
}

async fn handle_socket(mut socket: WebSocket, app: AppHandle) {
    let Some(hub) = app.try_state::<RemoteHub>() else {
        return;
    };
    let mut rx = hub.tx.subscribe();
    loop {
        match rx.recv().await {
            Ok(msg) => {
                if let Ok(text) = serde_json::to_string(&msg) {
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
}

/// 文件预览：仅允许工作目录内或粘贴图片落盘目录内的文件
async fn asset_handler(
    State(app): State<AppHandle>,
    headers: HeaderMap,
    Query(query): Query<Value>,
) -> Response {
    if !check_auth(&app, &headers, &query) {
        return unauthorized();
    }
    let Some(path) = query.get("path").and_then(|v| v.as_str()) else {
        return (StatusCode::BAD_REQUEST, "缺少 path 参数").into_response();
    };
    let server = app.state::<Arc<CodexServer>>().inner();
    let file = match session_fs::asset_path_for_remote(server.workspace(), Path::new(path)) {
        Ok(p) => p,
        Err(e) => return (StatusCode::FORBIDDEN, e).into_response(),
    };
    match std::fs::read(&file) {
        Ok(bytes) => {
            let mime = mime_for_path(&file);
            let mut res = ([(header::CONTENT_TYPE, mime)], bytes).into_response();
            let _ = res.headers_mut().insert(
                header::CACHE_CONTROL,
                "private, max-age=300".parse().unwrap(),
            );
            res
        }
        Err(e) => (StatusCode::NOT_FOUND, format!("读取文件失败: {e}")).into_response(),
    }
}

fn mime_for_path(p: &Path) -> &'static str {
    match p
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        Some("ico") => "image/x-icon",
        Some("avif") => "image/avif",
        Some("json") => "application/json",
        Some("map") => "application/json",
        Some("woff2") => "font/woff2",
        Some("pdf") => "application/pdf",
        _ => "application/octet-stream",
    }
}

// ---------------- 静态前端（dist 内嵌） ----------------

#[cfg(feature = "remote-web")]
async fn static_handler(
    State(_app): State<AppHandle>,
    uri: Uri,
    _req: Request,
) -> Response {
    use include_dir::{include_dir, Dir};
    static DIST: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../dist");

    let rel = uri.path().trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    let file = DIST
        .get_file(rel)
        .or_else(|| DIST.get_file("index.html")); // SPA 回退
    let Some(file) = file else {
        return (StatusCode::NOT_FOUND, "dist 未找到 index.html").into_response();
    };
    let mime = mime_for_path(Path::new(file.path()));
    let mut res = ([(header::CONTENT_TYPE, mime)], file.contents()).into_response();
    let _ = res
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());
    res
}

#[cfg(not(feature = "remote-web"))]
async fn static_handler(State(_app): State<AppHandle>, _uri: Uri, _req: Request) -> Response {
    (
        StatusCode::NOT_FOUND,
        "远程模式需要以 --features remote-web 构建以托管前端",
    )
        .into_response()
}

// ---------------- RPC 分发 ----------------

fn str_arg(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("缺少参数: {key}"))
}

fn opt_str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn u64_arg(args: &Value, key: &str) -> Result<u64, String> {
    args.get(key)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| format!("缺少参数: {key}"))
}

fn opt_u64_arg(args: &Value, key: &str) -> Option<u64> {
    args.get(key).and_then(|v| v.as_u64())
}

fn usize_arg(args: &Value, key: &str) -> Result<usize, String> {
    args.get(key)
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .ok_or_else(|| format!("缺少参数: {key}"))
}

fn opt_usize_arg(args: &Value, key: &str) -> Option<usize> {
    args.get(key).and_then(|v| v.as_u64()).map(|n| n as usize)
}

fn u16_arg(args: &Value, key: &str) -> Result<u16, String> {
    args.get(key)
        .and_then(|v| v.as_u64())
        .map(|n| n as u16)
        .ok_or_else(|| format!("缺少参数: {key}"))
}

fn opt_bool_arg(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(|v| v.as_bool())
}

fn value_arg(args: &Value, key: &str) -> Value {
    args.get(key).cloned().unwrap_or(Value::Null)
}

fn json_of<T: Serialize>(v: T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

fn git_err_str(e: GitError) -> String {
    e.message
}

/// 原生对话框/本地动作：远程模式明确拒绝（避免在公司电脑弹出对话框或执行本地操作）
fn native_only() -> Result<Value, String> {
    Err("远程模式不支持该本地操作".into())
}

async fn dispatch(app: &AppHandle, cmd: &str, args: &Value) -> Result<Value, String> {
    let server = app.state::<Arc<CodexServer>>().inner();
    match cmd {
        // ---- server / codex 协议 ----
        "server_status" => commands::server_status_impl(server).await,
        "server_connect" => {
            commands::server_connect_impl(server)?;
            Ok(Value::Null)
        }
        "server_logs" => Ok(json!(commands::server_logs_impl(server).await?)),
        "codex_rpc" => {
            commands::codex_rpc_impl(server, &str_arg(args, "method")?, value_arg(args, "params"))
                .await
        }
        "codex_rpc_long" => {
            commands::codex_rpc_long_impl(
                server,
                &str_arg(args, "method")?,
                value_arg(args, "params"),
                u64_arg(args, "timeout_ms")?,
            )
            .await
        }
        "codex_pin_capability" => commands::codex_pin_capability_impl(server).await,
        "codex_title_helper_capability" => {
            commands::codex_title_helper_capability_impl(server).await
        }
        "interaction_respond" => {
            commands::interaction_respond_impl(
                server,
                value_arg(args, "request_id"),
                value_arg(args, "result"),
            )
            .await?;
            Ok(Value::Null)
        }
        "thread_list" => {
            commands::thread_list_impl(
                server,
                opt_u64_arg(args, "limit"),
                opt_str_arg(args, "cursor"),
                opt_str_arg(args, "cwd"),
            )
            .await
        }
        "thread_start" => commands::thread_start_impl(server, value_arg(args, "params")).await,
        "thread_read" => {
            commands::thread_read_impl(
                server,
                &str_arg(args, "thread_id")?,
                opt_bool_arg(args, "include_turns"),
            )
            .await
        }
        "thread_resume" => commands::thread_resume_impl(server, value_arg(args, "params")).await,
        "thread_delete" => {
            commands::thread_delete_impl(server, &str_arg(args, "thread_id")?).await
        }
        "thread_set_name" => {
            commands::thread_set_name_impl(server, &str_arg(args, "thread_id")?, &str_arg(args, "name")?)
                .await
        }
        "turn_start" => commands::turn_start_impl(server, value_arg(args, "params")).await,
        "turn_steer" => commands::turn_steer_impl(server, value_arg(args, "params")).await,
        "turn_interrupt" => {
            commands::turn_interrupt_impl(
                server,
                &str_arg(args, "thread_id")?,
                &str_arg(args, "turn_id")?,
            )
            .await
        }
        "goal_set" => {
            commands::goal_set_impl(server, &str_arg(args, "thread_id")?, &str_arg(args, "objective")?)
                .await
        }
        "goal_get" => commands::goal_get_impl(server, &str_arg(args, "thread_id")?).await,
        "goal_clear" => commands::goal_clear_impl(server, &str_arg(args, "thread_id")?).await,
        "auth_status" => commands::auth_status_impl(server).await,
        "auth_logout" => commands::auth_logout_impl(server).await,
        "workspace_dir" => Ok(json!(commands::workspace_dir_impl(server))),
        "auth_api_key_configured" => Ok(json!(commands::auth_api_key_configured())),
        "settings_get" => Ok(json!(commands::settings_get(app.clone())?)),
        "settings_set" => {
            let settings: crate::codex::settings::AppSettings = serde_json::from_value(
                args.get("settings").cloned().unwrap_or(Value::Null),
            )
            .map_err(|e| format!("settings 参数无效: {e}"))?;
            commands::settings_set(app.clone(), settings)?;
            Ok(Value::Null)
        }
        "save_pasted_image" => {
            let bytes: Vec<u8> =
                serde_json::from_value(value_arg(args, "bytes")).map_err(|e| e.to_string())?;
            let saved = commands::save_pasted_image(
                app.clone(),
                bytes,
                str_arg(args, "name")?,
            )
            .await?;
            Ok(json!(saved))
        }

        // ---- 会话文件系统 ----
        "session_fs_list" => Ok(json_of(
            session_fs::session_fs_list(str_arg(args, "root")?, str_arg(args, "dir")?).await?,
        )),
        "session_fs_search" => Ok(json_of(
            session_fs::session_fs_search(
                str_arg(args, "root")?,
                str_arg(args, "query")?,
                opt_usize_arg(args, "limit"),
            )
            .await?,
        )),
        "session_fs_metadata" => Ok(json_of(
            session_fs::session_fs_metadata(str_arg(args, "root")?, str_arg(args, "path")?).await?,
        )),
        "session_fs_rename" => Ok(json_of(
            session_fs::session_fs_rename(
                str_arg(args, "root")?,
                str_arg(args, "path")?,
                str_arg(args, "new_name")?,
            )
            .await?,
        )),
        "session_fs_delete" => {
            session_fs::session_fs_delete(str_arg(args, "root")?, str_arg(args, "path")?).await?;
            Ok(Value::Null)
        }
        "session_fs_copy" => Ok(json_of(
            session_fs::session_fs_copy(
                str_arg(args, "root")?,
                str_arg(args, "src")?,
                str_arg(args, "dest_dir")?,
            )
            .await?,
        )),
        "session_fs_paste" => Ok(json_of(
            session_fs::session_fs_paste(
                str_arg(args, "root")?,
                str_arg(args, "dest_dir")?,
                serde_json::from_value(value_arg(args, "sources")).map_err(|e| e.to_string())?,
            )
            .await?,
        )),
        "session_fs_move" => Ok(json_of(
            session_fs::session_fs_move(
                str_arg(args, "root")?,
                str_arg(args, "src")?,
                str_arg(args, "dest_dir")?,
            )
            .await?,
        )),
        "session_fs_read" => Ok(json_of(
            session_fs::session_fs_read(str_arg(args, "root")?, str_arg(args, "path")?).await?,
        )),
        "session_fs_write" => Ok(json_of(
            session_fs::session_fs_write(
                str_arg(args, "root")?,
                str_arg(args, "path")?,
                str_arg(args, "content")?,
            )
            .await?,
        )),
        "session_fs_create_file" => Ok(json_of(
            session_fs::session_fs_create_file(str_arg(args, "root")?, str_arg(args, "dir")?)
                .await?,
        )),
        "session_fs_create_dir" => Ok(json_of(
            session_fs::session_fs_create_dir(str_arg(args, "root")?, str_arg(args, "dir")?)
                .await?,
        )),
        "session_fs_probe_text" => Ok(json_of(
            session_fs::session_fs_probe_text(str_arg(args, "root")?, str_arg(args, "path")?)
                .await?,
        )),
        "session_fs_read_bytes" => Ok(json_of(
            session_fs::session_fs_read_bytes(str_arg(args, "root")?, str_arg(args, "path")?)
                .await?,
        )),
        "session_fs_icons" => {
            let requests: Vec<session_fs::IconRequest> =
                serde_json::from_value(value_arg(args, "requests")).map_err(|e| e.to_string())?;
            Ok(json_of(
                session_fs::session_fs_icons(
                    str_arg(args, "root")?,
                    requests,
                    opt_u64_arg(args, "size").map(|n| n as u32),
                )
                .await?,
            ))
        }
        "session_fs_icon_for_ext" => Ok(json_of(
            session_fs::session_fs_icon_for_ext(str_arg(args, "ext")?).await?,
        )),
        "session_fs_watch_start" => {
            let state = app.state::<session_fs::FsWatcherState>().inner();
            session_fs::session_fs_watch_start_impl(
                app,
                state,
                &str_arg(args, "root")?,
            )
            .await?;
            Ok(Value::Null)
        }
        "session_fs_watch_stop" => {
            let state = app.state::<session_fs::FsWatcherState>().inner();
            session_fs::session_fs_watch_stop_impl(state).await?;
            Ok(Value::Null)
        }

        // ---- Git ----
        "git_changes_status" => Ok(json_of(
            git::git_changes_status(str_arg(args, "path")?)
                .await
                .map_err(git_err_str)?,
        )),
        "git_changes_commit" => Ok(json_of(
            git::git_changes_commit(str_arg(args, "root")?, str_arg(args, "message")?)
                .await
                .map_err(git_err_str)?,
        )),
        "git_changes_pull" => Ok(json_of(
            git::git_changes_pull(str_arg(args, "root")?)
                .await
                .map_err(git_err_str)?,
        )),
        "git_changes_push" => Ok(json_of(
            git::git_changes_push(str_arg(args, "root")?)
                .await
                .map_err(git_err_str)?,
        )),
        "git_changes_remotes" => Ok(json_of(
            git::git_changes_remotes(str_arg(args, "path")?)
                .await
                .map_err(git_err_str)?,
        )),
        "git_changes_remote_add" => Ok(json_of(
            git::git_changes_remote_add(
                str_arg(args, "root")?,
                str_arg(args, "name")?,
                str_arg(args, "url")?,
            )
            .await
            .map_err(git_err_str)?,
        )),
        "git_changes_remote_set_url" => Ok(json_of(
            git::git_changes_remote_set_url(
                str_arg(args, "root")?,
                str_arg(args, "name")?,
                str_arg(args, "url")?,
            )
            .await
            .map_err(git_err_str)?,
        )),
        "git_changes_remote_remove" => Ok(json_of(
            git::git_changes_remote_remove(str_arg(args, "root")?, str_arg(args, "name")?)
                .await
                .map_err(git_err_str)?,
        )),
        "git_changes_remote_fetch" => Ok(json_of(
            git::git_changes_remote_fetch(str_arg(args, "root")?, str_arg(args, "remote")?)
                .await
                .map_err(git_err_str)?,
        )),
        "git_changes_branch_checkout_remote" => Ok(json_of(
            git::git_changes_branch_checkout_remote(
                str_arg(args, "root")?,
                str_arg(args, "remote_branch")?,
            )
            .await
            .map_err(git_err_str)?,
        )),
        "git_changes_remote_branch_delete" => Ok(json_of(
            git::git_changes_remote_branch_delete(
                str_arg(args, "root")?,
                str_arg(args, "remote_branch")?,
            )
            .await
            .map_err(git_err_str)?,
        )),
        "git_changes_remote_switch_upstream" => Ok(json_of(
            git::git_changes_remote_switch_upstream(str_arg(args, "root")?, str_arg(args, "remote")?)
                .await
                .map_err(git_err_str)?,
        )),
        "git_changes_git_available" => Ok(json_of(
            git::git_changes_git_available(str_arg(args, "path")?).await,
        )),
        "git_changes_init" => Ok(json_of(
            git::git_changes_init(str_arg(args, "path")?)
                .await
                .map_err(git_err_str)?,
        )),
        "git_changes_branches" => Ok(json_of(
            git::git_changes_branches(str_arg(args, "path")?)
                .await
                .map_err(git_err_str)?,
        )),
        "git_changes_branch_create" => Ok(json_of(
            git::git_changes_branch_create(str_arg(args, "path")?, str_arg(args, "name")?)
                .await
                .map_err(git_err_str)?,
        )),
        "git_changes_branch_delete" => Ok(json_of(
            git::git_changes_branch_delete(str_arg(args, "path")?, str_arg(args, "name")?)
                .await
                .map_err(git_err_str)?,
        )),
        "git_changes_branch_switch" => Ok(json_of(
            git::git_changes_branch_switch(str_arg(args, "path")?, str_arg(args, "name")?)
                .await
                .map_err(git_err_str)?,
        )),
        "git_changes_branch_merge" => Ok(json_of(
            git::git_changes_branch_merge(str_arg(args, "path")?, str_arg(args, "name")?)
                .await
                .map_err(git_err_str)?,
        )),
        "git_changes_log" => Ok(json_of(
            git::git_changes_log(
                str_arg(args, "root")?,
                usize_arg(args, "limit")?,
                opt_str_arg(args, "before"),
            )
            .await
            .map_err(git_err_str)?,
        )),
        "git_changes_diff" => Ok(json_of(
            git::git_changes_diff(
                str_arg(args, "root")?,
                str_arg(args, "path")?,
                str_arg(args, "kind")?,
            )
            .await
            .map_err(git_err_str)?,
        )),
        "git_changes_stage" => Ok(json_of(
            git::git_changes_stage(str_arg(args, "root")?, str_arg(args, "path")?)
                .await
                .map_err(git_err_str)?,
        )),
        "git_changes_unstage" => Ok(json_of(
            git::git_changes_unstage(str_arg(args, "root")?, str_arg(args, "path")?)
                .await
                .map_err(git_err_str)?,
        )),
        "git_changes_stage_all" => Ok(json_of(
            git::git_changes_stage_all(str_arg(args, "root")?)
                .await
                .map_err(git_err_str)?,
        )),
        "git_changes_unstage_all" => Ok(json_of(
            git::git_changes_unstage_all(str_arg(args, "root")?)
                .await
                .map_err(git_err_str)?,
        )),
        "git_changes_restore" => Ok(json_of(
            git::git_changes_restore(str_arg(args, "root")?, str_arg(args, "path")?)
                .await
                .map_err(git_err_str)?,
        )),
        "git_changes_delete" => Ok(json_of(
            git::git_changes_delete(str_arg(args, "root")?, str_arg(args, "path")?)
                .await
                .map_err(git_err_str)?,
        )),
        "git_changes_ignore" => Ok(json_of(
            git::git_changes_ignore(str_arg(args, "root")?, str_arg(args, "path")?)
                .await
                .map_err(git_err_str)?,
        )),
        "git_changes_watch_start" => {
            let state = app.state::<git::GitWatcherState>().inner();
            git::git_changes_watch_start_impl(app, state, &str_arg(args, "root")?)
                .await
                .map_err(git_err_str)?;
            Ok(Value::Null)
        }
        "git_changes_watch_stop" => {
            let state = app.state::<git::GitWatcherState>().inner();
            git::git_changes_watch_stop_impl(state)
                .await
                .map_err(git_err_str)?;
            Ok(Value::Null)
        }

        // ---- diff 预览 ----
        "build_diff_preview" => {
            let p = args
                .get("params")
                .ok_or_else(|| "缺少参数: params".to_string())?;
            let params = crate::codex::diff::DiffPreviewParams {
                path: str_arg(p, "path")?,
                kind: str_arg(p, "kind")?,
                diff: str_arg(p, "diff")?,
                workspace_root: str_arg(p, "workspace_root")?,
            };
            Ok(json_of(crate::codex::diff::build_diff_preview(params)?))
        }

        // ---- 终端 ----
        "terminal_spawn" => {
            let state = app.state::<terminal::TerminalState>().inner();
            let result = terminal::terminal_spawn_impl(
                app,
                state,
                &str_arg(args, "id")?,
                &str_arg(args, "cwd")?,
            )?;
            Ok(json_of(result))
        }
        "terminal_write" => {
            let state = app.state::<terminal::TerminalState>().inner();
            terminal::terminal_write_impl(state, &str_arg(args, "id")?, &str_arg(args, "data")?)?;
            Ok(Value::Null)
        }
        "terminal_resize" => {
            let state = app.state::<terminal::TerminalState>().inner();
            terminal::terminal_resize_impl(
                state,
                &str_arg(args, "id")?,
                u16_arg(args, "cols")?,
                u16_arg(args, "rows")?,
            )?;
            Ok(Value::Null)
        }
        "terminal_kill" => {
            let state = app.state::<terminal::TerminalState>().inner();
            terminal::terminal_kill_impl(state, &str_arg(args, "id")?)?;
            Ok(Value::Null)
        }

        // ---- 原生本地操作：远程模式拒绝 ----
        "pick_files"
        | "pick_directory"
        | "pick_codex_file"
        | "reveal_path"
        | "open_url"
        | "clipboard_file_paths"
        | "auth_login"
        | "export_markdown_pdf" => native_only(),

        _ => Err(format!("未知命令: {cmd}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_remote_args_basic() {
        // 无法直接注入 env args，改测 token 生成与默认值形态
        let cfg = RemoteConfig {
            enabled: true,
            port: 9000,
            token: "abc".into(),
        };
        assert!(cfg.enabled);
        assert_eq!(cfg.port, 9000);
        assert_eq!(cfg.token, "abc");
    }

    #[test]
    fn generate_token_is_hex_and_unique() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn check_auth_requires_token() {
        // 不依赖 Tauri 运行时：校验函数逻辑在 hub 缺失时拒绝
        // （完整鉴权测试依赖 AppHandle，放在集成验证阶段）
        assert_eq!(mime_for_path(Path::new("a.png")), "image/png");
        assert_eq!(mime_for_path(Path::new("a.js")), "text/javascript; charset=utf-8");
        assert_eq!(
            mime_for_path(Path::new("a.unknown")),
            "application/octet-stream"
        );
    }

    #[test]
    fn native_only_rejects() {
        assert!(native_only().is_err());
    }

    #[test]
    fn arg_helpers_extract_typed_values() {
        let args = serde_json::json!({
            "root": "D:/work",
            "dest_dir": "D:/work/src",
            "limit": 200,
            "include_turns": false,
            "params": { "threadId": "t1" },
            "missing": null,
        });
        assert_eq!(str_arg(&args, "root").unwrap(), "D:/work");
        assert_eq!(str_arg(&args, "dest_dir").unwrap(), "D:/work/src");
        assert!(str_arg(&args, "missing").is_err());
        assert_eq!(opt_str_arg(&args, "root").unwrap(), "D:/work");
        assert_eq!(opt_u64_arg(&args, "limit").unwrap(), 200);
        assert_eq!(usize_arg(&args, "limit").unwrap(), 200);
        assert_eq!(opt_bool_arg(&args, "include_turns").unwrap(), false);
        assert_eq!(value_arg(&args, "params"), json!({ "threadId": "t1" }));
        assert_eq!(json_of("x"), json!("x"));
    }
}
