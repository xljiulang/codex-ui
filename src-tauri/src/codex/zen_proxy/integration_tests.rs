//! `zen_proxy` 的集成测试（原 `zen_proxy.rs` 内联测试模块拆出）。

use super::test_support::*;
use super::*;

use std::collections::VecDeque;
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
        axum::routing::post(
            move |headers: HeaderMap, Json(body): Json<Value>| async move {
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
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

/// 起一个本地 mock Zen 透传端点，返回带 `/v1` 前缀的 base URL。
async fn spawn_mock_passthrough(rec: Arc<AsyncMutex<Option<PassthroughReceived>>>) -> String {
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
    let state = proxy_state(base_url, None);
    let resp = forward(&req, &headers, false, &state, "req_test")
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
    assert_is_opencode_id(req_id, "msg");
    // 客户端没给 session-id：用代理级稳定会话（形状同样合法）
    assert_eq!(got.opencode_session.as_deref(), Some("ses_fixed123"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forwards_chat_prefers_client_session_header() {
    let rec: Arc<AsyncMutex<Option<Received>>> = Arc::new(AsyncMutex::new(None));
    let base_url = spawn_mock_zen(rec.clone()).await;

    let req = json!({ "model": "m", "input": "hi", "stream": false });
    let mut headers = HeaderMap::new();
    headers.insert("session-id", HeaderValue::from_static("3f1a2b3c"));
    let state = proxy_state(base_url, None);
    let resp = forward(&req, &headers, false, &state, "req_test")
        .await
        .expect("forward 应成功");
    assert!(resp.status.is_success());

    let got = rec.lock().await.clone().expect("mock 应已收到请求");
    // 按 codex 会话 id 派生的 opencode 会话标识：ses_ + 26 位 [0-9A-Za-z]
    let session = got.opencode_session.clone().expect("应有会话 id");
    assert_is_opencode_id(&session, "ses");
    // 反查能把实发值映射回 codex 线程 id（诊断用）
    assert_eq!(
        state.session_map.codex_of(&session).as_deref(),
        Some("3f1a2b3c")
    );
    assert_eq!(got.opencode_client.as_deref(), Some(OPENCODE_CLIENT));
    assert_eq!(got.opencode_project.as_deref(), Some(OPENCODE_PROJECT));
    assert_is_opencode_id(got.opencode_request.as_deref().expect("应有请求 id"), "msg");

    // 同一 codex 会话的后续请求发同一个 x-opencode-session，不同会话则不同
    let resp = forward(&req, &headers, false, &state, "req_test")
        .await
        .expect("forward 应成功");
    assert!(resp.status.is_success());
    let again = rec.lock().await.clone().expect("mock 应已收到请求");
    assert_eq!(again.opencode_session.as_deref(), Some(session.as_str()));
    assert_ne!(
        again.opencode_request.as_deref(),
        got.opencode_request.as_deref(),
        "每条上游请求都应换新的 x-opencode-request"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forwards_without_auth_when_client_sends_none() {
    let rec: Arc<AsyncMutex<Option<Received>>> = Arc::new(AsyncMutex::new(None));
    let base_url = spawn_mock_zen(rec.clone()).await;
    let req = json!({ "model": "m", "input": "hi", "stream": false });
    let headers = HeaderMap::new();
    let state = proxy_state(base_url, None);
    let resp = forward(&req, &headers, false, &state, "req_test")
        .await
        .expect("forward 应成功");
    assert!(resp.status.is_success());
    let got = rec.lock().await.clone().expect("mock 应已收到请求");
    assert_eq!(got.authorization, None);
    assert_eq!(got.user_agent.as_deref(), Some(ZEN_USER_AGENT));
    assert_is_opencode_id(got.opencode_request.as_deref().expect("应有请求 id"), "msg");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn passthrough_forwards_models_and_post_body() {
    let rec: Arc<AsyncMutex<Option<PassthroughReceived>>> = Arc::new(AsyncMutex::new(None));
    let base_url = spawn_mock_passthrough(rec.clone()).await;
    let state = ProxyState {
        session: "ses_fixed123".into(),
        zen_body_patch: is_zen_upstream(&base_url),
        base_url,
        log: None,
        trace: TraceSink::disabled(),
        requires_reasoning_rc: Arc::new(AtomicBool::new(false)),
        session_map: Arc::new(SessionMap::default()),
        modes: Arc::new(ThreadModeRegistry::default()),
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
    let resp = forward_passthrough(&state, Method::GET, &uri, &headers, Body::empty()).await;
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
    // 透传路径同样只需要合法形状：没带 session-id 时用代理级稳定会话
    assert_eq!(got.opencode_session.as_deref(), Some("ses_fixed123"));
    assert_is_opencode_id(got.opencode_request.as_deref().expect("应有请求 id"), "msg");

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
    let rec: Arc<AsyncMutex<Option<PassthroughReceived>>> = Arc::new(AsyncMutex::new(None));
    let base_url = spawn_mock_passthrough(rec.clone()).await;
    let state = ProxyState {
        session: "ses_fixed123".into(),
        zen_body_patch: is_zen_upstream(&base_url),
        base_url,
        log: None,
        trace: TraceSink::disabled(),
        requires_reasoning_rc: Arc::new(AtomicBool::new(false)),
        session_map: Arc::new(SessionMap::default()),
        modes: Arc::new(ThreadModeRegistry::default()),
    };
    let mut headers = HeaderMap::new();
    headers.insert("session-id", HeaderValue::from_static("client-session-42"));

    let uri: Uri = "/v1/models".parse().unwrap();
    let resp = forward_passthrough(&state, Method::GET, &uri, &headers, Body::empty()).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let got = rec.lock().await.clone().expect("mock 应已收到 GET 请求");
    // 入站 session-id 不合法（不是 opencode 形状）也不影响：代理按它派生一个合法会话 id
    let session = got.opencode_session.clone().expect("应有会话 id");
    assert_is_opencode_id(&session, "ses");
    assert_eq!(
        state.session_map.codex_of(&session).as_deref(),
        Some("client-session-42")
    );
}

/// 起一个"任何方法/路径都记录一行并回 chat.completion JSON"的上游，
/// 用于分辨请求走的是翻译分支还是透传分支。
async fn spawn_mock_record_all(rec: Arc<AsyncMutex<Vec<String>>>) -> String {
    let app = Router::new()
        .fallback(
            move |method: Method, uri: Uri, headers: HeaderMap, _body: axum::body::Bytes| {
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

// ---------- 口嗨自动续跑：脚本化上游与端到端用例 ----------

/// 脚本化上游响应：SSE 分片用于流式（正常回复与续跑轮都是流式），
/// `SseDelayed` 模拟「首包很慢」的上游。
#[derive(Clone)]
enum ScriptedReply {
    Sse(Vec<String>),
    /// 等待 `delay` 后一次性下发全部 `lines`：模拟「首包很慢」的上游（续跑轮会用到）。
    SseDelayed {
        lines: Vec<String>,
        delay: Duration,
    },
}

/// 起一个按调用次序返回脚本化响应的 mock 上游，并记录**全部**收到的请求体；
/// 脚本用完后回一条内容为「已完成」的普通 JSON（避免未覆盖的调用把用例带偏）。
async fn spawn_mock_zen_scripted(
    script: Vec<ScriptedReply>,
) -> (String, Arc<AsyncMutex<Vec<Value>>>) {
    let queue = Arc::new(AsyncMutex::new(VecDeque::from(script)));
    let rec: Arc<AsyncMutex<Vec<Value>>> = Arc::new(AsyncMutex::new(Vec::new()));
    let rec_for_route = rec.clone();
    let app = Router::new().route(
        "/chat/completions",
        axum::routing::post(move |Json(body): Json<Value>| {
            let queue = queue.clone();
            let rec = rec_for_route.clone();
            async move {
                rec.lock().await.push(body);
                let reply = queue.lock().await.pop_front();
                match reply {
                    Some(ScriptedReply::Sse(lines)) => {
                        let chunks: Vec<Result<axum::body::Bytes, std::io::Error>> = lines
                            .into_iter()
                            .map(|l| Ok(axum::body::Bytes::from(l)))
                            .collect();
                        axum::response::Response::builder()
                            .header(header::CONTENT_TYPE, "text/event-stream")
                            .body(Body::from_stream(stream::iter(chunks)))
                            .unwrap()
                    }
                    Some(ScriptedReply::SseDelayed { lines, delay }) => {
                        let body = stream::once(async move {
                            tokio::time::sleep(delay).await;
                            let body: Vec<u8> =
                                lines.into_iter().flat_map(|l| l.into_bytes()).collect();
                            Ok::<_, std::io::Error>(axum::body::Bytes::from(body))
                        });
                        axum::response::Response::builder()
                            .header(header::CONTENT_TYPE, "text/event-stream")
                            .body(Body::from_stream(body))
                            .unwrap()
                    }
                    None => axum::response::Response::builder()
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(
                            json!({
                                "choices": [{
                                    "message": { "role": "assistant", "content": "已完成" },
                                    "finish_reason": "stop"
                                }]
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), rec)
}

/// 一段「纯文本回复」的流式分片。
fn sse_text_reply(text: &str) -> Vec<String> {
    vec![
        format!(
            "data: {}\n\n",
            json!({ "choices": [{ "index": 0, "delta": { "content": text }, "finish_reason": null }] })
        ),
        format!(
            "data: {}\n\n",
            json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }] })
        ),
        "data: [DONE]\n\n".to_string(),
    ]
}

/// 一段「工具调用」的流式分片。
fn sse_tool_call_reply(name: &str, arguments: &str) -> Vec<String> {
    vec![
        format!(
            "data: {}\n\n",
            json!({ "choices": [{ "index": 0, "delta": { "tool_calls": [{
                "index": 0, "id": "call_1",
                "function": { "name": name, "arguments": arguments }
            }] }, "finish_reason": null }] })
        ),
        format!(
            "data: {}\n\n",
            json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "tool_calls" }] })
        ),
        "data: [DONE]\n\n".to_string(),
    ]
}

/// 默认模式按约定收尾的终局文本（`<zen_task_completed>` 成对标签）。
fn completion_reply(reason: &str) -> ScriptedReply {
    ScriptedReply::Sse(sse_text_reply(&format!(
        "<zen_task_completed>{reason}</zen_task_completed>"
    )))
}

/// 某个上游请求体里最后一条 assistant 消息的文本（首轮教学注入在 system 里，不影响它）。
fn last_assistant_text(call: &Value) -> String {
    call["messages"]
        .as_array()
        .and_then(|messages| {
            messages
                .iter()
                .rev()
                .find(|m| m["role"].as_str() == Some("assistant"))
        })
        .and_then(|m| m["content"].as_str())
        .unwrap_or_default()
        .to_string()
}

/// 某个上游请求体里的 system 文本（= 入站 `instructions`，首轮教学的契约挂在这里）。
fn system_text(call: &Value) -> String {
    call["messages"]
        .as_array()
        .and_then(|messages| messages.first())
        .filter(|m| m["role"].as_str() == Some("system"))
        .and_then(|m| m["content"].as_str())
        .unwrap_or_default()
        .to_string()
}

/// 某个上游请求体里最后一条消息的文本（续跑轮即注入的续跑提醒）。
fn last_message_text(call: &Value) -> String {
    call["messages"]
        .as_array()
        .and_then(|messages| messages.last())
        .and_then(|m| m["content"].as_str())
        .unwrap_or_default()
        .to_string()
}

/// 入站 Responses 请求体（JSON）：指定末条 user 文本，可选带一个工具声明。
fn nudge_probe_json_for(user_text: &str, with_tools: bool) -> Value {
    let mut req = json!({
        "model": "mimo-v2.5-free",
        "stream": true,
        "input": [{ "type": "message", "role": "user",
                    "content": [{ "type": "input_text", "text": user_text }] }]
    });
    if with_tools {
        req["tools"] = json!([{
            "type": "function",
            "name": "shell",
            "description": "run shell",
            "parameters": { "type": "object", "properties": {} }
        }]);
    }
    req
}

/// 入站 Responses 请求体（JSON）：可选带一个工具声明（无工具时不应催办）。
fn nudge_probe_json(with_tools: bool) -> Value {
    nudge_probe_json_for("把 base_url 的测试补上", with_tools)
}

fn nudge_probe(with_tools: bool) -> Body {
    Body::from(nudge_probe_json(with_tools).to_string())
}

/// 应用 `autoTitleThread` 发起的会话标题生成请求（后台临时线程）。
fn nudge_probe_title_task(with_tools: bool) -> Body {
    let text = format!(
        "{TITLE_TASK_PREFIX}，只输出标题本身，不要任何解释、引号或 Markdown。\n\n用户消息：\n把 zen 代理的看门狗改一下"
    );
    Body::from(nudge_probe_json_for(&text, with_tools).to_string())
}

/// 在探测请求前面插入一条协作模式开发者消息（模拟 codex 下发的模式文本）。
fn nudge_probe_with_mode(with_tools: bool, mode_text: &str) -> Body {
    nudge_probe_with_modes(with_tools, &[mode_text])
}

/// 在探测请求前面按顺序插入多条协作模式开发者消息：codex 会把历次模式块都留在历史里，
/// 切换模式后的请求正是「旧块在前、当前块在后」的形状（线上误判的复现形态）。
fn nudge_probe_with_modes(with_tools: bool, mode_texts: &[&str]) -> Body {
    let mut req = nudge_probe_json(with_tools);
    let mut input: Vec<Value> = mode_texts
        .iter()
        .map(|text| {
            json!({
                "type": "message",
                "role": "developer",
                "content": [{ "type": "input_text", "text": text }]
            })
        })
        .collect();
    input.extend(req["input"].as_array().unwrap().clone());
    req["input"] = Value::Array(input);
    Body::from(req.to_string())
}

/// 带 `session-id` 请求头（codex 上报的线程 id，代理给上游时才补 `ses_` 前缀）。
fn session_headers(thread_id: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("session-id", HeaderValue::from_str(thread_id).unwrap());
    headers
}

/// 预置一条登记的协议模式登记表（模拟 app-server 侧已经登记过该线程的模式）。
fn modes_with(thread_id: &str, mode: &str) -> Arc<ThreadModeRegistry> {
    let registry = ThreadModeRegistry::default();
    assert!(registry.record(thread_id, mode), "登记应成功");
    Arc::new(registry)
}

/// 探测用代理状态（可复用：`nudge` 计数表在同一个 state 内跨请求保留，
/// 测「跨请求不复用计数」这类行为时必须复用同一个 state）。
fn nudge_probe_state(upstream: &str, log: ZenLog) -> ProxyState {
    ProxyState {
        session: "ses_fixed123".into(),
        base_url: upstream.to_string(),
        log,
        trace: TraceSink::disabled(),
        requires_reasoning_rc: Arc::new(AtomicBool::new(false)),
        zen_body_patch: is_zen_upstream(upstream),
        session_map: Arc::new(SessionMap::default()),
        modes: Arc::new(ThreadModeRegistry::default()),
    }
}

/// 用一个已构造好的代理状态走一次流式翻译入口，返回发给 codex 的完整 SSE 文本。
async fn run_nudge_probe_state(state: ProxyState, headers: HeaderMap, body: Body) -> String {
    let uri: Uri = "/responses".parse().unwrap();
    let resp = handle_any(State(state), Method::POST, OriginalUri(uri), headers, body).await;
    assert_eq!(resp.status(), StatusCode::OK);
    body_text(resp).await
}

/// 走一次代理的流式翻译入口（指定请求头与模式登记表），返回发给 codex 的完整 SSE 文本。
async fn run_nudge_probe_full(
    upstream: &str,
    log: ZenLog,
    headers: HeaderMap,
    body: Body,
    modes: Arc<ThreadModeRegistry>,
) -> String {
    let mut state = nudge_probe_state(upstream, log);
    state.modes = modes;
    run_nudge_probe_state(state, headers, body).await
}

/// 走一次代理的流式翻译入口（无请求头、空登记表），返回发给 codex 的完整 SSE 文本。
async fn run_nudge_probe(upstream: &str, log: ZenLog, body: Body) -> String {
    run_nudge_probe_full(
        upstream,
        log,
        HeaderMap::new(),
        body,
        Arc::new(ThreadModeRegistry::default()),
    )
    .await
}

/// 读取会话日志目录下全部内容（断言 nudge_* 事件用）。
fn read_session_log(dir: &tempfile::TempDir) -> String {
    std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| std::fs::read_to_string(e.path()).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_injects_continuation_and_relays_tool_call() {
    let dir = tempfile::TempDir::new().unwrap();
    let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
    let (upstream, rec) = spawn_mock_zen_scripted(vec![
        ScriptedReply::Sse(sse_text_reply(
            "Now update the test - specifically the base_url path test.",
        )),
        ScriptedReply::Sse(sse_tool_call_reply("shell", "{\"cmd\":\"ls\"}")),
    ])
    .await;

    let body = run_nudge_probe(&upstream, log, nudge_probe(true)).await;

    // 对 codex 而言只是「一条带文本的响应，随后调用了工具」，只收尾一次
    assert!(body.contains("response.output_text.delta"), "{body}");
    assert!(body.contains("Now update the test"), "{body}");
    assert!(body.contains("function_call"), "{body}");
    assert_eq!(body.matches("event: response.completed").count(), 1);

    let calls = rec.lock().await.clone();
    assert_eq!(calls.len(), 2, "应为：首轮 + 续跑（不再有独立判定请求）");
    // 首轮与续跑轮都带首轮教学注入的契约（instructions → system 消息）
    assert!(
        system_text(&calls[0]).contains(DEFAULT_MODE_CONTRACT_TEXT),
        "首轮应带默认模式契约：{}",
        calls[0]
    );
    // 续跑调用：历史尾部是「助手原样文本 + 注入提醒」
    let messages = calls[1]["messages"].as_array().unwrap();
    assert_eq!(
        messages.len(),
        4,
        "契约的 system 消息 + user + assistant + 提醒：{messages:?}"
    );
    assert_eq!(messages.last().unwrap()["role"], "user");
    assert!(last_assistant_text(&calls[1]).contains("Now update the test"));
    let injected = messages.last().unwrap()["content"].as_str().unwrap();
    assert!(injected.contains("自动续跑"), "{injected}");
    assert!(
        injected.contains(&format!("{TASK_COMPLETED_MARKER}>")),
        "默认模式提醒必须教出成对标签：{injected}"
    );
    assert!(
        injected.contains("必须实际调用工具"),
        "默认模式提醒必须带执行口径：{injected}"
    );
    // 会话日志：只留注入一条（判定事件已随看门狗删除）
    let joined = read_session_log(&dir);
    assert!(
        joined.contains("event=zen_proxy.nudge_injected"),
        "{joined}"
    );
    assert!(!joined.contains("event=zen_proxy.nudge_judged"), "{joined}");
}

/// 终局自带 `<zen_task_completed>`：模型自己宣告任务结束，一次调用就收尾（不注入、不续跑）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_skipped_when_completion_tag_present() {
    let dir = tempfile::TempDir::new().unwrap();
    let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
    let (upstream, rec) = spawn_mock_zen_scripted(vec![
        ScriptedReply::Sse(sse_text_reply(
            "这段代码无需改动，原因是 ……\n<zen_task_completed>无需改动</zen_task_completed>",
        )),
        // 第 2 轮不该发生：真发生时脚本会回空话，下面的调用数断言会失败
        ScriptedReply::Sse(sse_text_reply("这一轮不应该被调用")),
    ])
    .await;

    let body = run_nudge_probe(&upstream, log, nudge_probe(true)).await;

    assert_eq!(body.matches("event: response.completed").count(), 1);
    assert!(!body.contains("function_call"), "{body}");
    assert_eq!(rec.lock().await.len(), 1, "带标签时不应续跑");
    // 正文保留、标签（含载荷）不进 codex：delta 与三个 done 事件里都不该有它
    assert!(body.contains("这段代码无需改动"), "{body}");
    assert!(
        !body.contains(TASK_COMPLETED_MARKER),
        "标签不得下发到 codex：{body}"
    );
    assert!(
        !body.contains("无需改动</zen_task_completed>"),
        "标签载荷也不得下发：{body}"
    );
    // 下发的 done 文本 = 剥掉标签后的正文（delta 与 done 一致）
    assert!(
        body.contains(r#""text":"这段代码无需改动，原因是 ……\n"#),
        "{body}"
    );
    let joined = read_session_log(&dir);
    assert!(
        joined.contains("event=zen_proxy.nudge_skipped")
            && joined.contains("原因=task_completed")
            && joined.contains("模式=default"),
        "{joined}"
    );
    assert!(
        joined.contains("标签内容=无需改动"),
        "被删载荷要进日志：{joined}"
    );
    assert!(
        !joined.contains("event=zen_proxy.nudge_injected"),
        "带标签时不应注入：{joined}"
    );
}

/// 续跑轮与首轮走同一个 `forward`：门禁补丁在每一轮上游请求体里都要在。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn zen_body_patch_applies_to_nudge_continuation() {
    let (upstream, rec) = spawn_mock_zen_scripted(vec![
        ScriptedReply::Sse(sse_text_reply(
            "Now update the test - specifically the base_url path test.",
        )),
        completion_reply("已完成"),
    ])
    .await;

    let mut state = nudge_probe_state(&upstream, None);
    state.zen_body_patch = true;
    let body = run_nudge_probe_state(state, HeaderMap::new(), nudge_probe(true)).await;
    assert_eq!(body.matches("event: response.completed").count(), 1);

    let calls = rec.lock().await.clone();
    assert_eq!(calls.len(), 2, "应为：首轮 + 续跑");
    for call in &calls {
        assert_eq!(
            call["max_tokens"], ZEN_MAX_TOKENS,
            "每一轮都要补 max_tokens：{call}"
        );
        let names: Vec<&str> = call["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec!["shell", "bash", "edit", "glob", "grep", "read", "write"],
            "每一轮都要补假工具：{call}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_stops_after_max_injections() {
    let dir = tempfile::TempDir::new().unwrap();
    let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
    let (upstream, rec) = spawn_mock_zen_scripted(vec![
        ScriptedReply::Sse(sse_text_reply("第一句空话")),
        ScriptedReply::Sse(sse_text_reply("第二句空话")),
        ScriptedReply::Sse(sse_text_reply("第三句空话")),
        ScriptedReply::Sse(sse_text_reply("第四句空话")),
        ScriptedReply::Sse(sse_text_reply("第五句空话")),
    ])
    .await;

    let body = run_nudge_probe(&upstream, log, nudge_probe(true)).await;

    assert_eq!(body.matches("event: response.completed").count(), 1);
    assert_eq!(
        rec.lock().await.len(),
        5,
        "首轮 + 续跑 × 4；第五次口嗨不再注入（单请求注入上限）"
    );
    let joined = read_session_log(&dir);
    assert!(
        joined.contains("event=zen_proxy.nudge_limited")
            && joined.contains("原因=max_injections")
            && joined.contains("轮次=5")
            && joined.contains("说明=本请求已催办 4 次仍未收尾，停止催办"),
        "{joined}"
    );
    assert_eq!(
        joined.matches("event=zen_proxy.nudge_injected").count(),
        4,
        "恰好注入四次：{joined}"
    );
    assert!(
        joined.contains("本请求催办次数=4/4") && joined.contains("说明="),
        "注入日志应带中文次数与说明：{joined}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_skipped_when_request_has_no_tools() {
    let (upstream, rec) =
        spawn_mock_zen_scripted(vec![ScriptedReply::Sse(sse_text_reply("纯聊天回答"))]).await;

    let body = run_nudge_probe(&upstream, None, nudge_probe(false)).await;

    assert_eq!(body.matches("event: response.completed").count(), 1);
    assert_eq!(rec.lock().await.len(), 1, "无工具可用时不催办");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_second_pass_survives_slow_upstream() {
    // 续跑轮的首包晚于 USAGE_GRACE：它不应继承上一轮的 finish_reason 而套用
    // 「尾包宽限」把自己整轮丢掉（那样注入就白做了、工具调用也拿不到）。
    let tool_chunks = sse_tool_call_reply("shell", "{\"cmd\":\"ls\"}");
    let (upstream, rec) = spawn_mock_zen_scripted(vec![
        ScriptedReply::Sse(sse_text_reply("我先说明一下接下来要做的事。")),
        ScriptedReply::SseDelayed {
            lines: tool_chunks,
            delay: USAGE_GRACE + Duration::from_millis(500),
        },
    ])
    .await;

    let body = run_nudge_probe(&upstream, None, nudge_probe(true)).await;

    assert!(body.contains("function_call"), "{body}");
    assert_eq!(body.matches("event: response.completed").count(), 1);
    assert_eq!(rec.lock().await.len(), 2);
}

/// 计划模式下的协作模式文本段（codex 0.154 实测文案，见 [`REAL_PLAN_MODE_BLOCK`]；
/// 前缀补一段 skills 文本以贴近真实请求：模式块拼在同一个 developer 条目末尾）。
fn plan_mode_text() -> String {
    format!("<skills_instructions>## Skills …</skills_instructions>{REAL_PLAN_MODE_BLOCK}")
}

/// 计划模式下终局没有计划标签：按「计划未交付」注入计划专用提醒续跑，
/// 全程纯代码判据（计划模式不看默认模式的收尾标签，也不发任何独立判定请求）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_plan_mode_asks_for_plan_when_no_marker() {
    let dir = tempfile::TempDir::new().unwrap();
    let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
    let (upstream, rec) = spawn_mock_zen_scripted(vec![
        // 故意不带 <proposed_plan> 包裹：弱模型常直接把计划写成普通 Markdown，
        // 这条用例要保证它被代码判据判为「未交付」并催出计划。
        ScriptedReply::Sse(sse_text_reply(
            "## 计划\n1. 后端加 git_version 命令\n2. 关于页面加一行显示\n3. 跑测试",
        )),
        // 续跑轮：这次给了带标签的计划（对 codex 只是一条普通文本响应）
        ScriptedReply::Sse(sse_text_reply(
            "<proposed_plan>\n1. 后端加 git_version 命令\n2. 关于页面加一行显示\n</proposed_plan>",
        )),
    ])
    .await;

    let body = run_nudge_probe(
        &upstream,
        log,
        nudge_probe_with_mode(true, &plan_mode_text()),
    )
    .await;

    assert_eq!(body.matches("event: response.completed").count(), 1);
    let calls = rec.lock().await.clone();
    assert_eq!(
        calls.len(),
        2,
        "计划模式下应「首轮 + 续跑」，无独立判定请求：{calls:?}"
    );
    // 续跑调用：历史尾部是「助手原样文本 + 计划专用提醒」
    let messages = calls[1]["messages"].as_array().unwrap();
    let last = messages.last().unwrap();
    assert_eq!(last["role"], "user");
    let injected = last["content"].as_str().unwrap();
    assert!(injected.contains("没有交付计划"), "{injected}");
    assert!(injected.contains("<proposed_plan>"), "{injected}");
    assert!(
        !injected.contains("必须实际调用工具"),
        "计划模式不得注入执行口径：{injected}"
    );
    let joined = read_session_log(&dir);
    assert!(
        joined.contains("event=zen_proxy.nudge_injected") && joined.contains("模式=plan"),
        "{joined}"
    );
    assert!(
        !joined.contains("event=zen_proxy.nudge_judged"),
        "已随看门狗删除的判定事件不应出现：{joined}"
    );
}

/// 用户中途放弃计划（实测例子：「行，那不处理了」）：首轮模型只写普通正文 → 催办一轮，
/// 续跑轮按约定用 `<zen_plan_cancelled>` 标签收尾 → 代码判据直接收尾，不再注入第 3 轮、
/// 不会把一份已被放弃的完整方案重新逼出来。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_plan_mode_stops_when_plan_cancelled() {
    let dir = tempfile::TempDir::new().unwrap();
    let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
    let (upstream, rec) = spawn_mock_zen_scripted(vec![
        ScriptedReply::Sse(sse_text_reply("好，那就保持现状，这一项先不改了。")),
        ScriptedReply::Sse(sse_text_reply(
            "明白，那就不处理了。\n<zen_plan_cancelled>用户说不用改了</zen_plan_cancelled>",
        )),
        // 第 3 轮不该发生：真发生时脚本会回空话，下面的调用数断言会失败
        ScriptedReply::Sse(sse_text_reply("这一轮不应该被调用")),
    ])
    .await;

    let body = run_nudge_probe(
        &upstream,
        log,
        nudge_probe_with_mode(true, &plan_mode_text()),
    )
    .await;

    assert_eq!(body.matches("event: response.completed").count(), 1);
    let calls = rec.lock().await.clone();
    assert_eq!(
        calls.len(),
        2,
        "「已取消计划」应当一轮催办后就收尾，不再注入：{calls:?}"
    );
    // 催办轮注入的提醒里带上了约定标签
    let messages = calls[1]["messages"].as_array().unwrap();
    let injected = messages.last().unwrap()["content"].as_str().unwrap();
    assert!(injected.contains("<zen_plan_cancelled>"), "{injected}");
    let joined = read_session_log(&dir);
    assert!(
        joined.contains("event=zen_proxy.nudge_skipped")
            && joined.contains("原因=plan_cancelled")
            && joined.contains("模式=plan"),
        "{joined}"
    );
    assert!(
        !joined.contains("event=zen_proxy.nudge_limited"),
        "已取消分支不该走到催办次数上限：{joined}"
    );
    assert!(
        !joined.contains("event=zen_proxy.nudge_judged"),
        "已随看门狗删除的判定事件不应出现：{joined}"
    );
}

/// 模型首轮就已按约定给出取消标签（早前回合被催过）：一次调用就收尾，连催办都不发。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_plan_mode_skips_injection_when_already_cancelled() {
    let dir = tempfile::TempDir::new().unwrap();
    let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
    let (upstream, rec) = spawn_mock_zen_scripted(vec![
        ScriptedReply::Sse(sse_text_reply(
            "那就不做改动了。\n<zen_plan_cancelled>用户说不用改了</zen_plan_cancelled>",
        )),
        ScriptedReply::Sse(sse_text_reply("这一轮不应该被调用")),
    ])
    .await;

    let body = run_nudge_probe(
        &upstream,
        log,
        nudge_probe_with_mode(true, &plan_mode_text()),
    )
    .await;

    assert_eq!(body.matches("event: response.completed").count(), 1);
    assert_eq!(rec.lock().await.len(), 1, "首轮即带取消标签时不应再打上游");
    // 标签不下发：聊天里只剩模型那句结论
    assert!(
        body.contains("那就不做改动了。") && !body.contains("<zen_plan_cancelled"),
        "计划模式的出口标签必须剥离：{body}"
    );
    let joined = read_session_log(&dir);
    assert!(
        joined.contains("event=zen_proxy.nudge_skipped")
            && joined.contains("原因=plan_cancelled")
            && joined.contains("轮次=1")
            && joined.contains("标签内容=用户说不用改了"),
        "{joined}"
    );
    assert!(
        !joined.contains("event=zen_proxy.nudge_injected"),
        "已取消的计划不应再注入催办：{joined}"
    );
}

/// 问题本身无法或无需产出实现计划（如「1+1=？」这类事实问题）：首轮模型只写普通正文 →
/// 催办一轮，续跑轮按约定用 `<zen_plan_unachievable>` 标签收尾 → 代码判据直接收尾，
/// 不会把一份本就不产出计划的方案重新逼出来。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_plan_mode_stops_when_plan_unachievable() {
    let dir = tempfile::TempDir::new().unwrap();
    let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
    let (upstream, rec) = spawn_mock_zen_scripted(vec![
        ScriptedReply::Sse(sse_text_reply("1 + 1 = 2。")),
        ScriptedReply::Sse(sse_text_reply(
            "1 + 1 = 2。\n<zen_plan_unachievable>事实问题，不产出实现计划</zen_plan_unachievable>",
        )),
        // 第 3 轮不该发生：真发生时脚本会回空话，下面的调用数断言会失败
        ScriptedReply::Sse(sse_text_reply("这一轮不应该被调用")),
    ])
    .await;

    let body = run_nudge_probe(
        &upstream,
        log,
        nudge_probe_with_mode(true, &plan_mode_text()),
    )
    .await;

    assert_eq!(body.matches("event: response.completed").count(), 1);
    let calls = rec.lock().await.clone();
    assert_eq!(
        calls.len(),
        2,
        "「无法/无需计划」应当一轮催办后就收尾，不再注入：{calls:?}"
    );
    // 催办轮注入的提醒里带上了约定标签
    let messages = calls[1]["messages"].as_array().unwrap();
    let injected = messages.last().unwrap()["content"].as_str().unwrap();
    assert!(injected.contains("<zen_plan_unachievable>"), "{injected}");
    let joined = read_session_log(&dir);
    assert!(
        joined.contains("event=zen_proxy.nudge_skipped")
            && joined.contains("原因=plan_unachievable")
            && joined.contains("模式=plan"),
        "{joined}"
    );
    assert!(
        !joined.contains("event=zen_proxy.nudge_limited"),
        "无法/无需计划分支不该走到催办次数上限：{joined}"
    );
    assert!(
        !joined.contains("event=zen_proxy.nudge_judged"),
        "已随看门狗删除的判定事件不应出现：{joined}"
    );
}

/// 模型首轮就已按约定给出「无法/无需计划」标签（早前回合被催过）：一次调用就收尾，
/// 连催办都不发。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_plan_mode_skips_injection_when_already_unachievable() {
    let dir = tempfile::TempDir::new().unwrap();
    let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
    let (upstream, rec) = spawn_mock_zen_scripted(vec![
        ScriptedReply::Sse(sse_text_reply(
            "这就是个纯查询。\n<zen_plan_unachievable>无实现计划可给</zen_plan_unachievable>",
        )),
        ScriptedReply::Sse(sse_text_reply("这一轮不应该被调用")),
    ])
    .await;

    let body = run_nudge_probe(
        &upstream,
        log,
        nudge_probe_with_mode(true, &plan_mode_text()),
    )
    .await;

    assert_eq!(body.matches("event: response.completed").count(), 1);
    assert_eq!(
        rec.lock().await.len(),
        1,
        "首轮即带无法/无需计划标签时不应再打上游"
    );
    let joined = read_session_log(&dir);
    assert!(
        joined.contains("event=zen_proxy.nudge_skipped")
            && joined.contains("原因=plan_unachievable")
            && joined.contains("轮次=1"),
        "{joined}"
    );
    assert!(
        !joined.contains("event=zen_proxy.nudge_injected"),
        "无法/无需计划不应再注入催办：{joined}"
    );
}

/// 关键回归：历史里残留旧的计划模式块（codex 把历次模式块都留在历史里），但协议登记表
/// 说这个线程现在是默认模式——必须走默认模式的续跑（注入带 `<zen_task_completed>` 契约的执行
/// 口径），不能按计划模式注入「请给出计划」（这正是线上 `collaboration_mode_kind=default`
/// 却按 plan 分流的那次误判）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_mode_registry_beats_stale_plan_block_in_history() {
    let dir = tempfile::TempDir::new().unwrap();
    let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
    let (upstream, rec) = spawn_mock_zen_scripted(vec![
        ScriptedReply::Sse(sse_text_reply(
            "Now update the test - specifically the base_url path test.",
        )),
        ScriptedReply::Sse(sse_tool_call_reply("shell", "{\"cmd\":\"ls\"}")),
    ])
    .await;

    let body = run_nudge_probe_full(
        &upstream,
        log,
        session_headers("01a0aaf8-abc"),
        nudge_probe_with_modes(true, &[&plan_mode_text(), REAL_DEFAULT_MODE_BLOCK]),
        modes_with("01a0aaf8-abc", "default"),
    )
    .await;

    assert_eq!(body.matches("event: response.completed").count(), 1);
    let calls = rec.lock().await.clone();
    assert_eq!(
        calls.len(),
        2,
        "登记表说默认模式：应「首轮 + 续跑」（无独立判定请求）：{calls:?}"
    );
    let injected = calls[1]["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap();
    assert!(
        injected.contains("必须实际调用工具"),
        "默认模式应注入执行口径：{injected}"
    );
    assert!(
        injected.contains(&format!("{TASK_COMPLETED_MARKER}>")),
        "默认模式应注入标签契约：{injected}"
    );
    assert!(
        !injected.contains("没有交付计划"),
        "不得按计划模式注入计划提醒：{injected}"
    );
    let joined = read_session_log(&dir);
    assert!(
        joined.contains("event=zen_proxy.nudge_injected"),
        "默认模式应注入续跑提醒：{joined}"
    );
    assert!(
        joined.contains("模式=default") && joined.contains("模式来源=registry"),
        "模式与来源应记进日志：{joined}"
    );
    assert!(
        !joined.contains("模式=plan"),
        "历史里的旧计划块不应把模式拉回 plan：{joined}"
    );
}

/// 反向：登记表说计划模式、而历史里最后一块是默认模式块时，以登记表为准走计划分支
/// （注入计划专用提醒，不看默认模式标签）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_mode_registry_plan_wins_over_default_block_in_history() {
    let dir = tempfile::TempDir::new().unwrap();
    let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
    let (upstream, rec) = spawn_mock_zen_scripted(vec![
        ScriptedReply::Sse(sse_text_reply(
            "## 计划\n1. 后端加 git_version 命令\n2. 关于页面加一行显示",
        )),
        ScriptedReply::Sse(sse_text_reply(
            "<proposed_plan>\n1. 后端加 git_version 命令\n</proposed_plan>",
        )),
    ])
    .await;

    let body = run_nudge_probe_full(
        &upstream,
        log,
        session_headers("ses_thread-plan"),
        nudge_probe_with_modes(true, &[REAL_DEFAULT_MODE_BLOCK]),
        modes_with("thread-plan", "plan"),
    )
    .await;

    assert_eq!(body.matches("event: response.completed").count(), 1);
    let calls = rec.lock().await.clone();
    assert_eq!(calls.len(), 2, "计划模式只应「首轮 + 续跑」：{calls:?}");
    let injected = calls[1]["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap();
    assert!(injected.contains("没有交付计划"), "{injected}");
    assert!(injected.contains("<proposed_plan>"), "{injected}");
    let joined = read_session_log(&dir);
    assert!(
        joined.contains("模式=plan") && joined.contains("模式来源=registry"),
        "{joined}"
    );
    assert!(
        !joined.contains("event=zen_proxy.nudge_judged"),
        "已随看门狗删除的判定事件不应出现：{joined}"
    );
}

/// 没有 `session-id` 请求头（其它客户端/连不上登记表的线程）时退回关键词兜底判据，
/// 且以**最后一个**模式块为准：默认块在最后 → 默认分支；计划块在最后 → 计划分支。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_mode_falls_back_to_last_mode_block_without_registry() {
    // ① 旧计划块在前、默认块在后 → 默认模式（续跑 + 执行口径）
    let dir = tempfile::TempDir::new().unwrap();
    let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
    let (upstream, rec) = spawn_mock_zen_scripted(vec![
        ScriptedReply::Sse(sse_text_reply("我先说明一下接下来要做的事。")),
        ScriptedReply::Sse(sse_tool_call_reply("shell", "{\"cmd\":\"ls\"}")),
    ])
    .await;
    let body = run_nudge_probe(
        &upstream,
        log,
        nudge_probe_with_modes(true, &[&plan_mode_text(), REAL_DEFAULT_MODE_BLOCK]),
    )
    .await;
    assert_eq!(body.matches("event: response.completed").count(), 1);
    assert_eq!(rec.lock().await.len(), 2, "默认分支应续跑一轮");
    let joined = read_session_log(&dir);
    assert!(
        joined.contains("模式=default") && joined.contains("模式来源=heuristic"),
        "无登记表时应记关键词兜底来源：{joined}"
    );
    assert!(
        joined.contains("event=zen_proxy.nudge_injected"),
        "默认分支应注入续跑提醒：{joined}"
    );

    // ② 默认块在前、计划块在后 → 计划模式（注入计划提醒）
    let dir = tempfile::TempDir::new().unwrap();
    let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
    let (upstream, rec) = spawn_mock_zen_scripted(vec![
        ScriptedReply::Sse(sse_text_reply("## 计划\n1. 做 A")),
        ScriptedReply::Sse(sse_text_reply("<proposed_plan>\n1. 做 A\n</proposed_plan>")),
    ])
    .await;
    let body = run_nudge_probe(
        &upstream,
        log,
        nudge_probe_with_modes(true, &[REAL_DEFAULT_MODE_BLOCK, &plan_mode_text()]),
    )
    .await;
    assert_eq!(body.matches("event: response.completed").count(), 1);
    let calls = rec.lock().await.clone();
    assert_eq!(calls.len(), 2, "计划分支只续跑一轮");
    let injected = calls[1]["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap();
    assert!(
        injected.contains("没有交付计划") && injected.contains("<proposed_plan>"),
        "计划分支必须注入计划专用提醒：{injected}"
    );
    let joined = read_session_log(&dir);
    assert!(
        joined.contains("模式=plan") && joined.contains("模式来源=heuristic"),
        "{joined}"
    );
}

/// 计划模式下终局带了计划标签：代码判据直接判「计划已交付」，一次上游调用就收尾。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_skipped_in_plan_mode_when_plan_delivered() {
    let dir = tempfile::TempDir::new().unwrap();
    let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
    let (upstream, rec) = spawn_mock_zen_scripted(vec![ScriptedReply::Sse(sse_text_reply(
        "<proposed_plan>\n1. 做 A\n2. 做 B\n</proposed_plan>",
    ))])
    .await;

    let body = run_nudge_probe(
        &upstream,
        log,
        nudge_probe_with_mode(true, &plan_mode_text()),
    )
    .await;

    assert_eq!(body.matches("event: response.completed").count(), 1);
    assert_eq!(rec.lock().await.len(), 1, "已交付的计划不应再打上游");
    let joined = read_session_log(&dir);
    assert!(
        joined.contains("event=zen_proxy.nudge_skipped")
            && joined.contains("原因=plan_output")
            && joined.contains("模式=plan"),
        "{joined}"
    );
}

/// 计划模式也受「同会话连续注入 2 次」上限约束：第 3 次没给出计划时不再注入。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_plan_mode_stops_after_max_injections() {
    let dir = tempfile::TempDir::new().unwrap();
    let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
    let (upstream, rec) = spawn_mock_zen_scripted(vec![
        ScriptedReply::Sse(sse_text_reply("第一句空话")),
        ScriptedReply::Sse(sse_text_reply("第二句空话")),
        ScriptedReply::Sse(sse_text_reply("第三句空话")),
        ScriptedReply::Sse(sse_text_reply("第四句空话")),
        ScriptedReply::Sse(sse_text_reply("第五句空话")),
    ])
    .await;

    let body = run_nudge_probe(
        &upstream,
        log,
        nudge_probe_with_mode(true, &plan_mode_text()),
    )
    .await;

    assert_eq!(body.matches("event: response.completed").count(), 1);
    assert_eq!(
        rec.lock().await.len(),
        5,
        "首轮 + 续跑 × 4；第五次没给计划也不再注入"
    );
    let joined = read_session_log(&dir);
    assert!(
        joined.contains("event=zen_proxy.nudge_limited")
            && joined.contains("原因=max_injections")
            && joined.contains("模式=plan")
            && joined.contains("轮次=5"),
        "{joined}"
    );
}

/// 默认模式的协作块正文里带「(e.g. Plan mode)」：不能被误判成计划模式而注入计划提醒
/// （否则默认模式的正常编码回合会被注入「请给出计划」）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_default_mode_block_injects_execution_nudge() {
    let dir = tempfile::TempDir::new().unwrap();
    let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
    let (upstream, rec) = spawn_mock_zen_scripted(vec![
        ScriptedReply::Sse(sse_text_reply("先给个说明，接下来我再动手。")),
        // 续跑轮真的调用了工具：回合就此收尾，不再产生第三轮
        ScriptedReply::Sse(sse_tool_call_reply("shell", "{\"cmd\":\"ls\"}")),
    ])
    .await;

    let body = run_nudge_probe(
        &upstream,
        log,
        nudge_probe_with_mode(true, REAL_DEFAULT_MODE_BLOCK),
    )
    .await;

    assert_eq!(body.matches("event: response.completed").count(), 1);
    assert!(body.contains("function_call"), "{body}");
    let calls = rec.lock().await.clone();
    assert_eq!(calls.len(), 2, "默认模式应「首轮 + 续跑」：{calls:?}");
    let injected = calls[1]["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap();
    assert!(
        injected.contains("必须实际调用工具"),
        "默认模式注入执行口径：{injected}"
    );
    assert!(
        injected.contains(&format!("{TASK_COMPLETED_MARKER}>")),
        "默认模式注入标签契约：{injected}"
    );
    let joined = read_session_log(&dir);
    assert!(
        joined.contains("event=zen_proxy.nudge_injected") && joined.contains("模式=default"),
        "{joined}"
    );
}

/// 默认模式下终局文本本身是计划产物（`<proposed_plan>`）：那不是默认模式的收尾标签，
/// 因此照常注入续跑提醒；模型在续跑轮按约定用 `<zen_task_completed>` 收尾即结束
/// （「已给方案、等你确认」也要写进标签，不再由 AI 判定替它放行）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_default_mode_plan_output_is_not_a_completion_tag() {
    let dir = tempfile::TempDir::new().unwrap();
    let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
    let (upstream, rec) = spawn_mock_zen_scripted(vec![
        ScriptedReply::Sse(sse_text_reply(
            "先给方案：\n<proposed_plan>\n1. 做 A\n2. 做 B\n</proposed_plan>",
        )),
        completion_reply("已给方案，等你确认后再动手"),
    ])
    .await;

    let body = run_nudge_probe(&upstream, log, nudge_probe(true)).await;

    assert_eq!(body.matches("event: response.completed").count(), 1);
    let calls = rec.lock().await.clone();
    assert_eq!(
        calls.len(),
        2,
        "计划产物不是默认模式收尾标签，应续跑一轮：{calls:?}"
    );
    // 续跑请求：历史尾部是「助手原样文本（含计划包裹）+ 注入提醒」
    assert!(
        last_assistant_text(&calls[1]).contains("1. 做 A"),
        "续跑请求应原样带上上一轮助手文本：{}",
        calls[1]
    );
    assert!(
        last_message_text(&calls[1]).contains(&format!("{TASK_COMPLETED_MARKER}>")),
        "续跑请求应教出标签契约：{}",
        calls[1]
    );
    // 第二轮带标签收尾：下发给 codex 的文本里只剩方案，标签被结构剥离
    assert!(body.contains("1. 做 A"), "{body}");
    assert!(
        !body.contains(TASK_COMPLETED_MARKER),
        "标签不得下发到 codex：{body}"
    );
    let joined = read_session_log(&dir);
    assert!(
        joined.contains("原因=task_completed") && joined.contains("模式=default"),
        "{joined}"
    );
    assert!(
        joined.contains("标签内容=已给方案"),
        "剥离后的载荷要进日志：{joined}"
    );
}

/// 催办轮按「标签外不得有任何其他字符」收尾：续跑轮只回一行标签，
/// 下发给 codex 的可见文本不新增任何字符（上一轮正文已经展示过，再补就是噪音）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_tag_only_continuation_keeps_stream_valid() {
    const FIRST_PASS: &str = "先给个说明，接下来我再动手。";
    let dir = tempfile::TempDir::new().unwrap();
    let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
    let (upstream, rec) = spawn_mock_zen_scripted(vec![
        // 首轮典型口嗨：纯文本 + 零工具调用 + 无收尾标签 → 触发催办
        ScriptedReply::Sse(sse_text_reply(FIRST_PASS)),
        // 催办轮严格照约定收尾：整条回复只有这一行标签
        completion_reply("已完成"),
    ])
    .await;

    let body = run_nudge_probe(&upstream, log, nudge_probe(true)).await;

    assert_eq!(body.matches("event: response.completed").count(), 1);
    assert!(
        !body.to_lowercase().contains("<zen_"),
        "标签不得下发到 codex：{body}"
    );
    // 收集下发的可见文本（output_text.done 的 text）：只有首轮那一句，续跑轮为空
    let visible: Vec<String> = body
        .split("\n\n")
        .filter(|block| block.starts_with("event: response.output_text.done"))
        .filter_map(|block| block.lines().nth(1))
        .filter_map(|data| {
            let payload: Value =
                serde_json::from_str(data.trim_start_matches("data:").trim()).ok()?;
            payload["text"].as_str().map(str::to_string)
        })
        .collect();
    assert!(!visible.is_empty(), "应有 output_text.done：{body}");
    assert_eq!(
        visible.concat(),
        FIRST_PASS,
        "续跑轮不得新增任何可见字符：{visible:?}"
    );

    let calls = rec.lock().await.clone();
    assert_eq!(calls.len(), 2, "应为：首轮 + 催办轮：{calls:?}");
    let joined = read_session_log(&dir);
    assert!(
        joined.contains("原因=task_completed") && joined.contains("模式=default"),
        "{joined}"
    );
    assert!(
        joined.contains("标签内容=已完成"),
        "被剥离的标签载荷要进日志：{joined}"
    );
}

/// 模型不守约定、标签里写了一整句话：照样按结构剥离（剥离不看载荷），日志记全。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_strips_tag_even_when_payload_is_free_text() {
    let dir = tempfile::TempDir::new().unwrap();
    let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
    let (upstream, rec) = spawn_mock_zen_scripted(vec![ScriptedReply::Sse(sse_text_reply(
        "我把 base_url 的测试补上了。\n<zen_task_completed>我判断已经全部完成：新增了 3 个用例，并跑通了 cargo test</zen_task_completed>",
    ))])
    .await;

    let body = run_nudge_probe(&upstream, log, nudge_probe(true)).await;

    assert_eq!(body.matches("event: response.completed").count(), 1);
    assert_eq!(rec.lock().await.len(), 1, "首轮带标签即收尾");
    assert!(
        body.contains("我把 base_url 的测试补上了。") && !body.to_lowercase().contains("<zen_"),
        "自由文本载荷也必须整段剥离：{body}"
    );
    let joined = read_session_log(&dir);
    assert!(
        joined.contains("标签内容=我判断已经全部完成"),
        "被删载荷要进日志：{joined}"
    );
}

/// 会话标题生成请求（应用后台临时线程）：整轮放行，不催办、不注入。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_skipped_for_title_task() {
    let dir = tempfile::TempDir::new().unwrap();
    let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
    let (upstream, rec) =
        spawn_mock_zen_scripted(vec![ScriptedReply::Sse(sse_text_reply("zen 看门狗改进"))]).await;

    let body = run_nudge_probe(&upstream, log, nudge_probe_title_task(true)).await;

    assert_eq!(body.matches("event: response.completed").count(), 1);
    assert!(body.contains("zen 看门狗改进"), "{body}");
    assert_eq!(
        rec.lock().await.len(),
        1,
        "标题任务不应注入续跑提醒：{:?}",
        rec.lock().await
    );
    let joined = read_session_log(&dir);
    assert!(
        joined.contains("event=zen_proxy.nudge_skipped") && joined.contains("原因=title_task"),
        "{joined}"
    );
}

/// 首轮教学（默认模式）：契约挂在 `instructions` 尾部（→ system 消息），首轮请求就带；
/// 续跑提醒仍然只出现在续跑轮，两者不是同一段文本。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_default_mode_first_pass_request_carries_contract() {
    let (upstream, rec) = spawn_mock_zen_scripted(vec![
        ScriptedReply::Sse(sse_text_reply("我先说明一下接下来要做的事。")),
        completion_reply("已完成：只是说明"),
    ])
    .await;

    let body = run_nudge_probe(&upstream, None, nudge_probe(true)).await;

    assert_eq!(body.matches("event: response.completed").count(), 1);
    let calls = rec.lock().await.clone();
    assert_eq!(calls.len(), 2, "应「首轮 + 续跑」：{calls:?}");
    // 首轮：system 里就是默认模式契约；不含续跑提醒，也不含计划模式的出口标签
    let first_system = system_text(&calls[0]);
    assert!(
        first_system.contains(DEFAULT_MODE_CONTRACT_TEXT),
        "首轮应带默认模式契约：{}",
        calls[0]
    );
    assert!(
        !first_system.contains("自动续跑"),
        "首轮不该有续跑提醒：{first_system}"
    );
    assert!(!first_system.contains(PLAN_CANCEL_MARKER), "{first_system}");
    assert!(
        !first_system.contains(PLAN_UNACHIEVABLE_MARKER),
        "{first_system}"
    );
    // 续跑轮：契约仍在，历史尾部追加了续跑提醒
    assert!(
        system_text(&calls[1]).contains(DEFAULT_MODE_CONTRACT_TEXT),
        "续跑轮应继续带契约：{}",
        calls[1]
    );
    let injected = last_message_text(&calls[1]);
    assert!(
        injected.contains(TASK_COMPLETED_MARKER) && injected.contains("自动续跑"),
        "续跑轮尾部应是带标签的提醒：{injected}"
    );
}

/// 首轮教学（计划模式）：只前置两个出口标签，不前置「必须给完整方案」；两种模式的契约互不串味。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_plan_mode_first_pass_request_carries_exit_contract() {
    let (upstream, rec) = spawn_mock_zen_scripted(vec![ScriptedReply::Sse(sse_text_reply(
        "<proposed_plan>\n1. 做 A\n</proposed_plan>",
    ))])
    .await;

    let body = run_nudge_probe(
        &upstream,
        None,
        nudge_probe_with_mode(true, &plan_mode_text()),
    )
    .await;

    assert_eq!(body.matches("event: response.completed").count(), 1);
    let calls = rec.lock().await.clone();
    assert_eq!(calls.len(), 1, "已交付的计划不该再有第二轮：{calls:?}");
    let system = system_text(&calls[0]);
    assert!(
        system.contains(PLAN_MODE_CONTRACT_TEXT),
        "计划模式首轮应带出口契约：{}",
        calls[0]
    );
    assert!(system.contains(PLAN_CANCEL_MARKER), "{system}");
    assert!(system.contains(PLAN_UNACHIEVABLE_MARKER), "{system}");
    // 模式不串味：计划模式请求不带默认模式收尾标签，默认模式请求不带计划出口标签
    assert!(!system.contains(TASK_COMPLETED_MARKER), "{system}");
    assert!(
        !system.contains("都必须给出完整方案"),
        "首轮教学不前置强制口径：{system}"
    );
}

/// 首轮教学的门槛：标题线程与「没有工具声明」的请求都不注入契约
/// （非流式由 `contract_injection_eligible` 单测覆盖）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn contract_not_injected_for_title_task_or_tool_less_request() {
    // ① 标题线程：整轮放行，system 里没有契约
    let (upstream, rec) =
        spawn_mock_zen_scripted(vec![ScriptedReply::Sse(sse_text_reply("zen 首轮教学"))]).await;
    let body = run_nudge_probe(&upstream, None, nudge_probe_title_task(true)).await;
    assert_eq!(body.matches("event: response.completed").count(), 1);
    let calls = rec.lock().await.clone();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(
        !system_text(&calls[0]).contains(DEFAULT_MODE_CONTRACT_TEXT),
        "标题线程不该注入契约：{}",
        calls[0]
    );

    // ② 没有工具声明：与催办同一门槛，不注入
    let (upstream, rec) =
        spawn_mock_zen_scripted(vec![ScriptedReply::Sse(sse_text_reply("纯聊天回答"))]).await;
    let body = run_nudge_probe(&upstream, None, nudge_probe(false)).await;
    assert_eq!(body.matches("event: response.completed").count(), 1);
    let calls = rec.lock().await.clone();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(
        !system_text(&calls[0]).contains(DEFAULT_MODE_CONTRACT_TEXT),
        "无工具请求不该注入契约：{}",
        calls[0]
    );
}

/// 注入上限只按**单次请求**计：同一个 `ProxyState`（同一个会话）连发两次请求，第二次
/// 仍然是「首轮 + 续跑」各打一次上游——不再有跨请求的连续计数与静默窗口。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nudge_injections_are_per_request() {
    let dir = tempfile::TempDir::new().unwrap();
    let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
    let headers = session_headers("thread-per-request");
    // 一个 state 贯穿两次请求：跨请求不再有任何计数状态
    let shared = nudge_probe_state("http://127.0.0.1:1", log.clone());

    for round in 1..=2 {
        let (upstream, rec) = spawn_mock_zen_scripted(vec![
            ScriptedReply::Sse(sse_text_reply("接下来我会把测试补上。")),
            completion_reply("已完成：补了测试"),
        ])
        .await;
        let mut state = shared.clone();
        state.base_url = upstream;
        let body = run_nudge_probe_state(state, headers.clone(), nudge_probe(true)).await;
        assert_eq!(body.matches("event: response.completed").count(), 1);
        assert_eq!(
            rec.lock().await.len(),
            2,
            "第 {round} 次请求都应是「首轮 + 续跑」：{:?}",
            rec.lock().await
        );
    }
    let joined = read_session_log(&dir);
    assert!(
        joined.contains("本请求催办次数=1/4") && !joined.contains("event=zen_proxy.nudge_limited"),
        "两次请求应各注入一次、都不触发上限：{joined}"
    );
}

fn responses_probe() -> Body {
    Body::from(json!({ "model": "m", "input": "hi", "stream": false }).to_string())
}

fn proxy_state(base_url: String, log: ZenLog) -> ProxyState {
    proxy_state_traced(base_url, log, TraceSink::disabled())
}

fn proxy_state_traced(base_url: String, log: ZenLog, trace: TraceSink) -> ProxyState {
    ProxyState {
        session: "ses_fixed123".into(),
        zen_body_patch: is_zen_upstream(&base_url),
        base_url,
        log,
        trace,
        requires_reasoning_rc: Arc::new(AtomicBool::new(false)),
        session_map: Arc::new(SessionMap::default()),
        modes: Arc::new(ThreadModeRegistry::default()),
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
        TraceSink::disabled(),
        Arc::new(ThreadModeRegistry::default()),
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

    let status = apply(
        &mut handle,
        false,
        port,
        upstream,
        None,
        TraceSink::disabled(),
        Arc::new(ThreadModeRegistry::default()),
    )
    .await;
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

    let status = apply(
        &mut handle,
        true,
        port,
        upstream_a.clone(),
        None,
        TraceSink::disabled(),
        Arc::new(ThreadModeRegistry::default()),
    )
    .await;
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
    let status = apply(
        &mut handle,
        true,
        port,
        upstream_a.clone(),
        None,
        TraceSink::disabled(),
        Arc::new(ThreadModeRegistry::default()),
    )
    .await;
    assert!(status.running, "{:?}", status.error);
    let status = apply(
        &mut handle,
        true,
        port,
        format!("{upstream_a}/"),
        None,
        TraceSink::disabled(),
        Arc::new(ThreadModeRegistry::default()),
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
    let status = apply(
        &mut handle,
        true,
        port,
        upstream_b.clone(),
        None,
        TraceSink::disabled(),
        Arc::new(ThreadModeRegistry::default()),
    )
    .await;
    assert!(status.running, "{:?}", status.error);
    probe(port).await;
    let second = last_row(&rec_b).await.expect("上游 B 应已收到请求");
    assert_ne!(first.split(' ').nth(2), second.split(' ').nth(2));
    assert_eq!(rec_a.lock().await.len(), before_a);

    // 端口变化 → 也重启。
    let other_port = free_loopback_port();
    let status = apply(
        &mut handle,
        true,
        other_port,
        upstream_b,
        None,
        TraceSink::disabled(),
        Arc::new(ThreadModeRegistry::default()),
    )
    .await;
    assert!(status.running, "{:?}", status.error);
    assert_eq!(status.port, other_port);
    probe(other_port).await;

    // 关闭 → 停止。
    let status = apply(
        &mut handle,
        false,
        other_port,
        upstream_a,
        None,
        TraceSink::disabled(),
        Arc::new(ThreadModeRegistry::default()),
    )
    .await;
    assert!(!status.running);
    assert!(handle.is_none());
}

/// 起一个"模仿 DeepSeek 思考模式 + 声明 tools"的 mock：**任一** assistant 消息
/// （带 content 或带 tool_calls）缺 `reasoning_content` 就回 400（原文照抄），全带上则 200。
/// 每次请求记录一行 `rc=on|off`。
async fn spawn_mock_requiring_reasoning_rc(rec: Arc<AsyncMutex<Vec<String>>>) -> String {
    use axum::response::IntoResponse;
    let app = Router::new().route(
        "/chat/completions",
        axum::routing::post(move |Json(body): Json<Value>| {
            let rec = rec.clone();
            async move {
                let messages = body
                    .get("messages")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let assistants = messages
                    .iter()
                    .filter(|m| m.get("role").and_then(Value::as_str) == Some("assistant"))
                    .collect::<Vec<_>>();
                let all_have_rc = !assistants.is_empty()
                    && assistants
                        .iter()
                        .all(|m| m.get("reasoning_content").is_some());
                rec.lock()
                    .await
                    .push(format!("rc={}", if all_have_rc { "on" } else { "off" }));
                if !assistants.is_empty() && !all_have_rc {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "error": {
                            "message": "The `reasoning_content` in the thinking mode must be passed back to the API.",
                            "type": "invalid_request_error"
                        } })),
                    )
                        .into_response();
                }
                (
                    StatusCode::OK,
                    Json(json!({
                        "id": "chatcmpl-mock",
                        "object": "chat.completion",
                        "created": 0,
                        "model": "m",
                        "choices": [{
                            "index": 0,
                            "message": { "role": "assistant", "content": "ok" },
                            "finish_reason": "stop"
                        }]
                    })),
                )
                    .into_response()
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
async fn learns_and_retries_reasoning_content_requirement() {
    let rec: Arc<AsyncMutex<Vec<String>>> = Arc::new(AsyncMutex::new(Vec::new()));
    let upstream = spawn_mock_requiring_reasoning_rc(rec.clone()).await;
    let dir = tempfile::TempDir::new().unwrap();
    let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
    let state = ProxyState {
        session: "ses_fixed123".into(),
        zen_body_patch: is_zen_upstream(&upstream),
        base_url: upstream,
        log,
        trace: TraceSink::disabled(),
        requires_reasoning_rc: Arc::new(AtomicBool::new(false)),
        session_map: Arc::new(SessionMap::default()),
        modes: Arc::new(ThreadModeRegistry::default()),
    };
    let probe = |state: ProxyState| async move {
        let uri: Uri = "/responses".parse().unwrap();
        let body = Body::from(
            json!({
                "model": "m",
                "stream": false,
                "input": [
                    { "type": "message", "role": "user", "content": "hi" },
                    { "type": "reasoning", "id": "rs_1", "summary": [
                        { "type": "summary_text", "text": "先看代码" }
                    ] },
                    { "type": "message", "role": "assistant", "content": "我先看一下" },
                    { "type": "function_call", "call_id": "call_1", "name": "shell", "arguments": "{}" },
                    { "type": "function_call_output", "call_id": "call_1", "output": "ok" }
                ]
            })
            .to_string(),
        );
        handle_any(
            State(state),
            Method::POST,
            OriginalUri(uri),
            HeaderMap::new(),
            body,
        )
        .await
    };

    // 第一次：上游因缺少 reasoning_content 回 400 → 代理打开开关内部重试 → 客户端只看到 200。
    let resp = probe(state.clone()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        rec.lock().await.clone(),
        vec!["rc=off".to_string(), "rc=on".to_string()]
    );
    assert!(state.requires_reasoning_rc.load(Ordering::Relaxed));

    // 第二次：开关已粘在代理实例上 → 只发一次请求且直接带字段。
    rec.lock().await.clear();
    let resp = probe(state.clone()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(rec.lock().await.clone(), vec!["rc=on".to_string()]);

    let logged = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| std::fs::read_to_string(e.path()).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        logged.contains("zen_proxy.reasoning_content_enabled")
            && logged.contains("zen_proxy.forward_attempt")
            && logged.contains("assistant_with_content=1")
            && logged.contains("assistant_with_content_rc=0")
            && logged.contains("zen_proxy.forward")
            && logged.contains("reasoning_rc=on"),
        "{logged}"
    );
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
    let state = proxy_state(base_url.clone(), None);
    let resp = forward(&req, &HeaderMap::new(), true, &state, "req_test")
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
    let state = proxy_state(base_url.clone(), None);
    let resp = forward(&req, &HeaderMap::new(), true, &state, "req_test")
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
    assert!(
        got.get("stream_options").is_none(),
        "重试不应再带 stream_options"
    );
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
    let state = proxy_state(base_url.clone(), None);
    let resp = forward(&req, &HeaderMap::new(), true, &state, "req_test")
        .await
        .expect("forward 应成功");
    assert!(resp.status.is_success());
    assert_eq!(resp.dropped_fields, vec!["reasoning_effort"]);
    assert_eq!(*count.lock().await, 2, "应恰好重试一次");

    let got = last.lock().await.clone().expect("mock 应已收到请求");
    assert!(
        got.get("reasoning_effort").is_none(),
        "重试不应再带被拒字段"
    );
    assert_eq!(
        got["stream_options"]["include_usage"], true,
        "未被拒的可选字段应保留"
    );
}

/// Zen 上游（补丁开启）：上游实收的 chat 请求体同时带 `max_tokens` 与 6 个假工具，
/// 真实工具保留在前、顺序不变。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zen_body_patch_reaches_upstream_request_body() {
    let rec: Arc<AsyncMutex<Option<Received>>> = Arc::new(AsyncMutex::new(None));
    let base_url = spawn_mock_zen(rec.clone()).await;

    let req = json!({
        "model": "m",
        "input": "hi",
        "stream": false,
        "tools": [{
            "type": "function",
            "name": "shell",
            "description": "run shell",
            "parameters": { "type": "object", "properties": {} }
        }]
    });
    let mut state = proxy_state(base_url, None);
    state.zen_body_patch = true;
    let resp = forward(&req, &HeaderMap::new(), false, &state, "req_test")
        .await
        .expect("forward 应成功");
    assert!(resp.status.is_success());

    let got = rec.lock().await.clone().expect("mock 应已收到请求");
    assert_eq!(
        got.body["max_tokens"], ZEN_MAX_TOKENS,
        "应补门禁要求的 max_tokens"
    );
    let names: Vec<&str> = got.body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec!["shell", "bash", "edit", "glob", "grep", "read", "write"],
        "真实工具保留在前、缺的假工具按固定顺序补在后：{}",
        got.body
    );
    assert_eq!(
        got.body["tools"][1]["function"]["description"],
        ZEN_FAKE_TOOL_DESCRIPTION
    );
}

/// 非 Zen 上游（自建 / 第三方兼容端点）：同一个入站请求一个字段都不补。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zen_body_patch_is_off_for_non_zen_upstream() {
    let rec: Arc<AsyncMutex<Option<Received>>> = Arc::new(AsyncMutex::new(None));
    let base_url = spawn_mock_zen(rec.clone()).await;

    let req = json!({
        "model": "m",
        "input": "hi",
        "stream": false,
        "tools": [{
            "type": "function",
            "name": "shell",
            "description": "run shell",
            "parameters": { "type": "object", "properties": {} }
        }]
    });
    // 补丁开关由 base_url 派生：回环地址不是 Zen 上游，自然关闭
    let state = proxy_state(base_url, None);
    assert!(!state.zen_body_patch, "回环地址不应被当成 Zen 上游");
    let resp = forward(&req, &HeaderMap::new(), false, &state, "req_test")
        .await
        .expect("forward 应成功");
    assert!(resp.status.is_success());

    let got = rec.lock().await.clone().expect("mock 应已收到请求");
    assert!(
        got.body.get("max_tokens").is_none(),
        "非 Zen 上游不补 max_tokens：{}",
        got.body
    );
    let names: Vec<&str> = got.body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["shell"], "非 Zen 上游不补假工具：{}", got.body);
}

/// 上游在 4xx 错误体里指名 `max_tokens`：摘掉后重试一次（复用既有可选字段降级路径），
/// 回环重试仍带工具补丁。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zen_max_tokens_is_dropped_and_retried_when_rejected() {
    let count: Arc<AsyncMutex<usize>> = Arc::new(AsyncMutex::new(0));
    let last: Arc<AsyncMutex<Option<Value>>> = Arc::new(AsyncMutex::new(None));
    let base_url = spawn_mock_zen_rejecting_field(count.clone(), last.clone(), "max_tokens").await;

    let req = json!({ "model": "m", "input": "hi", "stream": false });
    let mut state = proxy_state(base_url, None);
    state.zen_body_patch = true;
    let resp = forward(&req, &HeaderMap::new(), false, &state, "req_test")
        .await
        .expect("forward 应成功");
    assert!(resp.status.is_success());
    assert_eq!(
        resp.dropped_fields,
        vec!["max_tokens"],
        "应标记被摘掉的字段"
    );
    assert_eq!(*count.lock().await, 2, "应恰好重试一次");

    let got = last.lock().await.clone().expect("mock 应已收到请求");
    assert!(
        got.get("max_tokens").is_none(),
        "重试不应再带 max_tokens：{got}"
    );
    assert_eq!(
        got["tools"].as_array().unwrap().len(),
        ZEN_REQUIRED_TOOL_NAMES.len(),
        "工具补丁不受可选字段摘除影响：{got}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unrelated_4xx_is_not_retried() {
    let count: Arc<AsyncMutex<usize>> = Arc::new(AsyncMutex::new(0));
    let base_url = spawn_mock_zen_bad_request(count.clone()).await;

    let req = json!({ "model": "m", "input": "hi", "stream": true });
    let state = proxy_state(base_url.clone(), None);
    let resp = forward(&req, &HeaderMap::new(), true, &state, "req_test")
        .await
        .expect("forward 应成功");
    assert!(!resp.status.is_success());
    assert!(resp.dropped_fields.is_empty(), "未指名可选字段时不应摘字段");
    assert_eq!(*count.lock().await, 1, "不应重试");
    assert!(
        resp.reasoning_rc_change.is_none(),
        "未提及 reasoning_content 时不应翻转回传开关"
    );
    assert!(!resp.reasoning_rc);
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
    let state = proxy_state(base_url.clone(), log.clone());
    let out = proxy_stream_response(
        state,
        HeaderMap::new(),
        "req_test".to_string(),
        req,
        resp,
        TraceCall::disabled(),
    )
    .await;
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

// -----------------------------------------------------------------------
// 内容诊断日志（logs/zen）：请求体 + 上游响应原文 + 收尾事件 + 可疑标记
// -----------------------------------------------------------------------

/// trace 目录里的全部文件（按文件名排序）。
fn trace_files(dir: &std::path::Path) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                std::fs::read_to_string(entry.path()).unwrap_or_default(),
            )
        })
        .collect();
    files.sort();
    files
}

fn trace_file_with(dir: &std::path::Path, suffix: &str) -> (String, String) {
    trace_files(dir)
        .into_iter()
        .find(|(name, _)| name.ends_with(suffix))
        .unwrap_or_else(|| panic!("应存在 {suffix} 文件"))
}

fn traced_sink(dir: &std::path::Path) -> TraceSink {
    TraceSink::new(Arc::new(crate::codex::zen_trace::ZenTrace::new(
        dir.to_path_buf(),
    )))
}

/// 起一个返回固定 SSE 原文的 mock Zen（POST /chat/completions）。
async fn spawn_mock_zen_sse(chunks: Vec<&'static str>) -> String {
    let app = Router::new().route(
        "/chat/completions",
        axum::routing::post(move |Json(_body): Json<Value>| {
            let chunks = chunks.clone();
            async move {
                let stream = futures_util::stream::iter(
                    chunks
                        .into_iter()
                        .map(|chunk| {
                            Ok::<_, std::io::Error>(axum::body::Bytes::from(chunk.to_string()))
                        })
                        .collect::<Vec<_>>(),
                );
                axum::response::Response::builder()
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .body(Body::from_stream(stream))
                    .unwrap()
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

#[test]
fn suspicious_flags_cover_each_branch() {
    let flags = |facts: SuspiciousFacts<'_>| flags_field(&suspicious_flags(&facts));
    // 正常收尾：带工具调用 + 有 usage → 无标记
    assert_eq!(
        flags(SuspiciousFacts {
            finish_reason: Some("tool_calls"),
            call_count: 2,
            usage_present: true,
            ..Default::default()
        }),
        "-"
    );
    // 模型什么都没输出就 stop（最符合「任务未完成就结束」）
    assert_eq!(
        flags(SuspiciousFacts {
            finish_reason: Some("stop"),
            usage_present: true,
            ..Default::default()
        }),
        "stop_without_output"
    );
    // 只输出文本、没有工具调用
    assert_eq!(
        flags(SuspiciousFacts {
            finish_reason: Some("stop"),
            text_chars: 12,
            usage_present: true,
            ..Default::default()
        }),
        "stop_without_tool_call"
    );
    // 上游截断
    assert_eq!(
        flags(SuspiciousFacts {
            finish_reason: Some("length"),
            usage_present: true,
            ..Default::default()
        }),
        "truncated"
    );
    // 本次历史里已有失败的补丁调用（模型在补丁格式上打转）
    assert_eq!(
        flags(SuspiciousFacts {
            finish_reason: Some("tool_calls"),
            call_count: 1,
            usage_present: true,
            patch_retry: true,
            ..Default::default()
        }),
        "patch_retry"
    );
    // 失败收尾（无 finish_reason）
    assert_eq!(
        flags(SuspiciousFacts {
            failed: true,
            usage_present: true,
            ..Default::default()
        }),
        "failed,finish_reason_missing"
    );
    // 缺 usage / 宽限超时 / 上游非 2xx
    assert_eq!(
        flags(SuspiciousFacts {
            finish_reason: Some("tool_calls"),
            call_count: 1,
            ..Default::default()
        }),
        "usage_missing"
    );
    assert_eq!(
        flags(SuspiciousFacts {
            finish_reason: Some("tool_calls"),
            call_count: 1,
            usage_present: true,
            usage_timeout: true,
            ..Default::default()
        }),
        "usage_timeout"
    );
    assert_eq!(
        flags(SuspiciousFacts {
            upstream_http_error: true,
            usage_present: true,
            ..Default::default()
        }),
        "upstream_http_error,finish_reason_missing"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trace_records_stream_request_response_and_summary() {
    let dir = tempfile::TempDir::new().unwrap();
    let upstream = spawn_mock_zen_sse(vec![
        "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}\n\n",
        "data: [DONE]\n\n",
    ])
    .await;
    let state = proxy_state_traced(upstream, None, traced_sink(dir.path()));
    let resp = handle_any(
        State(state),
        Method::POST,
        OriginalUri("/responses".parse().unwrap()),
        HeaderMap::new(),
        Body::from(
            json!({
                "model": "mimo-v2.5-flash",
                "stream": true,
                "input": [{
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "hi" }]
                }]
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let text = body_text(resp).await;
    assert!(text.contains("event: response.completed"), "{text}");

    let (_, request) = trace_file_with(dir.path(), ".request.json");
    assert!(
        request.contains("\"model\": \"mimo-v2.5-flash\""),
        "{request}"
    );
    assert!(request.contains("\"role\": \"user\""), "{request}");
    assert!(request.contains("\"content\": \"hi\""), "{request}");
    assert!(request.contains("\"include_usage\": true"), "{request}");

    let (_, response) = trace_file_with(dir.path(), ".response.sse");
    assert!(
        response.contains("data: [DONE]"),
        "上游行应原样落盘：{response}"
    );
    assert!(
        response.contains("\"finish_reason\":\"stop\""),
        "{response}"
    );

    let (_, summary) = trace_file_with(dir.path(), ".summary.txt");
    assert!(summary.contains("call_id=req_"), "{summary}");
    assert!(summary.contains("attempt=1"), "{summary}");
    assert!(summary.contains("upstream_url="), "{summary}");
    assert!(summary.contains("authorization=absent"), "{summary}");
    assert!(summary.contains("finish_reason=stop"), "{summary}");
    assert!(summary.contains("usage=present"), "{summary}");
    // 剥离口径进摘要：普通回复没有被剥离的字符
    assert!(summary.contains("stripped_chars=0"), "{summary}");
    assert!(
        summary.contains("suspicious=stop_without_tool_call"),
        "{summary}"
    );
    assert!(
        summary.contains("translated_terminal_event=event: response.completed"),
        "{summary}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trace_marks_stop_without_output_as_suspicious() {
    let dir = tempfile::TempDir::new().unwrap();
    let upstream = spawn_mock_zen_sse(vec![
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    ])
    .await;
    let state = proxy_state_traced(upstream, None, traced_sink(dir.path()));
    let resp = handle_any(
        State(state),
        Method::POST,
        OriginalUri("/responses".parse().unwrap()),
        HeaderMap::new(),
        Body::from(json!({ "model": "m", "stream": true, "input": "hi" }).to_string()),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let text = body_text(resp).await;
    assert!(text.contains("event: response.completed"), "{text}");

    let (_, summary) = trace_file_with(dir.path(), ".summary.txt");
    assert!(
        summary.contains("suspicious=stop_without_output,usage_missing"),
        "{summary}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trace_records_each_retry_attempt() {
    let dir = tempfile::TempDir::new().unwrap();
    let trace = traced_sink(dir.path());
    let count: Arc<AsyncMutex<usize>> = Arc::new(AsyncMutex::new(0));
    let last: Arc<AsyncMutex<Option<Value>>> = Arc::new(AsyncMutex::new(None));
    let base_url =
        spawn_mock_zen_rejecting_field(count.clone(), last.clone(), "stream_options").await;

    let req = json!({ "model": "m", "input": "hi", "stream": true });
    let state = proxy_state_traced(base_url.clone(), None, trace);
    let resp = forward(&req, &HeaderMap::new(), true, &state, "req_retry")
        .await
        .expect("forward 应成功");
    assert!(resp.status.is_success());
    drop(resp);

    let files = trace_files(dir.path());
    let a1_request = files
        .iter()
        .find(|(name, _)| name.contains("req_retry-a1.request.json"))
        .expect("第一次尝试应有请求体")
        .clone();
    let a2_request = files
        .iter()
        .find(|(name, _)| name.contains("req_retry-a2.request.json"))
        .expect("重试应有独立请求体")
        .clone();
    assert!(a1_request.1.contains("stream_options"), "{}", a1_request.1);
    assert!(
        !a2_request.1.contains("stream_options"),
        "重试不应再带被拒字段：{}",
        a2_request.1
    );
    let a1_summary = files
        .iter()
        .find(|(name, _)| name.contains("req_retry-a1.summary.txt"))
        .expect("第一次尝试应有摘要")
        .clone();
    assert!(a1_summary.1.contains("retried=true"), "{}", a1_summary.1);
    assert!(
        a1_summary.1.contains("suspicious=upstream_http_error"),
        "{}",
        a1_summary.1
    );
    let (_, a1_error) = trace_file_with(dir.path(), "req_retry-a1.response.txt");
    assert!(a1_error.contains("stream_options"), "{a1_error}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trace_records_upstream_error_body() {
    let dir = tempfile::TempDir::new().unwrap();
    let count: Arc<AsyncMutex<usize>> = Arc::new(AsyncMutex::new(0));
    let upstream = spawn_mock_zen_bad_request(count.clone()).await;
    let state = proxy_state_traced(upstream, None, traced_sink(dir.path()));
    let resp = handle_any(
        State(state),
        Method::POST,
        OriginalUri("/responses".parse().unwrap()),
        HeaderMap::new(),
        responses_probe(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let (_, error_body) = trace_file_with(dir.path(), ".response.txt");
    assert!(error_body.contains("must be valid JSON"), "{error_body}");
    assert_eq!(
        error_body.matches("must be valid JSON").count(),
        1,
        "不可重试的上游错误只应落盘一次：{error_body}"
    );
    let (_, summary) = trace_file_with(dir.path(), ".summary.txt");
    assert!(summary.contains("upstream_status=400"), "{summary}");
    assert!(
        summary.contains("suspicious=upstream_http_error"),
        "{summary}"
    );
}
