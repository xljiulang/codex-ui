//! Zen 本地代理：把 codex 的 OpenAI Responses 请求翻译为 OpenCode Zen 的
//! Chat Completions 请求并转发；响应反向翻译回 Responses 格式（含流式 SSE）。
//!
//! - Zen base URL 与 User-Agent 硬编码（模块私有，不暴露到前端）。
//! - API Key 不固定：读取入站请求的 `Authorization` 头原样转发。
//! - 仅绑定回环地址，专供 codex-ui 自身使用，无额外鉴权。

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{OriginalUri, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};
use futures_util::stream::{self, StreamExt};
use serde_json::{Value, json};
use tokio::task::AbortHandle;

use crate::codex::session_log::SessionLog;

/// Zen 本地代理默认端口（可在设置页修改）。
pub(crate) const DEFAULT_ZEN_PROXY_PORT: u16 = 18080;
/// 模拟 opencode 桌面客户端的固定识别头（随请求发往 Zen）。
const OPENCODE_CLIENT: &str = "desktop";
const OPENCODE_PROJECT: &str = "global";
/// OpenCode Zen 默认上游 base URL（可在设置页修改，空/非法时回退此值）。
pub(crate) const DEFAULT_ZEN_BASE_URL: &str = "https://opencode.ai/zen/v1";
/// 请求上游时固定的 User-Agent（与 opencode 官方客户端一致）。
const ZEN_USER_AGENT: &str =
    "opencode/1.18.29 ai-sdk/provider-utils/4.0.23 runtime/node.js/24";
/// 反向代理时不应转发的逐跳请求/响应头。
const HOP_BY_HOP_HEADERS: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// 运行中的代理句柄；`stop()` 终止监听任务。
pub struct ZenProxyHandle {
    pub port: u16,
    abort: AbortHandle,
}

/// Zen 本地代理的诊断日志句柄（复用应用会话日志；None 时不落盘）。
pub(crate) type ZenLog = Option<Arc<SessionLog>>;

impl ZenProxyHandle {
    pub fn stop(&self) {
        self.abort.abort();
    }
}

/// 代理状态（供设置页与 `zen_proxy_status` 命令展示）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ZenProxyStatus {
    pub running: bool,
    pub port: u16,
    pub error: Option<String>,
}

impl ZenProxyStatus {
    pub fn stopped() -> Self {
        Self {
            running: false,
            port: DEFAULT_ZEN_PROXY_PORT,
            error: None,
        }
    }
}

/// 在当前 tokio runtime 上启动本地代理；端口被占用时返回 Err。
pub async fn start(port: u16, base_url: String, log: ZenLog) -> Result<ZenProxyHandle, String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("代理端口 {port} 启动失败（可能被占用）：{e}"))?;
    let base_url = if base_url.trim().is_empty() {
        DEFAULT_ZEN_BASE_URL.to_string()
    } else {
        base_url
    };
    let app = Router::new()
        .route("/v1/responses", post(handle_responses))
        .fallback(handle_passthrough)
        .with_state(ProxyState {
            session: random_id("ses"),
            base_url,
            log,
        });
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(ZenProxyHandle {
        port,
        abort: task.abort_handle(),
    })
}

/// 由 `CodexServer` 调用：按设置启停代理并返回当前状态。
pub async fn apply(
    handle: &mut Option<ZenProxyHandle>,
    enabled: bool,
    port: u16,
    base_url: String,
    log: ZenLog,
) -> ZenProxyStatus {
    let running_port = handle.as_ref().map(|h| h.port);
    let need_start = enabled && running_port != Some(port);
    if !enabled || running_port != Some(port) {
        if let Some(h) = handle.take() {
            h.stop();
        }
    }
    if need_start {
        match start(port, base_url, log).await {
            Ok(h) => {
                *handle = Some(h);
                ZenProxyStatus {
                    running: true,
                    port,
                    error: None,
                }
            }
            Err(e) => ZenProxyStatus {
                running: false,
                port,
                error: Some(e),
            },
        }
    } else if enabled {
        ZenProxyStatus {
            running: true,
            port,
            error: None,
        }
    } else {
        ZenProxyStatus::stopped()
    }
}

// ---------------------------------------------------------------------------
// HTTP 入口
// ---------------------------------------------------------------------------

/// 代理运行期共享状态；`session` 与真实 opencode 客户端一致，在代理生命周期内保持稳定，
/// 作为客户端未提供 `session-id` 请求头时的 `x-opencode-session` 回落值。
#[derive(Clone)]
struct ProxyState {
    session: String,
    base_url: String,
    log: ZenLog,
}

/// 写一条 zen_proxy 诊断日志；句柄为 None 或写盘失败时静默忽略。
fn log_at(log: &Option<Arc<SessionLog>>, level: &str, event: &str, kv: &[(&str, String)]) {
    let Some(log) = log else { return };
    let kv: Vec<(String, String)> = kv
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    log.write(level, None, event, &kv);
}

/// 生成 `prefix_` + 18 位字母数字随机串（无额外依赖，基于 `RandomState` 的随机种子）。
fn random_id(prefix: &str) -> String {
    const CHARS: &[u8] =
        b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut out = String::with_capacity(prefix.len() + 19);
    out.push_str(prefix);
    out.push('_');
    for _ in 0..18 {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(unix_now());
        out.push(CHARS[(h.finish() % 62) as usize] as char);
    }
    out
}

fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(ZEN_USER_AGENT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

async fn handle_responses(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    Json(req): Json<Value>,
) -> Response {
    let want_stream = req
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let model = req
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let input_msg_count = req
        .get("input")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let tool_count = req
        .get("tools")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    log_at(
        &state.log,
        "info",
        "zen_proxy.request",
        &[
            ("model", model),
            ("stream", want_stream.to_string()),
            ("input_msg_count", input_msg_count.to_string()),
            ("tool_count", tool_count.to_string()),
        ],
    );

    let started = Instant::now();
    let base_url = state.base_url.clone();
    match forward(
        &req,
        &headers,
        want_stream,
        &base_url,
        http_client(),
        &state.session,
    )
    .await
    {
        Ok((status, resp)) => {
            log_at(
                &state.log,
                "info",
                "zen_proxy.forward",
                &[
                    ("url", base_url.clone()),
                    ("status", status.as_u16().to_string()),
                    ("elapsed_ms", started.elapsed().as_millis().to_string()),
                ],
            );
            if !status.is_success() {
                return proxy_error_response(status, resp, &state.log).await;
            }
            if want_stream {
                proxy_stream_response(req, resp, &state.log).await
            } else {
                proxy_json_response(req, resp, &state.log).await
            }
        }
        Err((status, msg)) => {
            log_at(
                &state.log,
                "warn",
                "zen_proxy.forward_error",
                &[
                    ("url", base_url),
                    ("error", msg.clone()),
                ],
            );
            error_json(status, msg)
        }
    }
}

/// 非 `/v1/responses` 请求：按 base_url 路径前缀直接透传到 Zen。
async fn handle_passthrough(
    State(state): State<ProxyState>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    forward_passthrough(&state, method, &uri, &headers, body).await
}

/// 透传核心（便于单测）：构造上游 URL、转发请求并流式回传响应。
async fn forward_passthrough(
    state: &ProxyState,
    method: Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Body,
) -> Response {
    let url = match passthrough_url(&state.base_url, uri) {
        Ok(url) => url,
        Err(e) => return error_json(StatusCode::BAD_GATEWAY, e),
    };
    let mut rq = http_client()
        .request(to_reqwest_method(&method), url)
        .headers(forwarded_request_headers(headers, &state.session));
    if method_has_body(&method) {
        rq = rq.body(reqwest::Body::wrap_stream(body.into_data_stream()));
    }
    match rq.send().await {
        Ok(resp) => passthrough_response(resp).await,
        Err(e) => error_json(StatusCode::BAD_GATEWAY, format!("上游请求失败：{e}")),
    }
}

/// 把本地 `/v1` 前缀替换为 base_url 的路径前缀，保留 query。
fn passthrough_url(base_url: &str, uri: &Uri) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(base_url)
        .map_err(|e| format!("Zen 代理转发地址无效: {e}"))?;
    let base_path = url.path().trim_end_matches('/').to_string();
    let incoming = uri.path();
    let suffix = incoming.strip_prefix("/v1").unwrap_or(incoming);
    let mut path = format!("{base_path}{suffix}");
    if path.is_empty() {
        path.push('/');
    }
    url.set_path(&path);
    url.set_query(uri.query());
    url.set_fragment(None);
    Ok(url)
}

/// 上游 `x-opencode-session` 取值：优先用客户端请求头 `session-id`（OpenAI 协议会话标识，
/// codex 每次请求都会带上），统一加 `ses_` 前缀（已带前缀则不重复）；
/// 缺失、空白或非法值时回落到代理生命周期内稳定的 `ses_*`。
fn opencode_session(headers: &HeaderMap, fallback: &str) -> String {
    headers
        .get("session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            if value.starts_with("ses_") {
                value.to_string()
            } else {
                format!("ses_{value}")
            }
        })
        .unwrap_or_else(|| fallback.to_string())
}

/// 构造转发给上游的请求头：保留入站头，剔除逐跳头并附加固定 opencode 识别头。
fn forwarded_request_headers(headers: &HeaderMap, fallback_session: &str) -> HeaderMap {
    let mut out = headers.clone();
    for name in HOP_BY_HOP_HEADERS {
        out.remove(name);
    }
    out.remove(header::HOST);
    out.remove(header::CONTENT_LENGTH);
    // 保持固定 opencode User-Agent，不沿用入站客户端 UA
    out.remove(header::USER_AGENT);
    out.insert(
        "x-opencode-client",
        HeaderValue::from_static(OPENCODE_CLIENT),
    );
    out.insert(
        "x-opencode-project",
        HeaderValue::from_static(OPENCODE_PROJECT),
    );
    if let Ok(value) = HeaderValue::from_str(&random_id("msg")) {
        out.insert("x-opencode-request", value);
    }
    if let Ok(value) = HeaderValue::from_str(&opencode_session(headers, fallback_session)) {
        out.insert("x-opencode-session", value);
    }
    out
}

/// 构造回传给客户端的响应头：剔除逐跳头与长度头，保留内容类型/缓存等。
fn forwarded_response_headers(headers: &HeaderMap) -> HeaderMap {
    let mut out = headers.clone();
    for name in HOP_BY_HOP_HEADERS {
        out.remove(name);
    }
    out.remove(header::CONTENT_LENGTH);
    out
}

fn method_has_body(method: &Method) -> bool {
    method != Method::GET && method != Method::HEAD && method != Method::OPTIONS
}

fn to_reqwest_method(method: &Method) -> reqwest::Method {
    reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET)
}

async fn passthrough_response(resp: reqwest::Response) -> Response {
    let status = resp.status();
    let headers = forwarded_response_headers(resp.headers());
    let stream = resp
        .bytes_stream()
        .map(|chunk| chunk.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)));
    let mut builder = Response::builder().status(status);
    for (name, value) in headers.iter() {
        builder = builder.header(name.clone(), value.clone());
    }
    builder
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| error_json(StatusCode::INTERNAL_SERVER_ERROR, "响应构造失败".into()))
}

/// 翻译请求并转发到 Zen；返回 (状态码, 响应)。
/// `base_url` 由配置传入（测试时可指向本地 mock）。
/// `fallback_session` 仅在客户端未提供 `session-id` 时用作 `x-opencode-session`。
async fn forward(
    req: &Value,
    headers: &HeaderMap,
    want_stream: bool,
    base_url: &str,
    client: &reqwest::Client,
    fallback_session: &str,
) -> Result<(StatusCode, reqwest::Response), (StatusCode, String)> {
    let chat = responses_to_chat(req, want_stream)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("请求翻译失败：{e}")))?;
    let mut rq = client
        .post(format!("{}/chat/completions", base_url.trim_end_matches('/')))
        .json(&chat)
        .header("x-opencode-client", OPENCODE_CLIENT)
        .header("x-opencode-project", OPENCODE_PROJECT)
        .header("x-opencode-request", random_id("msg"))
        .header(
            "x-opencode-session",
            opencode_session(headers, fallback_session),
        );
    if let Some(auth) = headers.get(header::AUTHORIZATION) {
        if let Ok(v) = auth.to_str() {
            rq = rq.header(header::AUTHORIZATION, v.to_string());
        }
    }
    let resp = rq
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("上游请求失败：{e}")))?;
    Ok((resp.status(), resp))
}

/// 非流式：把 Zen 的 chat.completion JSON 翻译为 responses 对象。
async fn proxy_json_response(
    req: Value,
    resp: reqwest::Response,
    log: &Option<Arc<SessionLog>>,
) -> Response {
    let status = resp.status();
    let text = match resp.text().await {
        Ok(t) => t,
        Err(e) => return error_json(StatusCode::BAD_GATEWAY, format!("读取上游响应失败：{e}")),
    };
    let Ok(chat) = serde_json::from_str::<Value>(&text) else {
        log_at(
            log,
            "warn",
            "zen_proxy.response_parse_error",
            &[(
                "detail",
                "上游响应不是合法 JSON：".to_string()
                    + &text.chars().take(200).collect::<String>(),
            )],
        );
        return error_json(
            StatusCode::BAD_GATEWAY,
            "上游响应不是合法 JSON：".to_string() + &text.chars().take(200).collect::<String>(),
        );
    };
    let model = req
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let out = chat_to_responses(&chat, &model);
    let body = serde_json::to_string(&out).unwrap_or_else(|_| "{}".into());
    log_at(
        log,
        "info",
        "zen_proxy.response",
        &[
            ("status", status.as_u16().to_string()),
            ("event_bytes", body.len().to_string()),
        ],
    );
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap_or_else(|_| error_json(StatusCode::INTERNAL_SERVER_ERROR, "响应构造失败".into()))
}

/// 流式：把 Zen 的 SSE data 行翻译为 responses 事件序列（text/event-stream）。
async fn proxy_stream_response(
    req: Value,
    resp: reqwest::Response,
    log: &Option<Arc<SessionLog>>,
) -> Response {
    let model = req
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let response_id = gen_id("resp");
    let created = sse_event("response.created", &json!({
        "type": "response.created",
        "response": {
            "id": response_id,
            "object": "response",
            "created_at": unix_now(),
            "status": "in_progress",
            "model": model,
            "output": [],
            "error": null,
        }
    }));
    let st = StreamState::new(response_id, model);
    let byte_stream = resp.bytes_stream();
    let stream_log = log.clone();
    let events = stream::unfold(
        (byte_stream, Vec::<u8>::new(), st, false, stream_log),
        |(mut bytes, mut buf, mut st, mut done, log)| async move {
            if done {
                return None;
            }
            loop {
                match bytes.next().await {
                    Some(Ok(chunk)) => {
                        buf.extend_from_slice(&chunk);
                        let mut out = Vec::new();
                        while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                            let line: Vec<u8> = buf.drain(..=pos).collect();
                            let line = String::from_utf8_lossy(&line);
                            let line = line.trim_end();
                            if line.is_empty() || !line.starts_with("data:") {
                                continue;
                            }
                            let payload = line["data:".len()..].trim();
                            if payload == "[DONE]" {
                                log_at(&log, "info", "zen_proxy.stream_done", &[]);
                                out.extend(finish_stream(&mut st));
                                done = true;
                                break;
                            }
                            if let Ok(v) = serde_json::from_str::<Value>(payload) {
                                out.extend(process_chunk(&v, &mut st));
                            }
                        }
                        if done || !out.is_empty() {
                            return Some((out, (bytes, buf, st, done, log)));
                        }
                    }
                    Some(Err(_)) => {
                        log_at(&log, "warn", "zen_proxy.stream_error", &[(
                            "detail",
                            "上游读取出错，按完成收尾".to_string(),
                        )]);
                        // 上游读取出错：按已完成收尾，避免客户端悬挂
                        return Some((finish_stream(&mut st), (bytes, buf, st, true, log)));
                    }
                    None => {
                        log_at(&log, "info", "zen_proxy.stream_end", &[(
                            "detail",
                            "上游未发送 [DONE]，补发完成事件".to_string(),
                        )]);
                        // 上游正常结束但未收到 [DONE]（兼容实现差异）：补发完成事件
                        return Some((finish_stream(&mut st), (bytes, buf, st, true, log)));
                    }
                }
            }
        },
    )
    .flat_map(stream::iter);

    let full = stream::once(async move { created })
        .chain(events)
        .map(|s| Ok::<_, std::convert::Infallible>(s.into_bytes()));
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header("Cache-Control", "no-cache")
        .body(Body::from_stream(full))
        .unwrap_or_else(|_| error_json(StatusCode::INTERNAL_SERVER_ERROR, "响应构造失败".into()))
}

/// 上游非 2xx：透传状态码与错误体。
async fn proxy_error_response(
    status: StatusCode,
    resp: reqwest::Response,
    log: &Option<Arc<SessionLog>>,
) -> Response {
    let text = resp.text().await.unwrap_or_default();
    log_at(
        log,
        "warn",
        "zen_proxy.upstream_error",
        &[
            ("status", status.as_u16().to_string()),
            ("detail", text.chars().take(200).collect::<String>()),
        ],
    );
    let body = serde_json::from_str::<Value>(&text).unwrap_or_else(|_| {
        json!({ "error": { "message": text.chars().take(400).collect::<String>(), "type": "upstream_error" } })
    });
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap_or_else(|_| "{}".into())))
        .unwrap_or_else(|_| error_json(StatusCode::INTERNAL_SERVER_ERROR, "响应构造失败".into()))
}

fn error_json(status: StatusCode, msg: String) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&json!({ "error": { "message": msg, "type": "zen_proxy_error" } }))
                .unwrap_or_else(|_| "{}".into()),
        ))
        .unwrap_or_else(|_| Response::new(Body::from("{}")))
}

fn sse_event(name: &str, data: &Value) -> String {
    format!("event: {name}\ndata: {data}\n\n")
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn gen_id(prefix: &str) -> String {
    // 进程内自增 + 时间戳，保证同一会话中稳定递增且跨进程不重复
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let now = unix_now();
    format!("{prefix}_{now}_{n:x}")
}

// ---------------------------------------------------------------------------
// 请求翻译：Responses → Chat Completions
// ---------------------------------------------------------------------------

/// 把 Responses 请求体转换为 Chat Completions 请求体（纯函数，便于单测）。
fn responses_to_chat(req: &Value, want_stream: bool) -> Result<Value, String> {
    if !req.is_object() {
        return Err("请求体必须是 JSON 对象".into());
    }
    let model = req.get("model").cloned().unwrap_or(Value::Null);
    let mut messages: Vec<Value> = Vec::new();

    if let Some(instructions) = req.get("instructions").and_then(|v| v.as_str()) {
        if !instructions.is_empty() {
            messages.push(json!({ "role": "system", "content": instructions }));
        }
    }

    let input = req.get("input");
    match input {
        Some(Value::String(s)) if !s.is_empty() => {
            messages.push(json!({ "role": "user", "content": s }));
        }
        Some(Value::Array(items)) => {
            // 连续的 function_call 聚合成一条 assistant + tool_calls 消息
            let mut pending: Option<Value> = None;
            let flush = |messages: &mut Vec<Value>, pending: &mut Option<Value>| {
                if let Some(p) = pending.take() {
                    messages.push(p);
                }
            };
            for item in items {
                let Some(obj) = item.as_object() else { continue };
                match obj.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                    "message" => {
                        flush(&mut messages, &mut pending);
                        let role = obj
                            .get("role")
                            .and_then(|r| r.as_str())
                            .unwrap_or("user")
                            .to_string();
                        let content = item_content_text(item);
                        messages.push(json!({ "role": role, "content": content }));
                    }
                    "function_call" => {
                        let call_id = obj
                            .get("call_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let name = obj
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let arguments = obj
                            .get("arguments")
                            .and_then(|v| v.as_str())
                            .unwrap_or("{}")
                            .to_string();
                        let tc = json!({
                            "id": call_id,
                            "type": "function",
                            "function": { "name": name, "arguments": arguments }
                        });
                        if let Some(p) = pending.as_mut() {
                            p["tool_calls"]
                                .as_array_mut()
                                .map(|arr| arr.push(tc));
                        } else {
                            pending = Some(json!({
                                "role": "assistant",
                                "content": Value::Null,
                                "tool_calls": [tc]
                            }));
                        }
                    }
                    "function_call_output" => {
                        flush(&mut messages, &mut pending);
                        let call_id = obj
                            .get("call_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let content = value_to_text(obj.get("output").unwrap_or(&Value::Null));
                        messages.push(json!({
                            "role": "tool",
                            "tool_call_id": call_id,
                            "content": content
                        }));
                    }
                    _ => {
                        // reasoning / 其它未知类型：无法映射，忽略
                    }
                }
            }
            flush(&mut messages, &mut pending);
        }
        _ => {}
    }

    let mut chat = json!({
        "model": model,
        "messages": messages,
        "stream": want_stream,
    });

    if let Some(tools) = req.get("tools").and_then(|t| t.as_array()) {
        let chat_tools: Vec<Value> = tools
            .iter()
            .filter_map(|t| {
                let obj = t.as_object()?;
                let name = obj.get("name").and_then(|v| v.as_str())?;
                let mut function = json!({ "name": name });
                if let Some(desc) = obj.get("description").and_then(|v| v.as_str()) {
                    function["description"] = Value::String(desc.to_string());
                }
                if let Some(params) = obj.get("parameters") {
                    function["parameters"] = params.clone();
                }
                Some(json!({ "type": "function", "function": function }))
            })
            .collect();
        if !chat_tools.is_empty() {
            chat["tools"] = Value::Array(chat_tools);
        }
    }

    // 标量映射：Responses 名 → Chat 名；不支持的字段（reasoning/previous_response_id/store 等）不注入
    for (dst, src) in [
        ("max_tokens", "max_output_tokens"),
        ("temperature", "temperature"),
        ("top_p", "top_p"),
        ("seed", "seed"),
    ] {
        if let Some(v) = req.get(src) {
            if !v.is_null() {
                chat[dst] = v.clone();
            }
        }
    }

    Ok(chat)
}

/// 提取 Responses message item 的文本内容（多段 content 拼接为单一字符串）。
fn item_content_text(item: &Value) -> String {
    match item.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| {
                let t = p.get("type").and_then(|x| x.as_str()).unwrap_or("");
                match t {
                    "input_text" | "output_text" => p
                        .get("text")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string()),
                    "input_image" | "input_file" => None,
                    _ => None,
                }
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// 任意 JSON 值 → 聊天文本（工具输出等）。
fn value_to_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// 响应翻译：Chat Completions → Responses
// ---------------------------------------------------------------------------

/// 非流式 chat.completion → responses 对象（纯函数，便于单测）。
fn chat_to_responses(chat: &Value, model: &str) -> Value {
    let chat_id = chat
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("chatcmpl-zen");
    let response_id = format!("resp_{}", chat_id.trim_start_matches("chatcmpl-"));
    let mut output: Vec<Value> = Vec::new();

    if let Some(choices) = chat.get("choices").and_then(|c| c.as_array()) {
        if let Some(first) = choices.first() {
            let msg = first.get("message").cloned().unwrap_or_else(|| json!({}));
            let role = msg
                .get("role")
                .and_then(|r| r.as_str())
                .unwrap_or("assistant")
                .to_string();
            let content = msg
                .get("content")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();
            let mut content_parts = Vec::new();
            if !content.is_empty() {
                content_parts.push(json!({
                    "type": "output_text",
                    "text": content,
                    "annotations": []
                }));
            }
            let msg_id = gen_id("msg");
            output.push(json!({
                "id": msg_id,
                "type": "message",
                "role": role,
                "status": "completed",
                "content": content_parts
            }));
            if let Some(tcs) = msg.get("tool_calls").and_then(|t| t.as_array()) {
                for tc in tcs {
                    let call_id = tc
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = tc
                        .pointer("/function/name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let arguments = tc
                        .pointer("/function/arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("{}")
                        .to_string();
                    output.push(json!({
                        "id": gen_id("fc"),
                        "type": "function_call",
                        "status": "completed",
                        "call_id": call_id,
                        "name": name,
                        "arguments": arguments
                    }));
                }
            }
        }
    }

    let usage = chat.get("usage").cloned().unwrap_or_else(|| json!({}));
    let mut usage_out = json!({});
    if let Some(v) = usage.get("prompt_tokens") {
        usage_out["input_tokens"] = v.clone();
    }
    if let Some(v) = usage.get("completion_tokens") {
        usage_out["output_tokens"] = v.clone();
    }
    if let Some(v) = usage.get("total_tokens") {
        usage_out["total_tokens"] = v.clone();
    }

    json!({
        "id": response_id,
        "object": "response",
        "created_at": unix_now(),
        "status": "completed",
        "model": model,
        "output": output,
        "error": null,
        "usage": usage_out
    })
}

// ---------------------------------------------------------------------------
// 流式翻译：chat.completion.chunk（SSE data 行）→ responses 事件序列
// ---------------------------------------------------------------------------

struct TextTrack {
    item_id: String,
    text_buf: String,
    out_index: usize,
}

struct CallTrack {
    item_id: String,
    call_id: String,
    name: String,
    args_buf: String,
    out_index: usize,
}

struct StreamState {
    response_id: String,
    model: String,
    next_index: usize,
    text: Option<TextTrack>,
    calls: HashMap<usize, CallTrack>,
    closed: bool,
}

impl StreamState {
    fn new(response_id: String, model: String) -> Self {
        Self {
            response_id,
            model,
            next_index: 0,
            text: None,
            calls: HashMap::new(),
            closed: false,
        }
    }
}

/// 处理一个 chat chunk（choice 数组），返回零到多条 responses SSE 块。
fn process_chunk(chunk: &Value, st: &mut StreamState) -> Vec<String> {
    if st.closed {
        return Vec::new();
    }
    let mut out = Vec::new();
    let Some(choices) = chunk.get("choices").and_then(|c| c.as_array()) else {
        return out;
    };
    for choice in choices {
        let delta = choice.get("delta").cloned().unwrap_or_else(|| json!({}));
        // 文本增量
        if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
            if !content.is_empty() {
                out.extend(text_delta(content, st));
            }
        }
        // 工具调用增量
        if let Some(tool_calls) = delta.get("tool_calls").and_then(|t| t.as_array()) {
            for tc in tool_calls {
                let idx = tc
                    .get("index")
                    .and_then(|i| i.as_u64())
                    .unwrap_or(0) as usize;
                out.extend(tool_delta(idx, tc, st));
            }
        }
        // finish_reason 出现即收尾（null/空串跳过）
        if let Some(fr) = choice.get("finish_reason").and_then(|f| f.as_str()) {
            if !fr.is_empty() && fr != "null" {
                out.extend(finish_stream(st));
            }
        }
    }
    out
}

fn text_delta(text: &str, st: &mut StreamState) -> Vec<String> {
    let mut out = Vec::new();
    if st.text.is_none() {
        let out_index = st.next_index;
        st.next_index += 1;
        let item_id = gen_id("msg");
        st.text = Some(TextTrack {
            item_id: item_id.clone(),
            text_buf: String::new(),
            out_index,
        });
        out.push(sse_event(
            "response.output_item.added",
            &json!({
                "type": "response.output_item.added",
                "output_index": out_index,
                "item": {
                    "id": item_id,
                    "type": "message",
                    "role": "assistant",
                    "status": "in_progress",
                    "content": []
                }
            }),
        ));
        out.push(sse_event(
            "response.content_part.added",
            &json!({
                "type": "response.content_part.added",
                "item_id": item_id,
                "output_index": out_index,
                "content_index": 0,
                "part": { "type": "output_text", "text": "", "annotations": [] }
            }),
        ));
    }
    if let Some(t) = st.text.as_mut() {
        t.text_buf.push_str(text);
        out.push(sse_event(
            "response.output_text.delta",
            &json!({
                "type": "response.output_text.delta",
                "item_id": t.item_id,
                "output_index": t.out_index,
                "content_index": 0,
                "delta": text
            }),
        ));
    }
    out
}

fn tool_delta(tool_index: usize, tc: &Value, st: &mut StreamState) -> Vec<String> {
    let mut out = Vec::new();
    if !st.calls.contains_key(&tool_index) {
        let out_index = st.next_index;
        st.next_index += 1;
        let item_id = gen_id("fc");
        let call_id = tc
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let name = tc
            .pointer("/function/name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
            st.calls.insert(
                tool_index,
                CallTrack {
                    item_id: item_id.clone(),
                    call_id: call_id.clone(),
                    name: name.clone(),
                    args_buf: String::new(),
                    out_index,
                },
            );
        out.push(sse_event(
            "response.output_item.added",
            &json!({
                "type": "response.output_item.added",
                "output_index": out_index,
                "item": {
                    "id": item_id,
                    "type": "function_call",
                    "status": "in_progress",
                    "call_id": call_id,
                    "name": name,
                    "arguments": ""
                }
            }),
        ));
        // 首个 chunk 可能已带参数片段（某些实现）
        if let Some(args) = tc.pointer("/function/arguments").and_then(|v| v.as_str()) {
            if !args.is_empty() {
                out.extend(tool_args_delta(tool_index, args, st));
            }
        }
    } else {
        if let Some(args) = tc.pointer("/function/arguments").and_then(|v| v.as_str()) {
            if !args.is_empty() {
                out.extend(tool_args_delta(tool_index, args, st));
            }
        }
    }
    out
}

fn tool_args_delta(tool_index: usize, args: &str, st: &mut StreamState) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(c) = st.calls.get_mut(&tool_index) {
        c.args_buf.push_str(args);
        out.push(sse_event(
            "response.function_call_arguments.delta",
            &json!({
                "type": "response.function_call_arguments.delta",
                "item_id": c.item_id,
                "output_index": c.out_index,
                "delta": args
            }),
        ));
    }
    out
}

/// 收尾：把已开始的 message / function_call 补 done 事件并发送 response.completed。
fn finish_stream(st: &mut StreamState) -> Vec<String> {
    if st.closed {
        return Vec::new();
    }
    st.closed = true;
    let mut out = Vec::new();

    if let Some(t) = st.text.take() {
        out.push(sse_event(
            "response.output_text.done",
            &json!({
                "type": "response.output_text.done",
                "item_id": t.item_id,
                "output_index": t.out_index,
                "content_index": 0,
                "text": t.text_buf,
                "annotations": []
            }),
        ));
        out.push(sse_event(
            "response.content_part.done",
            &json!({
                "type": "response.content_part.done",
                "item_id": t.item_id,
                "output_index": t.out_index,
                "content_index": 0,
                "part": { "type": "output_text", "text": t.text_buf, "annotations": [] }
            }),
        ));
        out.push(sse_event(
            "response.output_item.done",
            &json!({
                "type": "response.output_item.done",
                "output_index": t.out_index,
                "item": {
                    "id": t.item_id,
                    "type": "message",
                    "role": "assistant",
                    "status": "completed",
                    "content": [{
                        "type": "output_text",
                        "text": t.text_buf,
                        "annotations": []
                    }]
                }
            }),
        ));
    }

    let mut calls: Vec<_> = st.calls.drain().collect();
    calls.sort_by_key(|(idx, _)| *idx);
    for (_, c) in calls {
        out.push(sse_event(
            "response.function_call_arguments.done",
            &json!({
                "type": "response.function_call_arguments.done",
                "item_id": c.item_id,
                "output_index": c.out_index,
                "arguments": c.args_buf
            }),
        ));
        out.push(sse_event(
            "response.output_item.done",
            &json!({
                "type": "response.output_item.done",
                "output_index": c.out_index,
                "item": {
                    "id": c.item_id,
                    "type": "function_call",
                    "status": "completed",
                    "call_id": c.call_id,
                    "name": c.name,
                    "arguments": c.args_buf
                }
            }),
        ));
    }

    out.push(sse_event(
        "response.completed",
        &json!({
            "type": "response.completed",
            "response": {
                "id": st.response_id,
                "object": "response",
                "created_at": unix_now(),
                "status": "completed",
                "model": st.model,
                "output": [],
                "error": null
            }
        }),
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sse_data_lines(block: &str) -> Vec<String> {
        block
            .lines()
            .filter(|l| l.starts_with("data:"))
            .map(|l| l["data:".len()..].trim().to_string())
            .collect()
    }

    #[test]
    fn passthrough_url_maps_v1_prefix_and_query() {
        let uri: Uri = "/v1/models?foo=bar".parse().unwrap();
        let url = passthrough_url("https://opencode.ai/zen/v1", &uri).unwrap();
        assert_eq!(url.as_str(), "https://opencode.ai/zen/v1/models?foo=bar");
    }

    #[test]
    fn passthrough_url_maps_custom_base_path() {
        let uri: Uri = "/v1/chat/completions?x=1".parse().unwrap();
        let url = passthrough_url("https://custom.example.com/api/v1/", &uri).unwrap();
        assert_eq!(
            url.as_str(),
            "https://custom.example.com/api/v1/chat/completions?x=1"
        );
    }

    #[test]
    fn passthrough_url_appends_non_v1_path() {
        let uri: Uri = "/health".parse().unwrap();
        let url = passthrough_url("https://example.com/v1", &uri).unwrap();
        assert_eq!(url.as_str(), "https://example.com/v1/health");
    }

    #[test]
    fn passthrough_url_rejects_invalid_base() {
        let uri: Uri = "/v1/models".parse().unwrap();
        assert!(passthrough_url("not a url", &uri).is_err());
    }

    #[test]
    fn responses_to_chat_string_input() {
        let req = json!({
            "model": "zen/gpt-5-mini",
            "input": "你好",
            "stream": false
        });
        let chat = responses_to_chat(&req, false).unwrap();
        assert_eq!(chat["model"], "zen/gpt-5-mini");
        assert_eq!(chat["stream"], false);
        assert_eq!(chat["messages"][0]["role"], "user");
        assert_eq!(chat["messages"][0]["content"], "你好");
    }

    #[test]
    fn responses_to_chat_instructions_and_item_array() {
        let req = json!({
            "model": "m",
            "instructions": "你是一个助手",
            "input": [
                {
                    "type": "message",
                    "role": "user",
                    "content": [
                        { "type": "input_text", "text": "请调用工具" }
                    ]
                },
                {
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "search",
                    "arguments": "{\"q\":\"x\"}"
                },
                {
                    "type": "function_call_output",
                    "call_id": "call_1",
                    "output": { "result": 1 }
                }
            ],
            "tools": [
                { "type": "function", "name": "search", "description": "搜索", "parameters": { "type": "object" } }
            ],
            "max_output_tokens": 100
        });
        let chat = responses_to_chat(&req, true).unwrap();
        assert_eq!(chat["stream"], true);
        assert_eq!(chat["messages"].as_array().unwrap().len(), 4);
        assert_eq!(chat["messages"][0]["role"], "system");
        assert_eq!(chat["messages"][0]["content"], "你是一个助手");
        assert_eq!(chat["messages"][1]["content"], "请调用工具");
        assert_eq!(chat["messages"][2]["role"], "assistant");
        assert_eq!(chat["messages"][2]["tool_calls"][0]["id"], "call_1");
        assert_eq!(chat["messages"][2]["tool_calls"][0]["function"]["name"], "search");
        assert_eq!(chat["messages"][3]["role"], "tool");
        assert_eq!(chat["messages"][3]["tool_call_id"], "call_1");
        assert_eq!(chat["tools"][0]["function"]["name"], "search");
        assert_eq!(chat["max_tokens"], 100);
        // Zen 不支持的字段不注入
        assert!(chat.get("store").is_none());
        assert!(chat.get("reasoning").is_none());
    }

    #[test]
    fn responses_to_chat_ignores_unsupported_fields() {
        let req = json!({
            "model": "m",
            "input": "hi",
            "store": true,
            "reasoning": { "effort": "high" },
            "previous_response_id": "resp_x"
        });
        let chat = responses_to_chat(&req, false).unwrap();
        assert!(chat.get("store").is_none());
        assert!(chat.get("reasoning").is_none());
        assert!(chat.get("previous_response_id").is_none());
    }

    #[test]
    fn chat_to_responses_basic() {
        let chat = json!({
            "id": "chatcmpl-abc",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "好的" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
        });
        let resp = chat_to_responses(&chat, "zen/gpt-5-mini");
        assert_eq!(resp["status"], "completed");
        assert_eq!(resp["model"], "zen/gpt-5-mini");
        assert_eq!(resp["output"][0]["type"], "message");
        assert_eq!(resp["output"][0]["content"][0]["text"], "好的");
        assert_eq!(resp["usage"]["input_tokens"], 10);
        assert_eq!(resp["usage"]["output_tokens"], 5);
    }

    #[test]
    fn chat_to_responses_tool_calls() {
        let chat = json!({
            "id": "chatcmpl-xyz",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_9",
                        "type": "function",
                        "function": { "name": "search", "arguments": "{\"q\":1}" }
                    }]
                }
            }]
        });
        let resp = chat_to_responses(&chat, "m");
        assert_eq!(resp["output"].as_array().unwrap().len(), 2);
        assert_eq!(resp["output"][1]["type"], "function_call");
        assert_eq!(resp["output"][1]["call_id"], "call_9");
        assert_eq!(resp["output"][1]["name"], "search");
        assert_eq!(resp["output"][1]["arguments"], "{\"q\":1}");
    }

    #[test]
    fn stream_text_chunks_produce_events() {
        let mut st = StreamState::new("resp_1".into(), "m".into());
        let c1 = json!({
            "choices": [{ "delta": { "role": "assistant", "content": "你" }, "finish_reason": null }]
        });
        let c2 = json!({
            "choices": [{ "delta": { "content": "好" }, "finish_reason": null }]
        });
        let c3 = json!({
            "choices": [{ "delta": {}, "finish_reason": "stop" }]
        });
        let mut lines: Vec<String> = Vec::new();
        lines.extend(process_chunk(&c1, &mut st));
        lines.extend(process_chunk(&c2, &mut st));
        lines.extend(process_chunk(&c3, &mut st));
        let events: Vec<&str> = lines
            .iter()
            .filter_map(|b| b.lines().next())
            .collect();
        assert_eq!(
            events,
            vec![
                "event: response.output_item.added",
                "event: response.content_part.added",
                "event: response.output_text.delta",
                "event: response.output_text.delta",
                "event: response.output_text.done",
                "event: response.content_part.done",
                "event: response.output_item.done",
                "event: response.completed",
            ]
        );
        // 验证 delta 内容累积
        let deltas: Vec<String> = lines
            .iter()
            .filter(|l| l.contains("response.output_text.delta"))
            .filter_map(|l| {
                let data = l.split("data:").nth(1)?;
                Some(serde_json::from_str::<Value>(data.trim()).ok()?["delta"]
                    .as_str()?
                    .to_string())
            })
            .collect();
        assert_eq!(deltas, vec!["你", "好"]);
    }

    #[test]
    fn stream_tool_calls_produce_function_call_events() {
        let mut st = StreamState::new("resp_2".into(), "m".into());
        let c1 = json!({
            "choices": [{ "delta": { "tool_calls": [{
                "index": 0,
                "id": "call_a",
                "type": "function",
                "function": { "name": "search", "arguments": "" }
            }] }, "finish_reason": null }]
        });
        let c2 = json!({
            "choices": [{ "delta": { "tool_calls": [{
                "index": 0,
                "function": { "arguments": "{\"q\":" }
            }] }, "finish_reason": null }]
        });
        let c3 = json!({
            "choices": [{ "delta": { "tool_calls": [{
                "index": 0,
                "function": { "arguments": "1}" }
            }] }, "finish_reason": "tool_calls" }]
        });
        let mut lines: Vec<String> = Vec::new();
        lines.extend(process_chunk(&c1, &mut st));
        lines.extend(process_chunk(&c2, &mut st));
        lines.extend(process_chunk(&c3, &mut st));
        let joined = lines.join("\n");
        assert!(joined.contains("event: response.output_item.added"));
        assert!(joined.contains(r#""type":"function_call""#));
        assert!(joined.contains("event: response.function_call_arguments.delta"));
        assert!(joined.contains("event: response.function_call_arguments.done"));
        assert!(joined.contains("event: response.completed"));
        // 参数累积正确
        let done = lines
            .iter()
            .find(|l| l.contains("function_call_arguments.done"))
            .unwrap();
        let data = done.split("data:").nth(1).unwrap().trim();
        let v: Value = serde_json::from_str(data).unwrap();
        assert_eq!(v["arguments"], "{\"q\":1}");
    }

    #[test]
    fn sse_line_parsing_handles_done_and_data() {
        let lines = sse_data_lines(&sse_event("x", &json!({"a": 1})));
        assert_eq!(lines, vec!["{\"a\":1}"]);
    }

    #[test]
    fn random_id_matches_opencode_format() {
        let msg = random_id("msg");
        let ses = random_id("ses");
        assert!(msg.starts_with("msg_"));
        assert!(ses.starts_with("ses_"));
        // 前缀后固定 18 位字母数字
        let body = &msg[4..];
        assert_eq!(body.len(), 18);
        assert!(body.chars().all(|c| c.is_ascii_alphanumeric()));
        // 两次调用应不同
        assert_ne!(msg, random_id("msg"));
    }

    #[test]
    fn opencode_session_prefers_client_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "session-id",
            HeaderValue::from_static("3f1a2b3c-4d5e-6f70-8192-a3b4c5d6e7f8"),
        );
        // 客户端值不带 ses_ 前缀时统一补前缀
        assert_eq!(
            opencode_session(&headers, "ses_fixed123"),
            "ses_3f1a2b3c-4d5e-6f70-8192-a3b4c5d6e7f8"
        );

        // 已带前缀则不重复添加
        let mut prefixed = HeaderMap::new();
        prefixed.insert("session-id", HeaderValue::from_static("ses_abc"));
        assert_eq!(opencode_session(&prefixed, "ses_fixed123"), "ses_abc");

        // 前后空白先 trim 再加前缀
        let mut padded = HeaderMap::new();
        padded.insert("session-id", HeaderValue::from_static("  abc  "));
        assert_eq!(opencode_session(&padded, "ses_fixed123"), "ses_abc");

        // 缺失 / 纯空白 / 非可见 ASCII：回落到代理稳定值
        assert_eq!(
            opencode_session(&HeaderMap::new(), "ses_fixed123"),
            "ses_fixed123"
        );
        let mut blank = HeaderMap::new();
        blank.insert("session-id", HeaderValue::from_static("   "));
        assert_eq!(opencode_session(&blank, "ses_fixed123"), "ses_fixed123");
        let mut invalid = HeaderMap::new();
        invalid.insert(
            "session-id",
            HeaderValue::from_bytes(b"caf\xe9").unwrap(),
        );
        assert_eq!(opencode_session(&invalid, "ses_fixed123"), "ses_fixed123");
    }

    #[test]
    fn log_at_writes_safe_events_to_session_log() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        log_at(
            &log,
            "info",
            "zen_proxy.request",
            &[
                ("model", "zen/gpt-5-mini".to_string()),
                ("stream", "true".to_string()),
            ],
        );
        log_at(
            &log,
            "warn",
            "zen_proxy.forward_error",
            &[("error", "上游请求失败：TLS".to_string())],
        );

        let files = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| std::fs::read_to_string(e.path()).unwrap())
            .collect::<Vec<_>>();
        let joined = files.join("\n");
        assert!(joined.contains("event=zen_proxy.request"));
        assert!(joined.contains("event=zen_proxy.forward_error"));
        assert!(joined.contains("model=zen/gpt-5-mini"));
        // 安全字段：不应包含 Authorization / 敏感内容字样
        assert!(!joined.contains("Authorization"));
        assert!(!joined.contains("Bearer"));
    }

    #[test]
    fn log_at_none_or_bad_dir_is_silent() {
        // 无句柄：不 panic
        log_at(&None, "info", "zen_proxy.request", &[]);
        // 日志目录不可写（路径指向一个普通文件）：静默不 panic
        let dir = tempfile::TempDir::new().unwrap();
        let blocker = dir.path().join("blocked");
        std::fs::write(&blocker, b"x").unwrap();
        let log = Some(Arc::new(SessionLog::new(blocker)));
        log_at(&log, "info", "zen_proxy.request", &[]);
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex as AsyncMutex;

    /// 记录一次上游收到的请求（聊身体 + 识别头）。
    #[derive(Debug, Default, Clone)]
    struct Received {
        body: Value,
        authorization: Option<String>,
        user_agent: Option<String>,
        opencode_client: Option<String>,
        opencode_project: Option<String>,
        opencode_request: Option<String>,
        opencode_session: Option<String>,
    }

    #[derive(Debug, Default, Clone)]
    struct PassthroughReceived {
        method: String,
        path: String,
        query: Option<String>,
        body: Value,
        authorization: Option<String>,
        user_agent: Option<String>,
        opencode_client: Option<String>,
        opencode_project: Option<String>,
        opencode_request: Option<String>,
        opencode_session: Option<String>,
    }

    /// 起一个本地 mock Zen 服务，返回监听地址与接收记录。
    async fn spawn_mock_zen(rec: Arc<AsyncMutex<Option<Received>>>) -> String {
        let app = Router::new().route(
            "/chat/completions",
            axum::routing::post(move |headers: HeaderMap, Json(body): Json<Value>| {
                async move {
                    let mut g = rec.lock().await;
                    *g = Some(Received {
                        body,
                        authorization: headers
                            .get(header::AUTHORIZATION)
                            .and_then(|h| h.to_str().ok())
                            .map(|s| s.to_string()),
                        user_agent: headers
                            .get(header::USER_AGENT)
                            .and_then(|h| h.to_str().ok())
                            .map(|s| s.to_string()),
                        opencode_client: headers
                            .get("x-opencode-client")
                            .and_then(|h| h.to_str().ok())
                            .map(|s| s.to_string()),
                        opencode_project: headers
                            .get("x-opencode-project")
                            .and_then(|h| h.to_str().ok())
                            .map(|s| s.to_string()),
                        opencode_request: headers
                            .get("x-opencode-request")
                            .and_then(|h| h.to_str().ok())
                            .map(|s| s.to_string()),
                        opencode_session: headers
                            .get("x-opencode-session")
                            .and_then(|h| h.to_str().ok())
                            .map(|s| s.to_string()),
                    });
                    Json(json!({
                        "id": "chatcmpl-mock",
                        "object": "chat.completion",
                        "created": 0,
                        "model": "m",
                        "choices": [{
                            "index": 0,
                            "message": { "role": "assistant", "content": "mock 回复" },
                            "finish_reason": "stop"
                        }],
                        "usage": { "prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3 }
                    }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    /// 起一个本地 mock Zen 透传端点，返回带 `/v1` 前缀的 base URL。
    async fn spawn_mock_passthrough(
        rec: Arc<AsyncMutex<Option<PassthroughReceived>>>,
    ) -> String {
        let rec_get = rec.clone();
        let app = Router::new()
            .route(
                "/v1/models",
                axum::routing::get(move |headers: HeaderMap, uri: Uri| {
                    let rec = rec_get.clone();
                    async move {
                        *rec.lock().await = Some(PassthroughReceived {
                            method: "GET".into(),
                            path: uri.path().to_string(),
                            query: uri.query().map(str::to_string),
                            body: Value::Null,
                            authorization: headers
                                .get(header::AUTHORIZATION)
                                .and_then(|h| h.to_str().ok())
                                .map(str::to_string),
                            user_agent: headers
                                .get(header::USER_AGENT)
                                .and_then(|h| h.to_str().ok())
                                .map(str::to_string),
                            opencode_client: headers
                                .get("x-opencode-client")
                                .and_then(|h| h.to_str().ok())
                                .map(str::to_string),
                            opencode_project: headers
                                .get("x-opencode-project")
                                .and_then(|h| h.to_str().ok())
                                .map(str::to_string),
                            opencode_request: headers
                                .get("x-opencode-request")
                                .and_then(|h| h.to_str().ok())
                                .map(str::to_string),
                            opencode_session: headers
                                .get("x-opencode-session")
                                .and_then(|h| h.to_str().ok())
                                .map(str::to_string),
                        });
                        Json(json!({
                            "object": "list",
                            "data": [{ "id": "zen-model" }]
                        }))
                    }
                }),
            )
            .route(
                "/v1/echo",
                axum::routing::post(
                    move |headers: HeaderMap, uri: Uri, Json(body): Json<Value>| {
                        let rec = rec.clone();
                        async move {
                            *rec.lock().await = Some(PassthroughReceived {
                                method: "POST".into(),
                                path: uri.path().to_string(),
                                query: uri.query().map(str::to_string),
                                body: body.clone(),
                                authorization: headers
                                    .get(header::AUTHORIZATION)
                                    .and_then(|h| h.to_str().ok())
                                    .map(str::to_string),
                                user_agent: headers
                                    .get(header::USER_AGENT)
                                    .and_then(|h| h.to_str().ok())
                                    .map(str::to_string),
                                opencode_client: headers
                                    .get("x-opencode-client")
                                    .and_then(|h| h.to_str().ok())
                                    .map(str::to_string),
                                opencode_project: headers
                                    .get("x-opencode-project")
                                    .and_then(|h| h.to_str().ok())
                                    .map(str::to_string),
                                opencode_request: headers
                                    .get("x-opencode-request")
                                    .and_then(|h| h.to_str().ok())
                                    .map(str::to_string),
                                opencode_session: headers
                                    .get("x-opencode-session")
                                    .and_then(|h| h.to_str().ok())
                                    .map(str::to_string),
                            });
                            Json(body)
                        }
                    },
                ),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}/v1")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn forwards_chat_with_auth_and_user_agent() {
        let rec: Arc<AsyncMutex<Option<Received>>> = Arc::new(AsyncMutex::new(None));
        let base_url = spawn_mock_zen(rec.clone()).await;

        let req = json!({
            "model": "zen/gpt-5-mini",
            "input": "你好",
            "instructions": "你是助手",
            "stream": false
        });
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer public"),
        );
        let resp = forward(&req, &headers, false, &base_url, http_client(), "ses_fixed123")
            .await
            .expect("forward 应成功");
        assert!(resp.0.is_success());

        // 验证上游收到的内容
        let got = rec.lock().await.clone().expect("mock 应已收到请求");
        assert_eq!(got.authorization.as_deref(), Some("Bearer public"));
        assert_eq!(got.user_agent.as_deref(), Some(ZEN_USER_AGENT));
        assert_eq!(got.body["model"], "zen/gpt-5-mini");
        assert_eq!(got.body["messages"][0]["role"], "system");
        assert_eq!(got.body["messages"][1]["role"], "user");
        assert_eq!(got.body["messages"][1]["content"], "你好");
        assert_eq!(got.body["stream"], false);
        // 模拟 opencode 客户端的识别头
        assert_eq!(got.opencode_client.as_deref(), Some(OPENCODE_CLIENT));
        assert_eq!(got.opencode_project.as_deref(), Some(OPENCODE_PROJECT));
        let req_id = got.opencode_request.as_deref().expect("应有请求 id");
        assert!(req_id.starts_with("msg_"));
        assert_eq!(got.opencode_session.as_deref(), Some("ses_fixed123"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn forwards_chat_prefers_client_session_header() {
        let rec: Arc<AsyncMutex<Option<Received>>> = Arc::new(AsyncMutex::new(None));
        let base_url = spawn_mock_zen(rec.clone()).await;

        let req = json!({ "model": "m", "input": "hi", "stream": false });
        let mut headers = HeaderMap::new();
        headers.insert("session-id", HeaderValue::from_static("3f1a2b3c"));
        let resp = forward(&req, &headers, false, &base_url, http_client(), "ses_fixed123")
            .await
            .expect("forward 应成功");
        assert!(resp.0.is_success());

        let got = rec.lock().await.clone().expect("mock 应已收到请求");
        // 客户端 session-id 优先，并补上 ses_ 前缀
        assert_eq!(got.opencode_session.as_deref(), Some("ses_3f1a2b3c"));
        assert_eq!(got.opencode_client.as_deref(), Some(OPENCODE_CLIENT));
        assert_eq!(got.opencode_project.as_deref(), Some(OPENCODE_PROJECT));
        assert!(got
            .opencode_request
            .as_deref()
            .is_some_and(|id| id.starts_with("msg_")));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn forwards_without_auth_when_client_sends_none() {
        let rec: Arc<AsyncMutex<Option<Received>>> = Arc::new(AsyncMutex::new(None));
        let base_url = spawn_mock_zen(rec.clone()).await;
        let req = json!({ "model": "m", "input": "hi", "stream": false });
        let headers = HeaderMap::new();
        let resp = forward(&req, &headers, false, &base_url, http_client(), "ses_fixed123")
            .await
            .expect("forward 应成功");
        assert!(resp.0.is_success());
        let got = rec.lock().await.clone().expect("mock 应已收到请求");
        assert_eq!(got.authorization, None);
        assert_eq!(got.user_agent.as_deref(), Some(ZEN_USER_AGENT));
        assert_eq!(got.opencode_request.as_deref().map(|s| &s[..4]), Some("msg_"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn passthrough_forwards_models_and_post_body() {
        let rec: Arc<AsyncMutex<Option<PassthroughReceived>>> =
            Arc::new(AsyncMutex::new(None));
        let base_url = spawn_mock_passthrough(rec.clone()).await;
        let state = ProxyState {
            session: "ses_fixed123".into(),
            base_url,
            log: None,
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer public"),
        );
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );

        let uri: Uri = "/v1/models?foo=bar".parse().unwrap();
        let resp =
            forward_passthrough(&state, Method::GET, &uri, &headers, Body::empty()).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&body).contains("zen-model"));
        let got = rec.lock().await.clone().expect("mock 应已收到 GET 请求");
        assert_eq!(got.method, "GET");
        assert_eq!(got.path, "/v1/models");
        assert_eq!(got.query.as_deref(), Some("foo=bar"));
        assert_eq!(got.authorization.as_deref(), Some("Bearer public"));
        assert_eq!(got.user_agent.as_deref(), Some(ZEN_USER_AGENT));
        assert_eq!(got.opencode_client.as_deref(), Some(OPENCODE_CLIENT));
        assert_eq!(got.opencode_project.as_deref(), Some(OPENCODE_PROJECT));
        assert_eq!(got.opencode_session.as_deref(), Some("ses_fixed123"));
        assert_eq!(
            got.opencode_request.as_deref().map(|s| &s[..4]),
            Some("msg_")
        );

        let uri: Uri = "/v1/echo".parse().unwrap();
        let resp = forward_passthrough(
            &state,
            Method::POST,
            &uri,
            &headers,
            Body::from(r#"{"a":1}"#),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let got = rec.lock().await.clone().expect("mock 应已收到 POST 请求");
        assert_eq!(got.method, "POST");
        assert_eq!(got.path, "/v1/echo");
        assert_eq!(got.body["a"], 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn passthrough_prefers_client_session_header() {
        let rec: Arc<AsyncMutex<Option<PassthroughReceived>>> =
            Arc::new(AsyncMutex::new(None));
        let base_url = spawn_mock_passthrough(rec.clone()).await;
        let state = ProxyState {
            session: "ses_fixed123".into(),
            base_url,
            log: None,
        };
        let mut headers = HeaderMap::new();
        headers.insert("session-id", HeaderValue::from_static("client-session-42"));

        let uri: Uri = "/v1/models".parse().unwrap();
        let resp =
            forward_passthrough(&state, Method::GET, &uri, &headers, Body::empty()).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let got = rec.lock().await.clone().expect("mock 应已收到 GET 请求");
        assert_eq!(
            got.opencode_session.as_deref(),
            Some("ses_client-session-42")
        );
    }
}
