//! Zen 本地代理：把 codex 的 OpenAI Responses 请求翻译为 OpenCode Zen 的
//! Chat Completions 请求并转发；响应反向翻译回 Responses 格式（含流式 SSE）。
//!
//! - Zen base URL 与 User-Agent 硬编码（模块私有，不暴露到前端）。
//! - API Key 不固定：读取入站请求的 `Authorization` 头原样转发。
//! - 仅绑定回环地址，专供 codex-ui 自身使用，无额外鉴权。

use std::collections::{BTreeSet, HashMap, HashSet};
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{OriginalUri, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::Response;
use axum::Router;
#[cfg(test)]
use axum::Json;
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

/// 畸形工具调用被拦截时写入 `response.failed` 的错误码。
const CODE_MALFORMED_TOOL_CALL: &str = "malformed_tool_call_arguments";
/// 上游随流下发 `error`、或读取中断时写入 `response.failed` 的错误码。
const CODE_UPSTREAM_STREAM_ERROR: &str = "upstream_stream_error";
/// 补齐悬空工具调用时代入的工具结果文本。
const MISSING_TOOL_OUTPUT_TEXT: &str =
    "该工具调用未执行（参数非法或调用被中止），请重新发起。";
/// `finish_reason` 之后等待尾部分片（usage 等）的宽限；超时按现状收尾，避免挂住回合。
const USAGE_GRACE: Duration = Duration::from_secs(3);
/// `zen_proxy.stream_summary` 里记录的 delta 键上限（去重后按字典序取前若干个）。
const DELTA_KEY_LIMIT: usize = 16;
/// 翻译分支的请求体上限：长会话实测已近 1MB，axum 默认 2MB 会撞 413。
const RESPONSES_BODY_LIMIT: usize = 64 * 1024 * 1024;
/// 端口刚被上一实例释放时的绑定重试次数与间隔（同端口换上游会立即重启代理）。
const BIND_RETRY: u32 = 20;
const BIND_RETRY_INTERVAL: Duration = Duration::from_millis(50);

/// 运行中的代理句柄；`stop()` 终止监听任务。
pub struct ZenProxyHandle {
    pub port: u16,
    /// 生效中的上游 base_url（已归一化，用于判断是否需要重启）。
    base_url: String,
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

/// 归一化上游 base_url：去首尾空白、空值回退默认地址、去掉末尾所有 `/`。
/// `https://a/` 与 `https://a`、`https://a/zen/v2/` 与 `https://a/zen/v2` 因此等价，
/// 既影响翻译路由的派生，也影响「上游地址是否变化」的重启判定。
fn normalize_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        DEFAULT_ZEN_BASE_URL.to_string()
    } else {
        trimmed.to_string()
    }
}

/// 翻译入口路径 = 上游 base_url 的路径 + `/responses`（base_url 无路径时为 `/responses`）。
/// codex 对 provider 的 base_url 追加 `/responses`，因此本地 provider 的路径需与提供方一致。
fn responses_path(base_url: &str) -> String {
    let base = normalize_base_url(base_url);
    let path = reqwest::Url::parse(&base)
        .map(|url| url.path().trim_end_matches('/').to_string())
        .unwrap_or_default();
    format!("{path}/responses")
}

/// 在当前 tokio runtime 上启动本地代理；端口被占用时返回 Err。
pub async fn start(port: u16, base_url: String, log: ZenLog) -> Result<ZenProxyHandle, String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut attempt = 0;
    let listener = loop {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => break listener,
            // 同端口换上游会先停旧实例再立即绑定，旧监听可能尚未释放，短暂重试。
            Err(e) => {
                if attempt >= BIND_RETRY {
                    return Err(format!("代理端口 {port} 启动失败（可能被占用）：{e}"));
                }
                attempt += 1;
                tokio::time::sleep(BIND_RETRY_INTERVAL).await;
            }
        }
    };
    let base_url = normalize_base_url(&base_url);
    let app = Router::new()
        .fallback(handle_any)
        .with_state(ProxyState {
            session: random_id("ses"),
            base_url: base_url.clone(),
            log,
        });
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(ZenProxyHandle {
        port,
        base_url,
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
    let base_url = normalize_base_url(&base_url);
    // 端口与上游地址都没变（含 `https://a/` 与 `https://a` 这类等价写法）才复用现有实例。
    let unchanged = handle
        .as_ref()
        .is_some_and(|h| h.port == port && h.base_url == base_url);
    let need_start = enabled && !unchanged;
    if !enabled || !unchanged {
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

/// `zen_proxy.request` 的诊断列（纯函数，便于单测）：体积（提示词 / 历史字符数）
/// 用于判断是否逼近上游窗口；`reasoning_effort` 记录 codex 本次实际请求的推理强度
/// （`-` 表示没请求推理），排查「思考过程为空」时先看这一格。
fn request_log_fields(req: &Value, want_stream: bool) -> Vec<(&'static str, String)> {
    let model = req
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let input_msg_count = req
        .get("input")
        .and_then(Value::as_array)
        .map(|items| items.len())
        .unwrap_or(0);
    let tool_count = req
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| tools.len())
        .unwrap_or(0);
    let instructions_chars = req
        .get("instructions")
        .and_then(Value::as_str)
        .map(|text| text.chars().count())
        .unwrap_or(0);
    let input_chars = req
        .get("input")
        .and_then(|value| serde_json::to_string(value).ok())
        .map(|text| text.chars().count())
        .unwrap_or(0);
    let reasoning_effort = req
        .pointer("/reasoning/effort")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("-")
        .to_string();
    vec![
        ("model", model),
        ("stream", want_stream.to_string()),
        ("input_msg_count", input_msg_count.to_string()),
        ("input_chars", input_chars.to_string()),
        ("instructions_chars", instructions_chars.to_string()),
        ("reasoning_effort", reasoning_effort),
        ("tool_count", tool_count.to_string()),
    ]
}

/// 统一入口：`POST {上游 base_url 路径}/responses` 走 Responses→Chat 翻译，其余路径/方法透传。
async fn handle_any(
    State(state): State<ProxyState>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let expected = responses_path(&state.base_url);
    if method == Method::POST && uri.path() == expected {
        return handle_responses(&state, &headers, body).await;
    }
    // 客户端把 `/responses` 打到了别的路径：本地 provider 的 base_url 路径与上游不一致。
    if uri.path().ends_with("/responses") {
        log_at(
            &state.log,
            "warn",
            "zen_proxy.path_unmatched",
            &[
                ("method", method.to_string()),
                ("path", uri.path().to_string()),
                ("expected", expected),
            ],
        );
    }
    forward_passthrough(&state, method, &uri, &headers, body).await
}

/// 把请求体读成 JSON 后交给翻译路径；读取或解析失败时返回明确错误。
async fn handle_responses(state: &ProxyState, headers: &HeaderMap, body: Body) -> Response {
    let bytes = match axum::body::to_bytes(body, RESPONSES_BODY_LIMIT).await {
        Ok(bytes) => bytes,
        Err(e) => {
            return error_json(
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("请求体过大或读取失败：{e}"),
            );
        }
    };
    let req: Value = match serde_json::from_slice(&bytes) {
        Ok(req) => req,
        Err(e) => {
            return error_json(
                StatusCode::BAD_REQUEST,
                format!("请求体不是合法 JSON：{e}"),
            );
        }
    };
    let want_stream = req
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let fields = request_log_fields(&req, want_stream);
    let kv: Vec<(&str, String)> = fields.iter().map(|(k, v)| (*k, v.clone())).collect();
    log_at(&state.log, "info", "zen_proxy.request", &kv);

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
        Ok(forwarded) => {
            let status = forwarded.status;
            log_at(
                &state.log,
                "info",
                "zen_proxy.forward",
                &[
                    ("url", base_url.clone()),
                    ("status", status.as_u16().to_string()),
                    ("elapsed_ms", started.elapsed().as_millis().to_string()),
                    (
                        "dropped_fields",
                        if forwarded.dropped_fields.is_empty() {
                            "-".to_string()
                        } else {
                            forwarded.dropped_fields.join(",")
                        },
                    ),
                ],
            );
            if forwarded.repairs.invalid_arguments > 0 || forwarded.repairs.missing_tool_outputs > 0 {
                log_at(
                    &state.log,
                    "warn",
                    "zen_proxy.history_repaired",
                    &[
                        (
                            "invalid_arguments",
                            forwarded.repairs.invalid_arguments.to_string(),
                        ),
                        (
                            "missing_tool_outputs",
                            forwarded.repairs.missing_tool_outputs.to_string(),
                        ),
                    ],
                );
            }
            if !forwarded.dropped_fields.is_empty() {
                log_at(
                    &state.log,
                    "warn",
                    "zen_proxy.optional_fields_dropped",
                    &[
                        ("fields", forwarded.dropped_fields.join(",")),
                        (
                            "detail",
                            "上游指名拒绝这些可选字段，已摘除后重试一次".to_string(),
                        ),
                    ],
                );
            }
            match forwarded.payload {
                ForwardPayload::Text(text) => proxy_error_text(status, &text, &state.log),
                ForwardPayload::Live(resp) => {
                    if !status.is_success() {
                        return proxy_error_response(status, resp, &state.log).await;
                    }
                    if want_stream {
                        proxy_stream_response(req, resp, &state.log).await
                    } else {
                        proxy_json_response(req, resp, &state.log).await
                    }
                }
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

/// 透传核心（便于单测）：构造上游 URL、转发请求并流式回传响应。
/// 只替换 host，路径与 query 原样发出——本地 provider 的 base_url 路径需与提供方一致。
async fn forward_passthrough(
    state: &ProxyState,
    method: Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Body,
) -> Response {
    let url = match passthrough_url(&state.base_url, uri) {
        Ok(url) => url,
        Err(e) => {
            log_passthrough(state, &method, uri.path(), "error");
            return error_json(StatusCode::BAD_GATEWAY, e);
        }
    };
    let mut rq = http_client()
        .request(to_reqwest_method(&method), url)
        .headers(forwarded_request_headers(headers, &state.session));
    if method_has_body(&method) {
        rq = rq.body(reqwest::Body::wrap_stream(body.into_data_stream()));
    }
    match rq.send().await {
        Ok(resp) => {
            let status = resp.status();
            log_passthrough(state, &method, uri.path(), &status.as_u16().to_string());
            passthrough_response(resp).await
        }
        Err(e) => {
            log_passthrough(state, &method, uri.path(), "error");
            error_json(StatusCode::BAD_GATEWAY, format!("上游请求失败：{e}"))
        }
    }
}

/// 透传日志：2xx 记 info、其余记 warn，配合 `zen_proxy.path_unmatched` 判断请求走了哪条分支。
fn log_passthrough(state: &ProxyState, method: &Method, path: &str, status: &str) {
    let level = if status.starts_with('2') { "info" } else { "warn" };
    log_at(
        &state.log,
        level,
        "zen_proxy.passthrough",
        &[
            ("method", method.to_string()),
            ("path", path.to_string()),
            ("status", status.to_string()),
        ],
    );
}

/// 透传目标 URL：scheme/host/port 取自 base_url，入站 path 与 query 原样发出（零路径转换）。
fn passthrough_url(base_url: &str, uri: &Uri) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(&normalize_base_url(base_url))
        .map_err(|e| format!("Zen 代理转发地址无效: {e}"))?;
    url.set_path(uri.path());
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

/// 上游响应载体：2xx 保留可流式读取的响应；4xx 为判定 `stream_options` 兼容性
/// 读出错误体时直接携带文本，避免二次读取。
enum ForwardPayload {
    Live(reqwest::Response),
    Text(String),
}

/// 一次转发的完整结果：状态、响应载体、历史修复统计、被摘掉的可选字段。
struct Forwarded {
    status: StatusCode,
    payload: ForwardPayload,
    repairs: RepairReport,
    dropped_fields: Vec<&'static str>,
}

/// 可选请求字段：取到值才加入请求体；上游指名拒绝时按需摘除后重试一次。
struct OptionalField {
    /// 写入 chat 请求体的字段名。
    key: &'static str,
    /// 上游错误体里指认该字段的关键词（小写比较）。
    needles: &'static [&'static str],
    value: Value,
}

/// 本次请求要带上的可选字段：流式用量 + 推理强度等透传字段。
fn optional_fields(req: &Value, want_stream: bool) -> Vec<OptionalField> {
    let mut out: Vec<OptionalField> = Vec::new();
    if want_stream {
        out.push(OptionalField {
            key: "stream_options",
            needles: &["stream_options", "include_usage"],
            value: json!({ "include_usage": true }),
        });
    }
    if let Some(effort) = req
        .pointer("/reasoning/effort")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|effort| !effort.is_empty())
    {
        out.push(OptionalField {
            key: "reasoning_effort",
            needles: &["reasoning_effort", "reasoning effort"],
            value: json!(effort),
        });
    }
    if let Some(v) = req.get("parallel_tool_calls").filter(|v| v.is_boolean()) {
        out.push(OptionalField {
            key: "parallel_tool_calls",
            needles: &["parallel_tool_calls"],
            value: v.clone(),
        });
    }
    if let Some(v) = req
        .get("prompt_cache_key")
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
    {
        out.push(OptionalField {
            key: "prompt_cache_key",
            needles: &["prompt_cache_key"],
            value: json!(v),
        });
    }
    if let Some(v) = req
        .get("service_tier")
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
    {
        out.push(OptionalField {
            key: "service_tier",
            needles: &["service_tier"],
            value: json!(v),
        });
    }
    if let Some(choice) = tool_choice_to_chat(req.get("tool_choice")) {
        out.push(OptionalField {
            key: "tool_choice",
            needles: &["tool_choice"],
            value: choice,
        });
    }
    out
}

/// Responses 的 `tool_choice` → chat 形态：字符串直接透传，
/// `{type:"function",name}` 转 `{type:"function",function:{name}}`，
/// 其余（`allowed_tools` 等无 chat 等价物）返回 None。
fn tool_choice_to_chat(choice: Option<&Value>) -> Option<Value> {
    let choice = choice.filter(|v| !v.is_null())?;
    match choice {
        Value::String(s) if !s.trim().is_empty() => Some(json!(s)),
        Value::Object(object) if object.get("type").and_then(Value::as_str) == Some("function") => {
            let name = object.get("name").and_then(Value::as_str)?;
            Some(json!({ "type": "function", "function": { "name": name } }))
        }
        _ => None,
    }
}

/// 错误体是否指认了该可选字段。
fn mentions_field(text: &str, needles: &[&str]) -> bool {
    let lower = text.to_ascii_lowercase();
    needles.iter().any(|needle| lower.contains(needle))
}

/// 翻译请求并转发到 Zen（含历史净化与可选字段降级重试）。
/// `base_url` 由配置传入（测试时可指向本地 mock）。
/// `fallback_session` 仅在客户端未提供 `session-id` 时用作 `x-opencode-session`。
async fn forward(
    req: &Value,
    headers: &HeaderMap,
    want_stream: bool,
    base_url: &str,
    client: &reqwest::Client,
    fallback_session: &str,
) -> Result<Forwarded, (StatusCode, String)> {
    let (chat, repairs) = responses_to_chat(req, want_stream)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("请求翻译失败：{e}")))?;
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let optional = optional_fields(req, want_stream);
    let mut dropped: Vec<&'static str> = Vec::new();
    let mut attempt = 0usize;
    loop {
        let mut body = chat.clone();
        for field in optional.iter() {
            if dropped.contains(&field.key) {
                continue;
            }
            body[field.key] = field.value.clone();
        }
        let resp = build_chat_request(client, &url, headers, fallback_session, &body)
            .send()
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, format!("上游请求失败：{e}")))?;
        let status = resp.status();
        // 上游以 4xx 指名拒绝某个可选字段时摘掉它重试一次（最多一次）；
        // 未指名任何可选字段（如参数校验失败）不重试，避免重复发请求。
        if attempt == 0 && status.is_client_error() {
            let text = resp.text().await.unwrap_or_default();
            let hit: Vec<&'static str> = optional
                .iter()
                .filter(|field| {
                    !dropped.contains(&field.key) && mentions_field(&text, field.needles)
                })
                .map(|field| field.key)
                .collect();
            if !hit.is_empty() {
                dropped.extend(hit);
                attempt = 1;
                continue;
            }
            return Ok(Forwarded {
                status,
                payload: ForwardPayload::Text(text),
                repairs,
                dropped_fields: Vec::new(),
            });
        }
        return Ok(Forwarded {
            status,
            payload: ForwardPayload::Live(resp),
            repairs,
            dropped_fields: dropped,
        });
    }
}

/// 构造发往 Zen 的 chat/completions 请求（固定识别头 + 透传的 Authorization）。
fn build_chat_request(
    client: &reqwest::Client,
    url: &str,
    headers: &HeaderMap,
    fallback_session: &str,
    body: &Value,
) -> reqwest::RequestBuilder {
    let mut rq = client
        .post(url)
        .json(body)
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
    rq
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
    let out = match chat_to_responses(&chat, &model) {
        Ok(out) => out,
        Err(e) => {
            log_at(
                log,
                "warn",
                "zen_proxy.malformed_tool_call",
                &[("reason", e.clone())],
            );
            return error_json(StatusCode::BAD_GATEWAY, e);
        }
    };
    if let Some(usage) = chat.get("usage").filter(|v| !v.is_null()) {
        log_at(
            log,
            "info",
            "zen_proxy.usage",
            &[("detail", truncate_chars(&usage.to_string(), 300))],
        );
    }
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
        (
            byte_stream,
            Vec::<u8>::new(),
            st,
            false,
            stream_log,
            None::<Instant>,
        ),
        |(mut bytes, mut buf, mut st, mut done, log, mut finish_deadline)| async move {
            if done {
                return None;
            }
            loop {
                // finish_reason 之后还要再等尾部分片（OpenAI 的 `include_usage`
                // 把 usage 放在最后一个独立分片里）；最多等 USAGE_GRACE。
                let next = if st.finish_reason.is_some() {
                    let deadline = *finish_deadline
                        .get_or_insert_with(|| Instant::now() + USAGE_GRACE);
                    match tokio::time::timeout(
                        deadline.saturating_duration_since(Instant::now()),
                        bytes.next(),
                    )
                    .await
                    {
                        Ok(item) => item,
                        Err(_) => {
                            log_at(
                                &log,
                                "warn",
                                "zen_proxy.usage_timeout",
                                &[(
                                    "detail",
                                    "finish_reason 后未再收到分片，按现状收尾".to_string(),
                                )],
                            );
                            return Some((
                                finish_stream(&mut st, &log),
                                (bytes, buf, st, true, log, None),
                            ));
                        }
                    }
                } else {
                    bytes.next().await
                };
                match next {
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
                                out.extend(finish_stream(&mut st, &log));
                                done = true;
                                break;
                            }
                            if let Ok(v) = serde_json::from_str::<Value>(payload) {
                                out.extend(process_chunk(&v, &mut st));
                                // 上游随流下发 error：立即按失败收尾，不再继续读
                                if st.failure.is_some() && !st.closed {
                                    out.extend(finish_stream(&mut st, &log));
                                    done = true;
                                    break;
                                }
                            }
                        }
                        if done || !out.is_empty() {
                            return Some((
                                out,
                                (bytes, buf, st, done, log, finish_deadline),
                            ));
                        }
                    }
                    Some(Err(_)) => {
                        // 上游读取出错：按失败收尾（原先按完成收尾会让回合静默结束）
                        if st.failure.is_none() {
                            st.failure = Some(StreamFailure {
                                code: CODE_UPSTREAM_STREAM_ERROR.to_string(),
                                message: "上游连接中断，本轮未正常结束；请重试".to_string(),
                                detail: "上游读取出错（连接中断或超时）".to_string(),
                            });
                        }
                        return Some((
                            finish_stream(&mut st, &log),
                            (bytes, buf, st, true, log, None),
                        ));
                    }
                    None => {
                        log_at(&log, "info", "zen_proxy.stream_end", &[(
                            "detail",
                            "上游未发送 [DONE]，补发完成事件".to_string(),
                        )]);
                        // 上游正常结束但未收到 [DONE]（兼容实现差异）：补发完成事件
                        return Some((
                            finish_stream(&mut st, &log),
                            (bytes, buf, st, true, log, None),
                        ));
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
    proxy_error_text(status, &text, log)
}

/// 上游非 2xx 且错误体已读出：透传状态码与错误体。
fn proxy_error_text(status: StatusCode, text: &str, log: &Option<Arc<SessionLog>>) -> Response {
    log_at(
        log,
        "warn",
        "zen_proxy.upstream_error",
        &[
            ("status", status.as_u16().to_string()),
            ("detail", text.chars().take(200).collect::<String>()),
        ],
    );
    let body = serde_json::from_str::<Value>(text).unwrap_or_else(|_| {
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

/// 历史净化统计：被改写的非法工具参数条数、补出的悬空工具结果条数。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct RepairReport {
    invalid_arguments: usize,
    missing_tool_outputs: usize,
}

/// 把 Responses 请求体转换为 Chat Completions 请求体（纯函数，便于单测）。
///
/// 同时净化历史：工具调用参数被截断/非法时改写为 `{}`，并有工具调用却缺少
/// 对应结果时补一条合成 `tool` 消息——Chat Completions 要求二者严格配对，
/// 否则整条会话会被上游以 400/500 永久拒绝。
fn responses_to_chat(req: &Value, want_stream: bool) -> Result<(Value, RepairReport), String> {
    if !req.is_object() {
        return Err("请求体必须是 JSON 对象".into());
    }
    let mut repairs = RepairReport::default();
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
                        let content = message_content(item);
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
                        let raw_arguments = obj
                            .get("arguments")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let (arguments, repaired) = normalize_arguments(raw_arguments);
                        if repaired {
                            repairs.invalid_arguments += 1;
                        }
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
            repair_tool_pairing(&mut messages, &mut repairs);
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

    Ok((chat, repairs))
}

/// 规范化工具调用参数：空/缺失统一写 `{}`（无参工具），非空但非法 JSON 也写 `{}`
/// 并计入修复（返回值第二项表示"发生了修复"）。
fn normalize_arguments(raw: &str) -> (String, bool) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ("{}".to_string(), false);
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Object(_)) => (trimmed.to_string(), false),
        // 数组合法等非对象形态同样视为不可用：Chat Completions 要求 object
        Ok(_) | Err(_) => ("{}".to_string(), true),
    }
}

/// 校验一条工具调用的参数是否可用（流式侧护栏，判定"畸形"）。
/// 工具名必须非空；`arguments` 允许为空串（无参工具），非空时必须是合法 JSON 对象。
fn validate_call_arguments(name: &str, args: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("工具名为空".into());
    }
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Object(_)) => Ok(()),
        Ok(other) => Err(format!(
            "工具参数不是 JSON 对象：{}",
            truncate_chars(&other.to_string(), 80)
        )),
        Err(e) => Err(format!("工具参数不是合法 JSON：{e}")),
    }
}

/// 为"有工具调用却没有结果"的 assistant 消息补出合成 `tool` 消息。
fn repair_tool_pairing(messages: &mut Vec<Value>, repairs: &mut RepairReport) {
    let mut i = 0usize;
    while i < messages.len() {
        let ids: Vec<String> = messages[i]
            .get("tool_calls")
            .and_then(|t| t.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|tc| {
                        tc.get("id").and_then(|v| v.as_str()).map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();
        if ids.is_empty() {
            i += 1;
            continue;
        }
        // 紧随其后的连续 tool 消息即这组调用的结果
        let mut j = i + 1;
        let mut answered: HashSet<String> = HashSet::new();
        while j < messages.len()
            && messages[j].get("role").and_then(|r| r.as_str()) == Some("tool")
        {
            if let Some(id) = messages[j].get("tool_call_id").and_then(|v| v.as_str()) {
                answered.insert(id.to_string());
            }
            j += 1;
        }
        let missing: Vec<String> = ids
            .into_iter()
            .filter(|id| !answered.contains(id))
            .collect();
        if missing.is_empty() {
            i = j;
            continue;
        }
        let count = missing.len();
        for (offset, id) in missing.into_iter().enumerate() {
            messages.insert(
                j + offset,
                json!({
                    "role": "tool",
                    "tool_call_id": id,
                    "content": MISSING_TOOL_OUTPUT_TEXT
                }),
            );
        }
        repairs.missing_tool_outputs += count;
        i = j + count;
    }
}

/// 按字符截断（UTF-8 安全），用于日志与错误摘要。
fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    value.chars().take(max_chars).collect::<String>() + "…"
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

/// 消息内容：纯文本消息仍用字符串（最大兼容）；含图片时改为 chat 多模态 parts。
/// Responses 的 `input_image` 是 `{type,image_url:<字符串>,detail?}`，chat 需要
/// `{type:"image_url",image_url:{url,detail?}}`；`detail` 仅透传 auto/low/high。
fn message_content(item: &Value) -> Value {
    let Some(parts) = item.get("content").and_then(Value::as_array) else {
        return Value::String(item_content_text(item));
    };
    let has_image = parts
        .iter()
        .any(|part| part.get("type").and_then(Value::as_str) == Some("input_image"));
    if !has_image {
        return Value::String(item_content_text(item));
    }
    let mut out: Vec<Value> = Vec::new();
    for part in parts {
        match part.get("type").and_then(Value::as_str).unwrap_or("") {
            "input_text" | "output_text" => {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        out.push(json!({ "type": "text", "text": text }));
                    }
                }
            }
            "input_image" => {
                let Some(url) = part
                    .get("image_url")
                    .and_then(Value::as_str)
                    .filter(|url| !url.trim().is_empty())
                else {
                    continue;
                };
                let mut image = json!({ "url": url });
                if let Some(detail) = part.get("detail").and_then(Value::as_str) {
                    if matches!(detail, "auto" | "low" | "high") {
                        image["detail"] = json!(detail);
                    }
                }
                out.push(json!({ "type": "image_url", "image_url": image }));
            }
            // input_file / input_audio：chat 侧形态差异较大，本次不透传
            _ => {}
        }
    }
    if out.is_empty() {
        return Value::String(item_content_text(item));
    }
    Value::Array(out)
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
/// 工具调用参数不可用时返回 Err（调用方以 502 结束，而不是把坏参数交给 codex）。
fn chat_to_responses(chat: &Value, model: &str) -> Result<Value, String> {
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
                    let raw_arguments = tc
                        .pointer("/function/arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let (arguments, repaired) = normalize_arguments(raw_arguments);
                    if repaired {
                        return Err(format!(
                            "上游工具调用参数不可用（{}）：{}",
                            if name.is_empty() { "未知工具" } else { &name },
                            truncate_chars(raw_arguments.trim(), 120)
                        ));
                    }
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

    let usage_out = usage_to_responses(chat.get("usage"));

    Ok(json!({
        "id": response_id,
        "object": "response",
        "created_at": unix_now(),
        "status": "completed",
        "model": model,
        "output": output,
        "error": null,
        "usage": usage_out
    }))
}

/// chat 的 `usage` → responses 的 `usage`（流式与非流式共用同一组字段名）。
/// 除三个总量外还翻译明细：`input_tokens_details.cached_tokens` /
/// `input_tokens_details.cache_write_tokens` / `output_tokens_details.reasoning_tokens`
/// ——codex 用它们填充 `cached_input_tokens` / `cache_write_input_tokens` /
/// `reasoning_output_tokens`（应用侧「缓存读取 / 缓存写入 / 推理输出」三行）。
fn usage_to_responses(usage: Option<&Value>) -> Value {
    let mut out = json!({});
    let Some(usage) = usage else {
        return out;
    };
    for (dst, src) in [
        ("input_tokens", "prompt_tokens"),
        ("output_tokens", "completion_tokens"),
        ("total_tokens", "total_tokens"),
    ] {
        if let Some(v) = usage.get(src).filter(|v| !v.is_null()) {
            out[dst] = v.clone();
        }
    }

    let mut input_details = json!({});
    if let Some(v) = pick_number(
        usage,
        &["prompt_tokens_details.cached_tokens", "cache_read_input_tokens"],
    ) {
        input_details["cached_tokens"] = v;
    }
    if let Some(v) = pick_number(
        usage,
        &[
            "cache_write_tokens",
            "prompt_tokens_details.cache_write_tokens",
            "cache_creation_input_tokens",
        ],
    ) {
        input_details["cache_write_tokens"] = v;
    }
    if input_details.as_object().is_some_and(|map| !map.is_empty()) {
        out["input_tokens_details"] = input_details;
    }

    let mut output_details = json!({});
    if let Some(v) = pick_number(
        usage,
        &["completion_tokens_details.reasoning_tokens", "reasoning_tokens"],
    ) {
        output_details["reasoning_tokens"] = v;
    }
    if output_details.as_object().is_some_and(|map| !map.is_empty()) {
        out["output_tokens_details"] = output_details;
    }
    out
}

/// 按路径取数值：支持 `a` 与 `a.b`；非数值、缺失、null 都视为未命中。
fn pick_number(value: &Value, paths: &[&str]) -> Option<Value> {
    for path in paths {
        let found = match path.split_once('.') {
            Some((head, tail)) => value.get(head).and_then(|v| v.get(tail)),
            None => value.get(*path),
        };
        if let Some(v) = found.filter(|v| v.is_number()) {
            return Some(v.clone());
        }
    }
    None
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
    /// 推理摘要（上游 reasoning 增量）；codex 用它渲染「思考过程」。
    reasoning: Option<TextTrack>,
    text: Option<TextTrack>,
    calls: HashMap<usize, CallTrack>,
    closed: bool,
    /// 已判定的失败（上游随流下发的 error / 读取中断）；畸形工具调用在收尾时判定。
    failure: Option<StreamFailure>,
    /// 上游给出的结束原因（`finish_reason`），仅用于观测。
    finish_reason: Option<String>,
    /// 上游随流下发的 token 用量，收尾时映射进 `response.completed`。
    usage: Option<Value>,
    /// 本次流里出现过的 delta 顶层键（去重、有上限），仅用于诊断。
    delta_keys: BTreeSet<String>,
}

impl StreamState {
    fn new(response_id: String, model: String) -> Self {
        Self {
            response_id,
            model,
            next_index: 0,
            reasoning: None,
            text: None,
            calls: HashMap::new(),
            closed: false,
            failure: None,
            finish_reason: None,
            usage: None,
            delta_keys: BTreeSet::new(),
        }
    }
}

/// 一次流内失败：`code` 写入 `response.failed`，`detail` 仅进日志。
#[derive(Debug, Clone)]
struct StreamFailure {
    code: String,
    message: String,
    detail: String,
}

/// 处理一个 chat chunk（choice 数组），返回零到多条 responses SSE 块。
fn process_chunk(chunk: &Value, st: &mut StreamState) -> Vec<String> {
    if st.closed {
        return Vec::new();
    }
    // 上游随流下发的 error（HTTP 仍为 200）：不再静默忽略，直接判定失败
    if let Some(err) = chunk.get("error").filter(|v| !v.is_null()) {
        if st.failure.is_none() {
            let message = err
                .get("message")
                .and_then(|m| m.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| "上游流内返回错误".to_string());
            st.failure = Some(StreamFailure {
                code: CODE_UPSTREAM_STREAM_ERROR.to_string(),
                message,
                detail: truncate_chars(&err.to_string(), 300),
            });
        }
        return Vec::new();
    }
    // token 用量（部分实现随任意 chunk 或末块下发）
    if let Some(usage) = chunk.get("usage").filter(|v| !v.is_null()) {
        st.usage = Some(usage.clone());
    }
    let mut out = Vec::new();
    let Some(choices) = chunk.get("choices").and_then(|c| c.as_array()) else {
        return out;
    };
    for choice in choices {
        let delta = choice.get("delta").cloned().unwrap_or_else(|| json!({}));
        // 诊断：记录出现过的 delta 键名（只记键，不记值）
        if let Some(object) = delta.as_object() {
            for key in object.keys() {
                if st.delta_keys.len() >= DELTA_KEY_LIMIT {
                    break;
                }
                st.delta_keys.insert(key.clone());
            }
        }
        // 推理增量（DeepSeek 系 reasoning_content / 中转 reasoning）：翻成
        // responses 的 reasoning summary 事件，供「思考过程」卡片展示
        if let Some(text) = reasoning_text(&delta) {
            out.extend(reasoning_delta(text, st));
        }
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
                if st.finish_reason.is_none() {
                    st.finish_reason = Some(fr.to_string());
                }
                // 不在此收尾：OpenAI 的 `include_usage` 会在 finish_reason 之后
                // 再发一个只带 usage 的分片，提前关流会把它丢掉（见流循环的宽限）。
            }
        }
    }
    out
}

/// 取增量里的推理文本。覆盖常见形态：
/// - `reasoning_content`：DeepSeek 系；
/// - `reasoning`：字符串，或带 `content` / `text` / `summary` 的对象；
/// - `reasoning_details`：OpenRouter 系的数组（取 `text` / `summary`，跳过加密项）。
fn reasoning_text(delta: &Value) -> Option<&str> {
    if let Some(text) = delta
        .get("reasoning_content")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        return Some(text);
    }
    match delta.get("reasoning") {
        Some(Value::String(text)) if !text.is_empty() => Some(text.as_str()),
        Some(Value::Object(object)) => reasoning_field(object),
        _ => delta
            .get("reasoning_details")
            .and_then(Value::as_array)
            .and_then(|details| {
                details.iter().find_map(|detail| {
                    let object = detail.as_object()?;
                    // 加密推理没有可展示文本
                    if object
                        .get("type")
                        .and_then(Value::as_str)
                        .is_some_and(|kind| kind.contains("encrypted"))
                    {
                        return None;
                    }
                    reasoning_field(object)
                })
            }),
    }
}

/// 从推理对象里按 `content` / `text` / `summary` 顺序取非空文本。
fn reasoning_field(object: &serde_json::Map<String, Value>) -> Option<&str> {
    ["content", "text", "summary"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .filter(|text| !text.is_empty())
}

/// 推理摘要增量 → `response.reasoning_summary_*` 事件序列（summary_index 固定 0）。
fn reasoning_delta(text: &str, st: &mut StreamState) -> Vec<String> {
    let mut out = Vec::new();
    if st.reasoning.is_none() {
        let out_index = st.next_index;
        st.next_index += 1;
        let item_id = gen_id("rs");
        st.reasoning = Some(TextTrack {
            item_id: item_id.clone(),
            text_buf: String::new(),
            out_index,
        });
        out.push(sse_event(
            "response.output_item.added",
            &json!({
                "type": "response.output_item.added",
                "output_index": out_index,
                "item": { "id": item_id, "type": "reasoning", "summary": [] }
            }),
        ));
        out.push(sse_event(
            "response.reasoning_summary_part.added",
            &json!({
                "type": "response.reasoning_summary_part.added",
                "item_id": item_id,
                "output_index": out_index,
                "summary_index": 0,
                "part": { "type": "summary_text", "text": "" }
            }),
        ));
    }
    if let Some(t) = st.reasoning.as_mut() {
        t.text_buf.push_str(text);
        out.push(sse_event(
            "response.reasoning_summary_text.delta",
            &json!({
                "type": "response.reasoning_summary_text.delta",
                "item_id": t.item_id,
                "output_index": t.out_index,
                "summary_index": 0,
                "delta": text
            }),
        ));
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

/// 收尾：文本 item 照常补 done 事件；工具调用正常时补 function_call 的 done 事件
/// 并发 `response.completed`（带 usage）；若上游已失败或存在畸形工具调用，则不发任何
/// function_call、改发 `response.failed`，让回合以明确错误结束而不是静默完成。
/// 无论哪条路径都记一次 `zen_proxy.stream_summary`。
fn finish_stream(st: &mut StreamState, log: &ZenLog) -> Vec<String> {
    if st.closed {
        return Vec::new();
    }
    st.closed = true;
    let mut out = Vec::new();
    let reasoning_chars = st
        .reasoning
        .as_ref()
        .map(|t| t.text_buf.chars().count())
        .unwrap_or(0);
    let text_chars = st
        .text
        .as_ref()
        .map(|t| t.text_buf.chars().count())
        .unwrap_or(0);

    // 推理摘要 item 先收尾（它的 output_index 先分配）
    if let Some(t) = st.reasoning.take() {
        out.push(sse_event(
            "response.reasoning_summary_text.done",
            &json!({
                "type": "response.reasoning_summary_text.done",
                "item_id": t.item_id,
                "output_index": t.out_index,
                "summary_index": 0,
                "text": t.text_buf
            }),
        ));
        out.push(sse_event(
            "response.reasoning_summary_part.done",
            &json!({
                "type": "response.reasoning_summary_part.done",
                "item_id": t.item_id,
                "output_index": t.out_index,
                "summary_index": 0,
                "part": { "type": "summary_text", "text": t.text_buf }
            }),
        ));
        out.push(sse_event(
            "response.output_item.done",
            &json!({
                "type": "response.output_item.done",
                "output_index": t.out_index,
                "item": {
                    "id": t.item_id,
                    "type": "reasoning",
                    "summary": [{ "type": "summary_text", "text": t.text_buf }]
                }
            }),
        ));
    }

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
    let call_count = calls.len();

    // 上游随流错误：补一条日志（错误详情只在检测处写入状态）
    if let Some(f) = st.failure.as_ref() {
        log_at(
            log,
            "warn",
            "zen_proxy.stream_error",
            &[("code", f.code.clone()), ("detail", f.detail.clone())],
        );
    }

    // 畸形工具调用：整轮判失败，且不发出任何 function_call——既避免半执行，
    // 也避免把非法 JSON 参数写进会话历史（那会让后续每个请求都被上游拒绝）。
    let mut malformed: Option<StreamFailure> = None;
    if st.failure.is_none() {
        for (_, c) in calls.iter() {
            let Err(reason) = validate_call_arguments(&c.name, &c.args_buf) else {
                continue;
            };
            log_at(
                log,
                "warn",
                "zen_proxy.malformed_tool_call",
                &[
                    ("name", c.name.clone()),
                    ("call_id", c.call_id.clone()),
                    ("args_len", c.args_buf.chars().count().to_string()),
                    ("preview", truncate_chars(c.args_buf.trim(), 200)),
                    ("reason", reason.clone()),
                ],
            );
            malformed = Some(StreamFailure {
                code: CODE_MALFORMED_TOOL_CALL.to_string(),
                message: format!(
                    "上游模型返回的工具调用参数不可用（{}），已阻止该调用进入会话；请重试本轮",
                    if c.name.trim().is_empty() {
                        "未知工具"
                    } else {
                        c.name.trim()
                    }
                ),
                detail: reason,
            });
            break;
        }
    }
    let failure = st.failure.clone().or(malformed);
    let failed = failure.is_some();
    // 被上游截断（finish_reason=length）：照常收尾，但以 incomplete 结束而不是 completed
    let truncated = st.finish_reason.as_deref() == Some("length");

    if failure.is_none() {
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
    }

    if let Some(f) = failure {
        // codex 的 SSE 解析器认识 response.failed，会以明确的失败结束本回合
        out.push(sse_event(
            "response.failed",
            &json!({
                "type": "response.failed",
                "response": {
                    "id": st.response_id,
                    "object": "response",
                    "created_at": unix_now(),
                    "status": "failed",
                    "model": st.model,
                    "output": [],
                    "error": { "code": f.code, "message": f.message }
                }
            }),
        ));
    } else if truncated {
        let mut response = json!({
            "id": st.response_id,
            "object": "response",
            "created_at": unix_now(),
            "status": "incomplete",
            "model": st.model,
            "output": [],
            "error": null,
            "incomplete_details": { "reason": "max_output_tokens" }
        });
        if let Some(usage) = st.usage.as_ref() {
            response["usage"] = usage_to_responses(Some(usage));
        }
        out.push(sse_event(
            "response.incomplete",
            &json!({ "type": "response.incomplete", "response": response }),
        ));
    } else {
        let mut response = json!({
            "id": st.response_id,
            "object": "response",
            "created_at": unix_now(),
            "status": "completed",
            "model": st.model,
            "output": [],
            "error": null
        });
        if let Some(usage) = st.usage.as_ref() {
            response["usage"] = usage_to_responses(Some(usage));
        }
        out.push(sse_event(
            "response.completed",
            &json!({ "type": "response.completed", "response": response }),
        ));
    }

    if let Some(usage) = st.usage.as_ref() {
        log_at(
            log,
            "info",
            "zen_proxy.usage",
            &[("detail", truncate_chars(&usage.to_string(), 300))],
        );
    }

    log_at(
        log,
        "info",
        "zen_proxy.stream_summary",
        &[
            (
                "finish_reason",
                st.finish_reason.clone().unwrap_or_default(),
            ),
            ("reasoning_chars", reasoning_chars.to_string()),
            ("text_chars", text_chars.to_string()),
            ("call_count", call_count.to_string()),
            ("failed", failed.to_string()),
            (
                "usage",
                if st.usage.is_some() { "present" } else { "none" }.to_string(),
            ),
            (
                "delta_keys",
                st.delta_keys
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(","),
            ),
        ],
    );
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
    fn passthrough_url_keeps_incoming_path_and_query_verbatim() {
        // 本地 provider 与上游同路径（都是 /zen/v1）→ 原样透传。
        let uri: Uri = "/zen/v1/models?foo=bar".parse().unwrap();
        let url = passthrough_url("https://opencode.ai/zen/v1", &uri).unwrap();
        assert_eq!(url.as_str(), "https://opencode.ai/zen/v1/models?foo=bar");

        // 上游无路径（如 DeepSeek 官方 base_url）→ 入站 /models 原样转发。
        let uri: Uri = "/models".parse().unwrap();
        let url = passthrough_url("https://api.deepseek.com/", &uri).unwrap();
        assert_eq!(url.as_str(), "https://api.deepseek.com/models");

        // 上游带自定义路径、入站同路径（含尾斜杠写法）→ 原样透传。
        let uri: Uri = "/api/v1/chat/completions?x=1".parse().unwrap();
        let url = passthrough_url("https://custom.example.com/api/v1/", &uri).unwrap();
        assert_eq!(
            url.as_str(),
            "https://custom.example.com/api/v1/chat/completions?x=1"
        );

        // 两侧路径不一致（上游 /v1、入站 /health）也不做任何转换，原样发出。
        let uri: Uri = "/health".parse().unwrap();
        let url = passthrough_url("https://example.com/v1", &uri).unwrap();
        assert_eq!(url.as_str(), "https://example.com/health");
    }

    #[test]
    fn normalize_base_url_trims_whitespace_and_trailing_slashes() {
        assert_eq!(normalize_base_url("https://a/"), "https://a");
        assert_eq!(normalize_base_url("https://a"), "https://a");
        assert_eq!(normalize_base_url("https://a///"), "https://a");
        assert_eq!(
            normalize_base_url("  https://a/zen/v2/  "),
            "https://a/zen/v2"
        );
        assert_eq!(normalize_base_url(""), DEFAULT_ZEN_BASE_URL);
        assert_eq!(normalize_base_url("   "), DEFAULT_ZEN_BASE_URL);
        assert_eq!(normalize_base_url("/"), DEFAULT_ZEN_BASE_URL);
    }

    #[test]
    fn responses_path_follows_base_url_path() {
        assert_eq!(responses_path("https://api.deepseek.com"), "/responses");
        assert_eq!(responses_path("https://api.deepseek.com/"), "/responses");
        assert_eq!(responses_path("https://opencode.ai/zen/v1"), "/zen/v1/responses");
        assert_eq!(responses_path("https://opencode.ai/zen/v2/"), "/zen/v2/responses");
        assert_eq!(responses_path("https://a/v1"), "/v1/responses");
        // 空/非法地址回退默认上游，派生出默认路径。
        assert_eq!(
            responses_path(""),
            responses_path(DEFAULT_ZEN_BASE_URL)
        );
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
        let (chat, repairs) = responses_to_chat(&req, false).unwrap();
        assert_eq!(repairs, RepairReport::default(), "纯文本历史不应触发净化");
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
        let (chat, repairs) = responses_to_chat(&req, true).unwrap();
        assert_eq!(repairs, RepairReport::default(), "配对完整的工具调用不应触发净化");
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
        let (chat, repairs) = responses_to_chat(&req, false).unwrap();
        assert_eq!(repairs, RepairReport::default());
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
        let resp = chat_to_responses(&chat, "zen/gpt-5-mini").unwrap();
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
        let resp = chat_to_responses(&chat, "m").unwrap();
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
        // finish_reason 只记录，收尾由流循环在尾部（usage 分片/结束）触发
        lines.extend(finish_stream(&mut st, &None));
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
        lines.extend(finish_stream(&mut st, &None));
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

    /// 从 SSE 块里取事件名序列。
    fn event_names(lines: &[String]) -> Vec<String> {
        lines
            .iter()
            .filter_map(|l| l.lines().next())
            .map(|l| l.trim_start_matches("event: ").to_string())
            .collect()
    }

    /// 取指定事件的 data 载荷。
    fn event_payload(lines: &[String], name: &str) -> Option<Value> {
        lines.iter().find_map(|l| {
            let mut it = l.lines();
            let head = it.next()?;
            if head.trim_start_matches("event: ") != name {
                return None;
            }
            let data = it.next()?.trim_start_matches("data:").trim();
            serde_json::from_str::<Value>(data).ok()
        })
    }

    #[test]
    fn malformed_tool_arguments_fail_the_stream() {
        let mut st = StreamState::new("resp_bad".into(), "m".into());
        let c1 = json!({
            "choices": [{ "delta": { "content": "我直接改文件" }, "finish_reason": null }]
        });
        // 真实故障形态：参数被截断
        let c2 = json!({
            "choices": [{ "delta": { "tool_calls": [{
                "index": 0,
                "id": "call_bad",
                "type": "function",
                "function": { "name": "apply_patch", "arguments": "{\"old_string\": " }
            }] }, "finish_reason": "tool_calls" }]
        });
        let mut lines = process_chunk(&c1, &mut st);
        lines.extend(process_chunk(&c2, &mut st));
        lines.extend(finish_stream(&mut st, &None));

        let names = event_names(&lines);
        assert!(names.contains(&"response.failed".to_string()));
        assert!(!names.contains(&"response.completed".to_string()));
        // 坏调用不得以完成态进入会话：`output_item.added` 是增量协议里先发出去的，
        // 但参数完成事件与 function_call 的 `output_item.done` 都不得发出。
        assert!(!names.contains(&"response.function_call_arguments.done".to_string()));
        assert!(!lines
            .iter()
            .any(|l| l.contains("output_item.done") && l.contains("\"type\":\"function_call\"")));
        // 文本照常收尾：用户仍能看到模型已产出的话
        assert!(lines
            .iter()
            .any(|l| l.contains("output_item.done") && l.contains("\"type\":\"message\"")));

        let failed = event_payload(&lines, "response.failed").expect("应发出 response.failed");
        assert_eq!(failed["response"]["status"], "failed");
        assert_eq!(failed["response"]["error"]["code"], CODE_MALFORMED_TOOL_CALL);
        assert!(failed["response"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("apply_patch"));
    }

    #[test]
    fn empty_tool_arguments_still_complete() {
        let mut st = StreamState::new("resp_noargs".into(), "m".into());
        let c1 = json!({
            "choices": [{ "delta": { "tool_calls": [{
                "index": 0,
                "id": "call_a",
                "type": "function",
                "function": { "name": "get_time", "arguments": "" }
            }] }, "finish_reason": "tool_calls" }]
        });
        let mut lines = process_chunk(&c1, &mut st);
        lines.extend(finish_stream(&mut st, &None));
        let names = event_names(&lines);
        assert!(names.contains(&"response.completed".to_string()));
        assert!(!names.contains(&"response.failed".to_string()));
        assert!(names.contains(&"response.function_call_arguments.done".to_string()));
    }

    #[test]
    fn stream_error_payload_fails_the_stream() {
        let mut st = StreamState::new("resp_err".into(), "m".into());
        let chunk = json!({ "error": { "message": "Internal server error", "type": "error" } });
        assert!(process_chunk(&chunk, &mut st).is_empty());
        assert!(st.failure.is_some(), "流内 error 应被记为失败");

        let lines = finish_stream(&mut st, &None);
        let names = event_names(&lines);
        assert!(names.contains(&"response.failed".to_string()));
        assert!(!names.contains(&"response.completed".to_string()));
        let failed = event_payload(&lines, "response.failed").unwrap();
        assert_eq!(failed["response"]["error"]["code"], CODE_UPSTREAM_STREAM_ERROR);
        assert!(failed["response"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Internal server error"));
    }

    #[test]
    fn stream_usage_is_mapped_into_completed() {
        let mut st = StreamState::new("resp_usage".into(), "m".into());
        let c1 = json!({
            "choices": [{ "delta": { "content": "好" }, "finish_reason": null }]
        });
        let c2 = json!({
            "choices": [],
            "usage": { "prompt_tokens": 120000, "completion_tokens": 30, "total_tokens": 120030 }
        });
        let c3 = json!({ "choices": [{ "delta": {}, "finish_reason": "stop" }] });
        let mut lines = process_chunk(&c1, &mut st);
        lines.extend(process_chunk(&c2, &mut st));
        lines.extend(process_chunk(&c3, &mut st));
        lines.extend(finish_stream(&mut st, &None));

        let done = event_payload(&lines, "response.completed").expect("应正常完成");
        assert_eq!(done["response"]["usage"]["input_tokens"], 120000);
        assert_eq!(done["response"]["usage"]["output_tokens"], 30);
        assert_eq!(done["response"]["usage"]["total_tokens"], 120030);
    }

    #[test]
    fn responses_to_chat_repairs_invalid_arguments() {
        let req = json!({
            "model": "m",
            "input": [
                {
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "apply_patch",
                    "arguments": "{\"old_string\": "
                }
            ]
        });
        let (chat, repairs) = responses_to_chat(&req, true).unwrap();
        assert_eq!(repairs.invalid_arguments, 1);
        assert_eq!(
            chat["messages"][0]["tool_calls"][0]["function"]["arguments"],
            "{}"
        );
    }

    #[test]
    fn responses_to_chat_synthesizes_missing_tool_output() {
        let req = json!({
            "model": "m",
            "input": [
                {
                    "type": "function_call",
                    "call_id": "call_9",
                    "name": "apply_patch",
                    "arguments": "{\"a\":1}"
                }
            ]
        });
        let (chat, repairs) = responses_to_chat(&req, true).unwrap();
        assert_eq!(repairs.missing_tool_outputs, 1);
        let messages = chat["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(messages[1]["tool_call_id"], "call_9");
        assert_eq!(messages[1]["content"], MISSING_TOOL_OUTPUT_TEXT);
    }

    #[test]
    fn chat_to_responses_rejects_invalid_arguments() {
        let chat = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_bad",
                        "type": "function",
                        "function": { "name": "apply_patch", "arguments": "{\"old_string\": " }
                    }]
                }
            }]
        });
        let err = chat_to_responses(&chat, "m").unwrap_err();
        assert!(err.contains("apply_patch"), "错误信息应含工具名：{err}");
    }

    #[test]
    fn optional_field_rejection_is_detected_by_parameter_name() {
        let stream_options = ["stream_options", "include_usage"];
        assert!(mentions_field(
            "{\"error\":{\"message\":\"Unrecognized request argument supplied: stream_options\"}}",
            &stream_options
        ));
        assert!(mentions_field(
            "include_usage is not supported by this model",
            &stream_options
        ));
        // 普通 4xx 不应触发降级重试
        assert!(!mentions_field(
            "{\"error\":{\"message\":\"Assistant tool call function.arguments must be valid JSON.\"}}",
            &stream_options
        ));
        // 推理强度被指名时的关键词匹配
        assert!(mentions_field(
            "Unknown parameter: 'reasoning_effort'.",
            &["reasoning_effort", "reasoning effort"]
        ));
    }

    #[test]
    fn request_log_records_reasoning_effort() {
        let fields = request_log_fields(
            &json!({
                "model": "m",
                "stream": true,
                "instructions": "提示词",
                "input": [{ "type": "message" }],
                "tools": [],
                "reasoning": { "effort": "high", "summary": "auto" }
            }),
            true,
        );
        let value_of = |key: &str| {
            fields
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(value_of("reasoning_effort"), Some("high".to_string()));
        assert_eq!(value_of("model"), Some("m".to_string()));
        assert_eq!(value_of("stream"), Some("true".to_string()));
        assert_eq!(value_of("input_msg_count"), Some("1".to_string()));
        assert_eq!(value_of("tool_count"), Some("0".to_string()));
        assert_eq!(value_of("instructions_chars"), Some("3".to_string()));
        assert!(value_of("input_chars").is_some());

        // 没请求推理时记 `-`
        let fields = request_log_fields(&json!({ "model": "m" }), false);
        assert_eq!(
            fields
                .iter()
                .find(|(name, _)| *name == "reasoning_effort")
                .map(|(_, value)| value.as_str()),
            Some("-")
        );
    }

    #[test]
    fn usage_details_are_mapped_into_responses() {
        let usage = json!({
            "prompt_tokens": 100,
            "completion_tokens": 40,
            "total_tokens": 140,
            "prompt_tokens_details": { "cached_tokens": 80, "cache_write_tokens": 20 },
            "completion_tokens_details": { "reasoning_tokens": 12 }
        });
        let out = usage_to_responses(Some(&usage));
        assert_eq!(out["input_tokens"], 100);
        assert_eq!(out["output_tokens"], 40);
        assert_eq!(out["total_tokens"], 140);
        assert_eq!(out["input_tokens_details"]["cached_tokens"], 80);
        assert_eq!(out["input_tokens_details"]["cache_write_tokens"], 20);
        assert_eq!(out["output_tokens_details"]["reasoning_tokens"], 12);
    }

    #[test]
    fn usage_details_fall_back_to_top_level_fields() {
        let usage = json!({
            "prompt_tokens": 10,
            "completion_tokens": 2,
            "cache_read_input_tokens": 6,
            "cache_creation_input_tokens": 4,
            "reasoning_tokens": 1
        });
        let out = usage_to_responses(Some(&usage));
        assert_eq!(out["input_tokens_details"]["cached_tokens"], 6);
        assert_eq!(out["input_tokens_details"]["cache_write_tokens"], 4);
        assert_eq!(out["output_tokens_details"]["reasoning_tokens"], 1);
    }

    #[test]
    fn usage_without_details_omits_detail_objects() {
        let out = usage_to_responses(Some(&json!({
            "prompt_tokens": 5,
            "completion_tokens": 1,
            "total_tokens": 6
        })));
        assert!(out.get("input_tokens_details").is_none());
        assert!(out.get("output_tokens_details").is_none());
        // null / 非数值都不算命中
        let out = usage_to_responses(Some(&json!({
            "prompt_tokens_details": { "cached_tokens": null },
            "completion_tokens_details": { "reasoning_tokens": "7" },
            "cache_write_tokens": null
        })));
        assert!(out.get("input_tokens_details").is_none());
        assert!(out.get("output_tokens_details").is_none());
    }

    #[test]
    fn optional_fields_map_reasoning_and_passthrough_fields() {
        let req = json!({
            "model": "m",
            "reasoning": { "effort": "high", "summary": "auto" },
            "parallel_tool_calls": false,
            "prompt_cache_key": "cache-key",
            "service_tier": "flex"
        });
        let fields = optional_fields(&req, true);
        let value_of = |key: &str| {
            fields
                .iter()
                .find(|field| field.key == key)
                .map(|field| field.value.clone())
        };
        assert_eq!(value_of("stream_options"), Some(json!({ "include_usage": true })));
        assert_eq!(value_of("reasoning_effort"), Some(json!("high")));
        assert_eq!(value_of("parallel_tool_calls"), Some(json!(false)));
        assert_eq!(value_of("prompt_cache_key"), Some(json!("cache-key")));
        assert_eq!(value_of("service_tier"), Some(json!("flex")));
        // 非流式不带 stream_options；未给可选字段时列表为空
        assert!(optional_fields(&req, false)
            .iter()
            .all(|field| field.key != "stream_options"));
        assert!(optional_fields(&json!({ "model": "m" }), false).is_empty());
    }

    #[test]
    fn tool_choice_converts_only_known_shapes() {
        assert_eq!(
            tool_choice_to_chat(Some(&json!({ "type": "function", "name": "search" }))),
            Some(json!({ "type": "function", "function": { "name": "search" } }))
        );
        assert_eq!(
            tool_choice_to_chat(Some(&json!("required"))),
            Some(json!("required"))
        );
        // 无 chat 等价物的形态与空值一律丢弃
        assert_eq!(tool_choice_to_chat(Some(&json!({ "type": "allowed_tools" }))), None);
        assert_eq!(tool_choice_to_chat(Some(&Value::Null)), None);
        assert_eq!(tool_choice_to_chat(None), None);
    }

    #[test]
    fn message_images_become_chat_content_parts() {
        let req = json!({
            "model": "m",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [
                    { "type": "input_text", "text": "看这张图" },
                    {
                        "type": "input_image",
                        "image_url": "data:image/png;base64,AAA",
                        "detail": "high"
                    }
                ]
            }]
        });
        let (chat, _) = responses_to_chat(&req, false).unwrap();
        let content = &chat["messages"][0]["content"];
        assert_eq!(content[0], json!({ "type": "text", "text": "看这张图" }));
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,AAA");
        assert_eq!(content[1]["image_url"]["detail"], "high");
    }

    #[test]
    fn unsupported_image_detail_or_missing_url_is_skipped() {
        let req = json!({
            "model": "m",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [
                    { "type": "input_image", "image_url": "https://x/y.png", "detail": "original" },
                    { "type": "input_image" }
                ]
            }]
        });
        let (chat, _) = responses_to_chat(&req, false).unwrap();
        let content = chat["messages"][0]["content"].as_array().unwrap().clone();
        assert_eq!(content.len(), 1, "无 url 的图片应被丢弃");
        assert_eq!(content[0]["image_url"]["url"], "https://x/y.png");
        assert!(
            content[0]["image_url"].get("detail").is_none(),
            "未知 detail 不写入"
        );
    }

    #[test]
    fn text_only_messages_keep_string_content() {
        let req = json!({
            "model": "m",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "hi" }]
            }]
        });
        let (chat, _) = responses_to_chat(&req, false).unwrap();
        assert_eq!(chat["messages"][0]["content"], json!("hi"));
    }

    #[test]
    fn reasoning_deltas_become_reasoning_summary_events() {
        let mut st = StreamState::new("resp_r".into(), "m".into());
        let c1 = json!({ "choices": [{ "delta": { "reasoning_content": "先看" }, "finish_reason": null }] });
        let c2 = json!({ "choices": [{ "delta": { "reasoning_content": "日志" }, "finish_reason": null }] });
        let c3 = json!({ "choices": [{ "delta": { "content": "结论" }, "finish_reason": "stop" }] });
        let mut lines = process_chunk(&c1, &mut st);
        lines.extend(process_chunk(&c2, &mut st));
        lines.extend(process_chunk(&c3, &mut st));
        lines.extend(finish_stream(&mut st, &None));

        let names = event_names(&lines);
        assert_eq!(names[0], "response.output_item.added", "推理 item 先开");
        assert!(names.contains(&"response.reasoning_summary_part.added".to_string()));
        assert!(names.contains(&"response.reasoning_summary_text.delta".to_string()));
        assert!(names.contains(&"response.reasoning_summary_text.done".to_string()));
        assert!(names.contains(&"response.reasoning_summary_part.done".to_string()));
        assert!(names.contains(&"response.completed".to_string()));

        let delta = event_payload(&lines, "response.reasoning_summary_text.delta").unwrap();
        assert_eq!(delta["summary_index"], 0);
        assert_eq!(delta["delta"], "先看");
        let done = event_payload(&lines, "response.reasoning_summary_text.done").unwrap();
        assert_eq!(done["text"], "先看日志");

        // output_item.done 顺序：推理 item 在前，且带摘要全文
        let done_items: Vec<Value> = lines
            .iter()
            .filter(|line| line.contains("event: response.output_item.done"))
            .filter_map(|line| {
                serde_json::from_str::<Value>(
                    line.lines().nth(1)?.trim_start_matches("data:").trim(),
                )
                .ok()
            })
            .collect();
        assert_eq!(done_items[0]["item"]["type"], "reasoning");
        assert_eq!(done_items[0]["item"]["summary"][0]["text"], "先看日志");
        assert_eq!(done_items[1]["item"]["type"], "message");
    }

    #[test]
    fn reasoning_object_form_is_understood() {
        let mut st = StreamState::new("resp_ro".into(), "m".into());
        let chunk = json!({
            "choices": [{ "delta": { "reasoning": { "content": "想想" } }, "finish_reason": null }]
        });
        let lines = process_chunk(&chunk, &mut st);
        assert!(lines
            .iter()
            .any(|line| line.contains("response.reasoning_summary_text.delta")
                && line.contains("想想")));
    }

    #[test]
    fn finish_reason_length_yields_incomplete() {
        let mut st = StreamState::new("resp_len".into(), "m".into());
        let c1 = json!({ "choices": [{ "delta": { "content": "半截" }, "finish_reason": null }] });
        let c2 = json!({ "choices": [{ "delta": {}, "finish_reason": "length" }] });
        let mut lines = process_chunk(&c1, &mut st);
        lines.extend(process_chunk(&c2, &mut st));
        lines.extend(finish_stream(&mut st, &None));

        let names = event_names(&lines);
        assert!(names.contains(&"response.incomplete".to_string()));
        assert!(!names.contains(&"response.completed".to_string()));
        let event = event_payload(&lines, "response.incomplete").unwrap();
        assert_eq!(event["response"]["status"], "incomplete");
        assert_eq!(
            event["response"]["incomplete_details"]["reason"],
            "max_output_tokens"
        );
    }

    #[test]
    fn stream_usage_details_are_forwarded() {
        let mut st = StreamState::new("resp_ud".into(), "m".into());
        let c1 = json!({
            "choices": [],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 5,
                "total_tokens": 105,
                "prompt_tokens_details": { "cached_tokens": 60 },
                "completion_tokens_details": { "reasoning_tokens": 3 }
            }
        });
        let c2 = json!({ "choices": [{ "delta": {}, "finish_reason": "stop" }] });
        let mut lines = process_chunk(&c1, &mut st);
        lines.extend(process_chunk(&c2, &mut st));
        lines.extend(finish_stream(&mut st, &None));
        let done = event_payload(&lines, "response.completed").unwrap();
        assert_eq!(
            done["response"]["usage"]["input_tokens_details"]["cached_tokens"],
            60
        );
        assert_eq!(
            done["response"]["usage"]["output_tokens_details"]["reasoning_tokens"],
            3
        );
    }

    #[test]
    fn chat_to_responses_carries_usage_details() {
        let chat = json!({
            "choices": [{ "message": { "role": "assistant", "content": "好" } }],
            "usage": {
                "prompt_tokens": 9,
                "completion_tokens": 3,
                "total_tokens": 12,
                "prompt_tokens_details": { "cached_tokens": 4 }
            }
        });
        let resp = chat_to_responses(&chat, "m").unwrap();
        assert_eq!(resp["usage"]["input_tokens_details"]["cached_tokens"], 4);
        assert_eq!(resp["usage"]["input_tokens"], 9);
    }

    #[test]
    fn failure_takes_precedence_over_incomplete() {
        let mut st = StreamState::new("resp_fi".into(), "m".into());
        st.failure = Some(StreamFailure {
            code: CODE_UPSTREAM_STREAM_ERROR.to_string(),
            message: "boom".to_string(),
            detail: "读取出错".to_string(),
        });
        st.finish_reason = Some("length".to_string());
        let lines = finish_stream(&mut st, &None);
        let names = event_names(&lines);
        assert!(names.contains(&"response.failed".to_string()));
        assert!(!names.contains(&"response.incomplete".to_string()));
        assert!(!names.contains(&"response.completed".to_string()));
    }

    #[test]
    fn finish_reason_alone_does_not_complete_the_stream() {
        let mut st = StreamState::new("resp_ord".into(), "m".into());
        let finish = json!({ "choices": [{ "delta": {}, "finish_reason": "stop" }] });
        assert!(
            process_chunk(&finish, &mut st).is_empty(),
            "finish_reason 分片本身不产出事件"
        );
        assert!(
            !st.closed,
            "不应在 finish_reason 处关流，否则会丢掉随后的 usage 分片"
        );

        // OpenAI `include_usage` 的真实形态：finish_reason 之后再发一个只带 usage 的分片
        let usage = json!({
            "choices": [],
            "usage": {
                "prompt_tokens": 1000,
                "completion_tokens": 20,
                "total_tokens": 1020,
                "prompt_tokens_details": { "cached_tokens": 800 },
                "completion_tokens_details": { "reasoning_tokens": 7 }
            }
        });
        assert!(process_chunk(&usage, &mut st).is_empty());

        let lines = finish_stream(&mut st, &None);
        let done = event_payload(&lines, "response.completed").expect("应正常完成");
        assert_eq!(done["response"]["usage"]["input_tokens"], 1000);
        assert_eq!(
            done["response"]["usage"]["input_tokens_details"]["cached_tokens"],
            800
        );
        assert_eq!(
            done["response"]["usage"]["output_tokens_details"]["reasoning_tokens"],
            7
        );
    }

    #[test]
    fn stream_summary_reports_usage_presence_and_delta_keys() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let mut st = StreamState::new("resp_diag".into(), "m".into());
        st.usage = Some(json!({ "prompt_tokens": 1 }));
        st.delta_keys.insert("content".to_string());
        st.delta_keys.insert("reasoning_content".to_string());
        let _ = finish_stream(&mut st, &log);

        let joined = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| std::fs::read_to_string(e.path()).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("usage=present"), "{joined}");
        assert!(
            joined.contains("delta_keys=content,reasoning_content"),
            "{joined}"
        );
    }

    #[test]
    fn reasoning_details_array_is_understood() {
        let mut st = StreamState::new("resp_rd".into(), "m".into());
        let chunk = json!({
            "choices": [{ "delta": { "reasoning_details": [
                { "type": "reasoning.encrypted", "data": "xxx" },
                { "type": "reasoning.text", "text": "明细推理" }
            ] }, "finish_reason": null }]
        });
        let lines = process_chunk(&chunk, &mut st);
        assert!(lines.iter().any(|line| {
            line.contains("response.reasoning_summary_text.delta") && line.contains("明细推理")
        }));
    }

    #[test]
    fn reasoning_text_and_summary_fields_are_understood() {
        let mut st = StreamState::new("resp_rs".into(), "m".into());
        let text = json!({
            "choices": [{ "delta": { "reasoning": { "text": "来自 text" } }, "finish_reason": null }]
        });
        let summary = json!({
            "choices": [{ "delta": { "reasoning": { "summary": "来自 summary" } }, "finish_reason": null }]
        });
        let mut lines = process_chunk(&text, &mut st);
        lines.extend(process_chunk(&summary, &mut st));
        let joined = lines.join("\n");
        assert!(joined.contains("来自 text"));
        assert!(joined.contains("来自 summary"));
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
        assert!(resp.status.is_success());

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
        assert!(resp.status.is_success());

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
        assert!(resp.status.is_success());
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

    /// 起一个"任何方法/路径都记录一行并回 chat.completion JSON"的上游，
    /// 用于分辨请求走的是翻译分支还是透传分支。
    async fn spawn_mock_record_all(rec: Arc<AsyncMutex<Vec<String>>>) -> String {
        let app = Router::new()
            .fallback(
                move |method: Method,
                      uri: Uri,
                      headers: HeaderMap,
                      _body: axum::body::Bytes| {
                let rec = rec.clone();
                async move {
                    let session = headers
                        .get("x-opencode-session")
                        .and_then(|h| h.to_str().ok())
                        .unwrap_or("-")
                        .to_string();
                    rec.lock().await.push(format!(
                        "{} {} {}",
                        method.as_str(),
                        uri.path(),
                        session
                    ));
                    Json(json!({
                        "id": "chatcmpl-mock",
                        "object": "chat.completion",
                        "created": 0,
                        "model": "m",
                        "choices": [{
                            "index": 0,
                            "message": { "role": "assistant", "content": "mock 回复" },
                            "finish_reason": "stop"
                        }]
                    }))
                }
            },
            )
            // 大请求体用例要能读完整 body（axum 默认上限 2MB）；layer 只作用于此前注册的路由。
            .layer(axum::extract::DefaultBodyLimit::max(RESPONSES_BODY_LIMIT));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    async fn body_text(resp: Response) -> String {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// 记录行的最后一条：`METHOD path session`。
    async fn last_row(rec: &Arc<AsyncMutex<Vec<String>>>) -> Option<String> {
        rec.lock().await.last().cloned()
    }

    /// 取一个当前空闲的回环端口（测试用；绑定后立即释放）。
    fn free_loopback_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    fn responses_probe() -> Body {
        Body::from(
            json!({ "model": "m", "input": "hi", "stream": false }).to_string(),
        )
    }

    fn proxy_state(base_url: String, log: ZenLog) -> ProxyState {
        ProxyState {
            session: "ses_fixed123".into(),
            base_url,
            log,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dispatches_translation_on_base_url_path_plus_responses() {
        let rec: Arc<AsyncMutex<Vec<String>>> = Arc::new(AsyncMutex::new(Vec::new()));
        let upstream = spawn_mock_record_all(rec.clone()).await;

        // 上游路径 /zen/v2 → 只把 /zen/v2/responses 当翻译入口（带不带尾斜杠都一样）。
        for base in [format!("{upstream}/zen/v2"), format!("{upstream}/zen/v2/")] {
            rec.lock().await.clear();
            let state = proxy_state(base.clone(), None);
            let uri: Uri = "/zen/v2/responses".parse().unwrap();
            let resp = handle_any(
                State(state),
                Method::POST,
                OriginalUri(uri),
                HeaderMap::new(),
                responses_probe(),
            )
            .await;
            assert_eq!(resp.status(), StatusCode::OK);
            let body = body_text(resp).await;
            assert!(body.contains("\"object\":\"response\""), "{body}");
            assert_eq!(
                rec.lock().await.clone(),
                vec!["POST /zen/v2/chat/completions ses_fixed123".to_string()]
            );
        }

        // 上游无路径（DeepSeek 官方 base_url 的现实情况）→ /responses 走翻译。
        rec.lock().await.clear();
        let uri: Uri = "/responses".parse().unwrap();
        let resp = handle_any(
            State(proxy_state(upstream.clone(), None)),
            Method::POST,
            OriginalUri(uri),
            HeaderMap::new(),
            responses_probe(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_text(resp).await;
        assert!(body.contains("\"object\":\"response\""), "{body}");
        assert_eq!(
            rec.lock().await.clone(),
            vec!["POST /chat/completions ses_fixed123".to_string()]
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unmatched_responses_path_passes_through_and_warns() {
        let rec: Arc<AsyncMutex<Vec<String>>> = Arc::new(AsyncMutex::new(Vec::new()));
        let upstream = spawn_mock_record_all(rec.clone()).await;
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let state = proxy_state(format!("{upstream}/zen/v1"), log);

        // 本地 provider 少写了 /zen/v1：原样透传到上游，不再静默收尾。
        let uri: Uri = "/responses".parse().unwrap();
        let resp = handle_any(
            State(state),
            Method::POST,
            OriginalUri(uri),
            HeaderMap::new(),
            responses_probe(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        // 透传即上游原文（chat.completion），而非翻译后的 response 对象。
        assert!(body_text(resp).await.contains("chat.completion"));
        assert_eq!(
            rec.lock().await.clone(),
            vec!["POST /responses ses_fixed123".to_string()]
        );

        let logged = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| std::fs::read_to_string(e.path()).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            logged.contains("zen_proxy.path_unmatched")
                && logged.contains("path=/responses")
                && logged.contains("expected=/zen/v1/responses"),
            "{logged}"
        );
        assert!(logged.contains("zen_proxy.passthrough"), "{logged}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn non_post_on_responses_path_passes_through() {
        let rec: Arc<AsyncMutex<Vec<String>>> = Arc::new(AsyncMutex::new(Vec::new()));
        let upstream = spawn_mock_record_all(rec.clone()).await;
        let uri: Uri = "/responses".parse().unwrap();
        let resp = handle_any(
            State(proxy_state(upstream, None)),
            Method::GET,
            OriginalUri(uri),
            HeaderMap::new(),
            Body::empty(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            rec.lock().await.clone(),
            vec!["GET /responses ses_fixed123".to_string()]
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn started_proxy_translates_path_derived_from_upstream() {
        // 端到端覆盖 start() 的路由装配：这正是"翻译分支从来没被走到"的根因所在。
        let rec: Arc<AsyncMutex<Vec<String>>> = Arc::new(AsyncMutex::new(Vec::new()));
        let upstream = spawn_mock_record_all(rec.clone()).await;
        let port = free_loopback_port();
        let mut handle = None;
        let status = apply(
            &mut handle,
            true,
            port,
            format!("{upstream}/zen/v1"),
            None,
        )
        .await;
        assert!(status.running, "{:?}", status.error);

        let resp = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/zen/v1/responses"))
            .header("content-type", "application/json")
            .body(json!({ "model": "m", "input": "hi", "stream": false }).to_string())
            .send()
            .await
            .expect("代理应可访问");
        assert_eq!(resp.status(), StatusCode::OK);
        let text = resp.text().await.unwrap();
        assert!(text.contains("\"object\":\"response\""), "{text}");
        let rows = rec.lock().await.clone();
        assert_eq!(rows.len(), 1);
        assert!(
            rows[0].starts_with("POST /zen/v1/chat/completions ses_"),
            "{rows:?}"
        );

        let status = apply(&mut handle, false, port, upstream, None).await;
        assert!(!status.running);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn oversized_body_still_reaches_translation() {
        // 2MB 曾是 axum 默认上限，长会话请求体已近 1MB；这里用 >2MB 体确认不再 413。
        let rec: Arc<AsyncMutex<Vec<String>>> = Arc::new(AsyncMutex::new(Vec::new()));
        let upstream = spawn_mock_record_all(rec.clone()).await;
        let filler = "x".repeat(3 * 1024 * 1024);
        let body = json!({ "model": "m", "stream": false, "input": filler }).to_string();
        let uri: Uri = "/responses".parse().unwrap();
        let resp = handle_any(
            State(proxy_state(upstream, None)),
            Method::POST,
            OriginalUri(uri),
            HeaderMap::new(),
            Body::from(body),
        )
        .await;
        let status = resp.status();
        let text = body_text(resp).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        assert!(text.contains("\"object\":\"response\""), "{text}");
        assert_eq!(rec.lock().await.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn apply_restarts_only_when_port_or_upstream_changes() {
        let rec_a: Arc<AsyncMutex<Vec<String>>> = Arc::new(AsyncMutex::new(Vec::new()));
        let rec_b: Arc<AsyncMutex<Vec<String>>> = Arc::new(AsyncMutex::new(Vec::new()));
        let upstream_a = spawn_mock_record_all(rec_a.clone()).await;
        let upstream_b = spawn_mock_record_all(rec_b.clone()).await;
        let port = free_loopback_port();
        let mut handle = None;

        let status = apply(&mut handle, true, port, upstream_a.clone(), None).await;
        assert!(status.running, "{:?}", status.error);
        let probe = |port: u16| async move {
            // 每次用新 client：代理重启会关闭旧连接，复用连接池会读到半关闭的连接。
            let resp = reqwest::Client::new()
                .get(format!("http://127.0.0.1:{port}/models"))
                .send()
                .await
                .expect("代理应可访问");
            assert_eq!(resp.status(), StatusCode::OK);
        };

        probe(port).await;
        let first = last_row(&rec_a).await.expect("上游 A 应已收到请求");
        rec_a.lock().await.clear();

        // 端口与上游都没变（含只差尾斜杠的等价写法）→ 复用实例，会话标识不变。
        let status = apply(&mut handle, true, port, upstream_a.clone(), None).await;
        assert!(status.running, "{:?}", status.error);
        let status = apply(
            &mut handle,
            true,
            port,
            format!("{upstream_a}/"),
            None,
        )
        .await;
        assert!(status.running, "{:?}", status.error);
        probe(port).await;
        assert_eq!(
            last_row(&rec_a).await.as_deref(),
            Some(first.as_str()),
            "同端口同上游不应重启（{first}）"
        );

        // 换上游地址 → 立即重启：B 收到请求、A 不再收到。
        let before_a = rec_a.lock().await.len();
        let status = apply(&mut handle, true, port, upstream_b.clone(), None).await;
        assert!(status.running, "{:?}", status.error);
        probe(port).await;
        let second = last_row(&rec_b).await.expect("上游 B 应已收到请求");
        assert_ne!(first.split(' ').nth(2), second.split(' ').nth(2));
        assert_eq!(rec_a.lock().await.len(), before_a);

        // 端口变化 → 也重启。
        let other_port = free_loopback_port();
        let status = apply(&mut handle, true, other_port, upstream_b, None).await;
        assert!(status.running, "{:?}", status.error);
        assert_eq!(status.port, other_port);
        probe(other_port).await;

        // 关闭 → 停止。
        let status = apply(&mut handle, false, other_port, upstream_a, None).await;
        assert!(!status.running);
        assert!(handle.is_none());
    }

    /// 起一个"带指定字段就用 4xx 指名拒绝、否则放行"的 mock Zen
    /// （记录请求次数与末次 body）。
    async fn spawn_mock_zen_rejecting_field(
        count: Arc<AsyncMutex<usize>>,
        last: Arc<AsyncMutex<Option<Value>>>,
        field: &'static str,
    ) -> String {
        let app = Router::new().route(
            "/chat/completions",
            axum::routing::post(move |Json(body): Json<Value>| {
                let count = count.clone();
                let last = last.clone();
                async move {
                    let rejected = body.get(field).is_some();
                    *count.lock().await += 1;
                    *last.lock().await = Some(body);
                    if rejected {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(json!({
                                "error": {
                                    "message": format!("Unrecognized request argument supplied: {field}"),
                                    "type": "invalid_request_error"
                                }
                            })),
                        );
                    }
                    (
                        StatusCode::OK,
                        Json(json!({
                            "id": "chatcmpl-mock",
                            "object": "chat.completion",
                            "model": "m",
                            "choices": [{
                                "index": 0,
                                "message": { "role": "assistant", "content": "ok" },
                                "finish_reason": "stop"
                            }]
                        })),
                    )
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

    /// 起一个恒定 4xx（错误体不指认任何可选字段）的 mock Zen。
    async fn spawn_mock_zen_bad_request(count: Arc<AsyncMutex<usize>>) -> String {
        let app = Router::new().route(
            "/chat/completions",
            axum::routing::post(move |Json(_body): Json<Value>| {
                let count = count.clone();
                async move {
                    *count.lock().await += 1;
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({
                            "error": {
                                "message": "Assistant tool call function.arguments must be valid JSON.",
                                "type": "invalid_request_error"
                            }
                        })),
                    )
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn streaming_request_asks_for_token_usage() {
        let rec: Arc<AsyncMutex<Option<Received>>> = Arc::new(AsyncMutex::new(None));
        let base_url = spawn_mock_zen(rec.clone()).await;

        let req = json!({ "model": "m", "input": "hi", "stream": true });
        let resp = forward(
            &req,
            &HeaderMap::new(),
            true,
            &base_url,
            http_client(),
            "ses_fixed123",
        )
        .await
        .expect("forward 应成功");
        assert!(resp.status.is_success());
        assert!(resp.dropped_fields.is_empty(), "上游未拒绝时不应摘字段");

        let got = rec.lock().await.clone().expect("mock 应已收到请求");
        assert_eq!(got.body["stream_options"]["include_usage"], true);
        assert_eq!(got.body["stream"], true);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn include_usage_is_dropped_and_retried_when_upstream_rejects_it() {
        let count: Arc<AsyncMutex<usize>> = Arc::new(AsyncMutex::new(0));
        let last: Arc<AsyncMutex<Option<Value>>> = Arc::new(AsyncMutex::new(None));
        let base_url =
            spawn_mock_zen_rejecting_field(count.clone(), last.clone(), "stream_options").await;

        let req = json!({ "model": "m", "input": "hi", "stream": true });
        let resp = forward(
            &req,
            &HeaderMap::new(),
            true,
            &base_url,
            http_client(),
            "ses_fixed123",
        )
        .await
        .expect("forward 应成功");
        assert!(resp.status.is_success());
        assert_eq!(
            resp.dropped_fields,
            vec!["stream_options"],
            "应标记被摘掉的字段"
        );
        assert_eq!(*count.lock().await, 2, "应恰好重试一次");

        let got = last.lock().await.clone().expect("mock 应已收到请求");
        assert!(got.get("stream_options").is_none(), "重试不应再带 stream_options");
        assert_eq!(got["stream"], true);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reasoning_effort_is_dropped_and_retried_when_rejected() {
        let count: Arc<AsyncMutex<usize>> = Arc::new(AsyncMutex::new(0));
        let last: Arc<AsyncMutex<Option<Value>>> = Arc::new(AsyncMutex::new(None));
        let base_url =
            spawn_mock_zen_rejecting_field(count.clone(), last.clone(), "reasoning_effort").await;

        let req = json!({
            "model": "m",
            "input": "hi",
            "stream": true,
            "reasoning": { "effort": "xhigh" }
        });
        let resp = forward(
            &req,
            &HeaderMap::new(),
            true,
            &base_url,
            http_client(),
            "ses_fixed123",
        )
        .await
        .expect("forward 应成功");
        assert!(resp.status.is_success());
        assert_eq!(resp.dropped_fields, vec!["reasoning_effort"]);
        assert_eq!(*count.lock().await, 2, "应恰好重试一次");

        let got = last.lock().await.clone().expect("mock 应已收到请求");
        assert!(got.get("reasoning_effort").is_none(), "重试不应再带被拒字段");
        assert_eq!(
            got["stream_options"]["include_usage"], true,
            "未被拒的可选字段应保留"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unrelated_4xx_is_not_retried() {
        let count: Arc<AsyncMutex<usize>> = Arc::new(AsyncMutex::new(0));
        let base_url = spawn_mock_zen_bad_request(count.clone()).await;

        let req = json!({ "model": "m", "input": "hi", "stream": true });
        let resp = forward(
            &req,
            &HeaderMap::new(),
            true,
            &base_url,
            http_client(),
            "ses_fixed123",
        )
        .await
        .expect("forward 应成功");
        assert!(!resp.status.is_success());
        assert!(resp.dropped_fields.is_empty(), "未指名可选字段时不应摘字段");
        assert_eq!(*count.lock().await, 1, "不应重试");
        match resp.payload {
            ForwardPayload::Text(text) => {
                assert!(text.contains("valid JSON"), "错误体应原样透传：{text}")
            }
            ForwardPayload::Live(_) => panic!("4xx 应携带已读出的错误体"),
        }
    }

    /// 起一个"发一片 finish_reason 后就保持连接不关"的 mock 上游。
    async fn spawn_mock_upstream_holding_open() -> String {
        let app = Router::new().route(
            "/stream",
            axum::routing::get(|| async {
                let first = futures_util::stream::once(async {
                    Ok::<_, std::io::Error>(axum::body::Bytes::from_static(
                        b"data: {\"choices\":[{\"delta\":{\"content\":\"half\"},\"finish_reason\":\"stop\"}]}\n\n",
                    ))
                });
                let rest = futures_util::stream::pending::<
                    Result<axum::body::Bytes, std::io::Error>,
                >();
                axum::response::Response::builder()
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .body(Body::from_stream(first.chain(rest)))
                    .unwrap()
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn finish_without_more_chunks_times_out_and_completes() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let base_url = spawn_mock_upstream_holding_open().await;
        let resp = http_client()
            .get(format!("{base_url}/stream"))
            .send()
            .await
            .expect("mock 上游应可连接");

        let req = json!({ "model": "m", "input": "hi", "stream": true });
        let started = std::time::Instant::now();
        let out = proxy_stream_response(req, resp, &log).await;
        let body = tokio::time::timeout(
            std::time::Duration::from_secs(8),
            axum::body::to_bytes(out.into_body(), usize::MAX),
        )
        .await
        .expect("宽限超时后应已收尾，不能挂住回合")
        .unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.contains("event: response.completed"),
            "应正常收尾：{text}"
        );
        assert!(
            started.elapsed() >= USAGE_GRACE,
            "应先等满宽限期再收尾，实际 {:?}",
            started.elapsed()
        );

        let joined = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| std::fs::read_to_string(e.path()).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            joined.contains("zen_proxy.usage_timeout"),
            "应记录宽限超时：{joined}"
        );
        assert!(joined.contains("usage=none"), "{joined}");
    }
}
