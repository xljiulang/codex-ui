//! 微信 ilink bot 协议客户端（纯 Rust，替代 Node sidecar）。
//!
//! 实现文本收发链路的完整协议：二维码登录、getUpdates 长轮询、sendmessage、
//! contextToken 管理、账号/会话/同步缓冲/回复上下文的文件存储；媒体上传下载不在
//! 范围内（UI 本就忽略媒体）。事件经 mpsc 推送给桥（WechatEvent），语义对齐
//! 原 sidecar 的 stdio 事件。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use futures_util::Future;
use serde_json::{json, Value};
use tokio::sync::{mpsc, Mutex, Notify};

/// 默认 API 基地址（与 wechat-channel 一致）。
pub const DEFAULT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
/// ilink bot 类型。
const BOT_TYPE: &str = "3";
/// base_info 里的通道版本（沿用库版本串，保持兼容）。
const CHANNEL_VERSION: &str = "1.1.0";
/// getUpdates 长轮询超时：客户端超时视为空响应继续轮询。
const LONG_POLL_TIMEOUT: Duration = Duration::from_secs(35);
/// 二维码状态轮询超时：超时视为 wait。
const QR_POLL_TIMEOUT: Duration = Duration::from_secs(35);
/// 登录总超时（与扫码窗口一致）。
const LOGIN_TIMEOUT: Duration = Duration::from_secs(480);
/// 二维码过期最多刷新次数。
const MAX_QR_REFRESH: u32 = 3;
/// 被动回复窗口：24 小时。
const REPLY_WINDOW_MS: u64 = 24 * 60 * 60 * 1000;
/// 普通失败重试间隔。
const RETRY_DELAY: Duration = Duration::from_secs(2);
/// 连续失败后的退避间隔。
const BACKOFF_DELAY: Duration = Duration::from_secs(30);
/// 连续失败阈值。
const MAX_CONSECUTIVE_FAILURES: u32 = 3;
/// getconfig / sendtyping 请求超时（正在输入状态指示相关）。
const TYPING_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// 统一 ID 计数器（替代随机源：唯一性足够，避免额外依赖）。
static ID_SEQ: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// X-WECHAT-UIN 头：新鲜唯一值即可（库用随机 uint32 的十进制字符串 base64）。
fn random_uin() -> String {
    let n = now_ms().wrapping_add(ID_SEQ.fetch_add(1, Ordering::Relaxed));
    BASE64.encode(n.to_string())
}

/// 生成客户端 ID：`{prefix}-{时间戳 hex}-{自增 hex}`（格式不参与服务端校验，仅需唯一）。
fn generate_id(prefix: &str) -> String {
    let r = ID_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{ms:x}-{r:x}", ms = now_ms())
}

/// 账号 ID 标准化：@ 和 . 替换为 -（与 wechat-channel 一致，文件系统安全）。
pub fn normalize_account_id(id: &str) -> String {
    id.replace(['@', '.'], "-")
}

/// base_info：随每个请求携带。
fn build_base_info() -> Value {
    json!({ "channel_version": CHANNEL_VERSION })
}

/// 会话过期判定（收紧规则，等价当前补丁后行为）：
/// errcode == -14，或 errmsg 匹配 session/expired/token expired（不含 timeout）。
pub fn is_session_expired_payload(payload: &Value) -> bool {
    if payload.get("errcode").and_then(|v| v.as_i64()) == Some(-14) {
        return true;
    }
    let msg = payload
        .get("errmsg")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    subseq(&msg, "session", "expired")
        || subseq(&msg, "expired", "session")
        || subseq(&msg, "token", "expired")
}

/// 子串 a 是否在 b 之后出现（对应正则 a.*b）。
fn subseq(s: &str, a: &str, b: &str) -> bool {
    s.find(a)
        .is_some_and(|i| s[i + a.len()..].contains(b))
}

/// 响应是否携带业务错误（ret/errcode 非 0）。
fn is_api_error(resp: &Value) -> bool {
    (resp.get("ret").and_then(|v| v.as_i64()).unwrap_or(0) != 0)
        || (resp.get("errcode").and_then(|v| v.as_i64()).unwrap_or(0) != 0)
}

/// 从消息 item_list 提取文本：TEXT（type=1）text_item.text；VOICE（type=3）voice_item.text。
pub fn extract_message_text(item_list: &Value) -> Option<String> {
    let list = item_list.as_array()?;
    for item in list {
        let ty = item.get("type").and_then(|v| v.as_i64()).unwrap_or(0);
        let text = if ty == 1 {
            item.pointer("/text_item/text").and_then(|v| v.as_str())
        } else if ty == 3 {
            item.pointer("/voice_item/text").and_then(|v| v.as_str())
        } else {
            None
        };
        if let Some(t) = text {
            let t = t.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// 存储（复用 wechannel-data 现有文件布局，已登录账号无缝迁移）
// ---------------------------------------------------------------------------

fn state_dir(root: &Path) -> PathBuf {
    root.join("wechannel-data")
}

fn accounts_index_path(root: &Path) -> PathBuf {
    state_dir(root).join("accounts.json")
}

fn account_path(root: &Path, account_id: &str) -> PathBuf {
    state_dir(root)
        .join("accounts")
        .join(format!("{}.json", normalize_account_id(account_id)))
}

fn session_status_path(root: &Path, account_id: &str) -> PathBuf {
    state_dir(root)
        .join("session-status")
        .join(format!("{}.json", normalize_account_id(account_id)))
}

fn sync_buf_path(root: &Path, account_id: &str) -> PathBuf {
    state_dir(root)
        .join("sync-buf")
        .join(format!("{}.sync.json", normalize_account_id(account_id)))
}

fn reply_context_path(root: &Path, account_id: &str) -> PathBuf {
    state_dir(root)
        .join("reply-context")
        .join(format!("{}.json", normalize_account_id(account_id)))
}

fn read_json(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let body = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(path, body).map_err(|e| e.to_string())
}

fn load_account_ids(root: &Path) -> Vec<String> {
    read_json(&accounts_index_path(root))
        .and_then(|v| v.as_array().cloned())
        .map(|list| {
            list.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn register_account_id(root: &Path, account_id: &str) {
    let mut ids = load_account_ids(root);
    if !ids.contains(&account_id.to_string()) {
        ids.push(account_id.to_string());
        let _ = write_json(&accounts_index_path(root), &json!(ids));
    }
}

fn unregister_account_id(root: &Path, account_id: &str) {
    let ids: Vec<String> = load_account_ids(root)
        .into_iter()
        .filter(|id| id != account_id)
        .collect();
    let _ = write_json(&accounts_index_path(root), &json!(ids));
}

fn load_account(root: &Path, account_id: &str) -> Option<Value> {
    read_json(&account_path(root, account_id))
}

fn save_account(root: &Path, account_id: &str, token: &str, base_url: &str, user_id: &str) {
    let data = json!({
        "token": token,
        "savedAt": now_ms(),
        "baseUrl": base_url,
        "userId": user_id,
    });
    if write_json(&account_path(root, account_id), &data).is_ok() {
        register_account_id(root, account_id);
    }
}

fn delete_account(root: &Path, account_id: &str) {
    let _ = std::fs::remove_file(account_path(root, account_id));
    unregister_account_id(root, account_id);
}

fn load_session_status(root: &Path, account_id: &str) -> Value {
    read_json(&session_status_path(root, account_id)).unwrap_or_else(|| {
        json!({
            "accountId": account_id,
            "status": "disconnected",
            "changedAt": 0,
        })
    })
}

fn save_session_status(
    root: &Path,
    account_id: &str,
    status: &str,
    error_code: Option<i64>,
    error_message: Option<&str>,
) {
    let mut data = json!({
        "accountId": account_id,
        "status": status,
        "changedAt": now_ms(),
    });
    if let Some(code) = error_code {
        data["errorCode"] = json!(code);
    }
    if let Some(msg) = error_message {
        data["errorMessage"] = json!(msg);
    }
    let _ = write_json(&session_status_path(root, account_id), &data);
}

fn delete_session_status(root: &Path, account_id: &str) {
    let _ = std::fs::remove_file(session_status_path(root, account_id));
}

fn load_sync_buf(root: &Path, account_id: &str) -> String {
    match read_json(&sync_buf_path(root, account_id)) {
        Some(v) => v
            .get("get_updates_buf")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        None => String::new(),
    }
}

fn save_sync_buf(root: &Path, account_id: &str, buf: &str) {
    let _ = write_json(&sync_buf_path(root, account_id), &json!({ "get_updates_buf": buf }));
}

fn load_reply_context(root: &Path, account_id: &str) -> HashMap<String, Value> {
    match read_json(&reply_context_path(root, account_id)) {
        Some(v) => v
            .as_object()
            .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default(),
        None => HashMap::new(),
    }
}

fn save_reply_context(root: &Path, account_id: &str, map: &HashMap<String, Value>) {
    let obj = map.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<Value>();
    let _ = write_json(&reply_context_path(root, account_id), &obj);
}

/// 保存入站消息的 contextToken（24h 被动回复窗口）。
fn set_context_token(root: &Path, account_id: &str, user_id: &str, token: &str, message_id: Option<&str>) {
    let mut map = load_reply_context(root, account_id);
    let now = now_ms();
    let mut entry = json!({
        "peerId": user_id,
        "contextToken": token,
        "lastInboundAt": now,
        "expiresAt": now + REPLY_WINDOW_MS,
    });
    if let Some(mid) = message_id {
        entry["messageId"] = json!(mid);
    }
    map.insert(user_id.to_string(), entry);
    save_reply_context(root, account_id, &map);
}

/// 取有效 contextToken（过期视为无）。
fn get_context_token(root: &Path, account_id: &str, user_id: &str) -> Option<String> {
    let entry = load_reply_context(root, account_id).get(user_id)?.clone();
    let expires_at = entry.get("expiresAt").and_then(|v| v.as_u64())?;
    if expires_at <= now_ms() {
        return None;
    }
    entry.get("contextToken").and_then(|v| v.as_str()).map(str::to_string)
}

fn clear_reply_contexts(root: &Path, account_id: &str) {
    let _ = std::fs::remove_file(reply_context_path(root, account_id));
}

/// 账号快照（对齐原 sidecar 的 accounts 事件：id/name/configured/userId/status）。
pub fn accounts_snapshot(root: &Path) -> Vec<Value> {
    load_account_ids(root)
        .into_iter()
        .map(|id| {
            let account = load_account(root, &id);
            let configured = account.is_some();
            let user_id = account
                .as_ref()
                .and_then(|a| a.get("userId").and_then(|v| v.as_str()))
                .unwrap_or("")
                .to_string();
            let status = load_session_status(root, &id)
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("disconnected")
                .to_string();
            json!({
                "id": id,
                "name": null,
                "configured": configured,
                "userId": user_id,
                "status": status,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// API 抽象（便于用 mock 验证轮询/登录流程）
// ---------------------------------------------------------------------------

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait WechatApi: Send + Sync {
    fn get_qr_code(&self, base_url: &str, bot_type: &str) -> BoxFuture<'_, Result<Value, String>>;
    fn poll_qr_status(&self, base_url: &str, qrcode: &str) -> BoxFuture<'_, Result<Value, String>>;
    fn get_updates(
        &self,
        base_url: &str,
        token: &str,
        buf: &str,
        timeout_ms: u64,
    ) -> BoxFuture<'_, Result<Value, String>>;
    fn send_message(&self, base_url: &str, token: &str, body: Value)
        -> BoxFuture<'_, Result<Value, String>>;
    fn get_config(&self, base_url: &str, token: &str, body: Value)
        -> BoxFuture<'_, Result<Value, String>>;
    fn send_typing(&self, base_url: &str, token: &str, body: Value)
        -> BoxFuture<'_, Result<Value, String>>;
}

/// 真实 HTTP 实现（reqwest）。
#[derive(Clone)]
pub struct ReqwestApi {
    http: reqwest::Client,
}

impl ReqwestApi {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    fn post_json(
        &self,
        base_url: &str,
        endpoint: &str,
        token: Option<&str>,
        body: Value,
        timeout: Duration,
    ) -> BoxFuture<'_, Result<Value, String>> {
        let http = self.http.clone();
        let url = format!("{}/{}", base_url.trim_end_matches('/'), endpoint);
        let body_str = serde_json::to_string(&body).unwrap_or_else(|_| "{}".into());
        let token_owned: Option<String> = token.map(|t| t.trim().to_string());
        Box::pin(async move {
            let mut req = http
                .post(&url)
                .header("Content-Type", "application/json")
                .header("AuthorizationType", "ilink_bot_token")
                .header("Content-Length", body_str.len().to_string())
                .header("X-WECHAT-UIN", random_uin())
                .body(body_str)
                .timeout(timeout);
            if let Some(tok) = token_owned {
                if !tok.trim().is_empty() {
                    req = req.header("Authorization", format!("Bearer {}", tok.trim()));
                }
            }
            let resp = req.send().await.map_err(|e| e.to_string())?;
            let text = resp.text().await.map_err(|e| e.to_string())?;
            if text.trim().is_empty() {
                return Ok(json!({}));
            }
            serde_json::from_str(&text).map_err(|e| format!("响应 JSON 解析失败: {e}"))
        })
    }
}

impl WechatApi for ReqwestApi {
    fn get_qr_code(
        &self,
        base_url: &str,
        bot_type: &str,
    ) -> BoxFuture<'_, Result<Value, String>> {
        let http = self.http.clone();
        let url = format!(
            "{}/ilink/bot/get_bot_qrcode?bot_type={}",
            base_url.trim_end_matches('/'),
            bot_type
        );
        Box::pin(async move {
            let resp = http
                .get(&url)
                .timeout(Duration::from_secs(15))
                .send()
                .await
                .map_err(|e| format!("获取二维码失败: {e}"))?;
            let text = resp.text().await.map_err(|e| e.to_string())?;
            serde_json::from_str(&text).map_err(|e| format!("二维码响应解析失败: {e}"))
        })
    }

    fn poll_qr_status(
        &self,
        base_url: &str,
        qrcode: &str,
    ) -> BoxFuture<'_, Result<Value, String>> {
        let http = self.http.clone();
        let url = format!(
            "{}/ilink/bot/get_qrcode_status?qrcode={}",
            base_url.trim_end_matches('/'),
            urlencode(qrcode)
        );
        Box::pin(async move {
            let resp = http
                .get(&url)
                .header("iLink-App-ClientVersion", "1")
                .timeout(QR_POLL_TIMEOUT)
                .send()
                .await;
            match resp {
                Ok(r) => {
                    let text = r.text().await.map_err(|e| e.to_string())?;
                    if text.trim().is_empty() {
                        return Ok(json!({ "status": "wait" }));
                    }
                    serde_json::from_str(&text).map_err(|e| format!("二维码状态解析失败: {e}"))
                }
                Err(e) if e.is_timeout() => Ok(json!({ "status": "wait" })),
                Err(e) => Err(format!("二维码状态轮询失败: {e}")),
            }
        })
    }

    fn get_updates(
        &self,
        base_url: &str,
        token: &str,
        buf: &str,
        _timeout_ms: u64,
    ) -> BoxFuture<'_, Result<Value, String>> {
        let body = json!({
            "get_updates_buf": buf,
            "base_info": build_base_info(),
        });
        self.post_json(
            base_url,
            "ilink/bot/getupdates",
            Some(token),
            body,
            LONG_POLL_TIMEOUT,
        )
    }

    fn send_message(
        &self,
        base_url: &str,
        token: &str,
        body: Value,
    ) -> BoxFuture<'_, Result<Value, String>> {
        self.post_json(
            base_url,
            "ilink/bot/sendmessage",
            Some(token),
            body,
            Duration::from_secs(15),
        )
    }

    fn get_config(
        &self,
        base_url: &str,
        token: &str,
        body: Value,
    ) -> BoxFuture<'_, Result<Value, String>> {
        self.post_json(
            base_url,
            "ilink/bot/getconfig",
            Some(token),
            body,
            TYPING_REQUEST_TIMEOUT,
        )
    }

    fn send_typing(
        &self,
        base_url: &str,
        token: &str,
        body: Value,
    ) -> BoxFuture<'_, Result<Value, String>> {
        self.post_json(
            base_url,
            "ilink/bot/sendtyping",
            Some(token),
            body,
            TYPING_REQUEST_TIMEOUT,
        )
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 事件与客户端
// ---------------------------------------------------------------------------

/// 桥消费的事件（语义对齐原 sidecar 事件）。
#[derive(Debug, Clone)]
pub enum WechatEvent {
    Qr(String),
    LoginResult {
        login_id: String,
        success: bool,
        message: String,
        account_id: Option<String>,
        user_id: Option<String>,
    },
    Message {
        account_id: String,
        from: String,
        to: String,
        timestamp: u64,
        context_token: String,
        text: Option<String>,
    },
    SessionStatus {
        account_id: String,
        status: String,
        error_code: Option<i64>,
        error_message: Option<String>,
    },
    Error {
        message: String,
        kind: String,
    },
    Accounts(Vec<Value>),
}

pub struct WechatClient<A: WechatApi = ReqwestApi> {
    root: PathBuf,
    api: Arc<A>,
    tx: mpsc::UnboundedSender<WechatEvent>,
    receivers: Arc<Mutex<HashMap<String, Arc<Notify>>>>,
    /// typing_ticket 内存缓存：键为 (account_id, peer)。
    typing_tickets: Mutex<HashMap<(String, String), String>>,
}

impl WechatClient<ReqwestApi> {
    pub fn new(root: PathBuf, tx: mpsc::UnboundedSender<WechatEvent>) -> Self {
        Self::with_api(root, tx, ReqwestApi::new())
    }
}

impl<A: WechatApi + 'static> WechatClient<A> {
    pub fn with_api(root: PathBuf, tx: mpsc::UnboundedSender<WechatEvent>, api: A) -> Self {
        Self {
            root,
            api: Arc::new(api),
            tx,
            receivers: Arc::new(Mutex::new(HashMap::new())),
            typing_tickets: Mutex::new(HashMap::new()),
        }
    }

    pub fn accounts(&self) -> Vec<Value> {
        accounts_snapshot(&self.root)
    }

    /// 发起二维码登录（结果与二维码经事件推送，可用 cancel 取消）。
    pub fn start_login(&self, login_id: String, cancel: Arc<Notify>) {
        let api = self.api.clone();
        let tx = self.tx.clone();
        let root = self.root.clone();
        tokio::spawn(async move {
            let qr_res = api.get_qr_code(DEFAULT_BASE_URL, BOT_TYPE).await;
            let qr = match qr_res {
                Ok(v) => v,
                Err(e) => {
                    let _ = tx.send(WechatEvent::LoginResult {
                        login_id,
                        success: false,
                        message: e,
                        account_id: None,
                        user_id: None,
                    });
                    return;
                }
            };
            let mut qrcode = qr.get("qrcode").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let url = qr
                .get("qrcode_img_content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if url.is_empty() {
                let _ = tx.send(WechatEvent::LoginResult {
                    login_id,
                    success: false,
                    message: "未获取到二维码".into(),
                    account_id: None,
                    user_id: None,
                });
                return;
            }
            let _ = tx.send(WechatEvent::Qr(url));
            let deadline = now_ms() + LOGIN_TIMEOUT.as_millis() as u64;
            let mut refreshes = 0u32;
            loop {
                if now_ms() >= deadline {
                    let _ = tx.send(WechatEvent::LoginResult {
                        login_id,
                        success: false,
                        message: "登录超时，请重试。".into(),
                        account_id: None,
                        user_id: None,
                    });
                    return;
                }
                let poll = api.poll_qr_status(DEFAULT_BASE_URL, &qrcode);
                tokio::pin!(poll);
                let status = tokio::select! {
                    _ = cancel.notified() => {
                        let _ = tx.send(WechatEvent::LoginResult {
                            login_id,
                            success: false,
                            message: "登录已取消".into(),
                            account_id: None,
                            user_id: None,
                        });
                        return;
                    }
                    r = &mut poll => r,
                };
                let st = match status {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = tx.send(WechatEvent::LoginResult {
                            login_id,
                            success: false,
                            message: e,
                            account_id: None,
                            user_id: None,
                        });
                        return;
                    }
                };
                match st.get("status").and_then(|v| v.as_str()).unwrap_or("") {
                    "confirmed" => {
                        let bot_id = st.get("ilink_bot_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let bot_token = st.get("bot_token").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let base_url = st.get("baseurl").and_then(|v| v.as_str()).unwrap_or(DEFAULT_BASE_URL).to_string();
                        let user_id = st.get("ilink_user_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        if bot_id.is_empty() || bot_token.is_empty() {
                            let _ = tx.send(WechatEvent::LoginResult {
                                login_id,
                                success: false,
                                message: "登录确认但缺少账号信息".into(),
                                account_id: None,
                                user_id: None,
                            });
                            return;
                        }
                        save_account(&root, &bot_id, &bot_token, &base_url, &user_id);
                        let _ = tx.send(WechatEvent::LoginResult {
                            login_id,
                            success: true,
                            message: "与微信连接成功！".into(),
                            account_id: Some(bot_id),
                            user_id: Some(user_id),
                        });
                        return;
                    }
                    "expired" => {
                        refreshes += 1;
                        if refreshes > MAX_QR_REFRESH {
                            let _ = tx.send(WechatEvent::LoginResult {
                                login_id,
                                success: false,
                                message: "登录超时：二维码多次过期，请重新开始登录流程。".into(),
                                account_id: None,
                                user_id: None,
                            });
                            return;
                        }
                        match api.get_qr_code(DEFAULT_BASE_URL, BOT_TYPE).await {
                            Ok(nq) => {
                                let new_url = nq
                                    .get("qrcode_img_content")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let new_code = nq.get("qrcode").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                if !new_url.is_empty() {
                                    let _ = tx.send(WechatEvent::Qr(new_url));
                                }
                                if new_code.is_empty() {
                                    let _ = tx.send(WechatEvent::LoginResult {
                                        login_id,
                                        success: false,
                                        message: "刷新二维码失败".into(),
                                        account_id: None,
                                        user_id: None,
                                    });
                                    return;
                                }
                                qrcode = new_code;
                                continue;
                            }
                            Err(e) => {
                                let _ = tx.send(WechatEvent::LoginResult {
                                    login_id,
                                    success: false,
                                    message: format!("刷新二维码失败: {e}"),
                                    account_id: None,
                                    user_id: None,
                                });
                                return;
                            }
                        }
                    }
                    // wait / scaned / 其它未知状态：继续轮询
                    _ => {}
                }
            }
        });
    }

    /// 启动指定账号的消息接收（长轮询任务，幂等）。
    pub async fn start_receiver(&self, account_id: String) {
        {
            let mut map = self.receivers.lock().await;
            if map.contains_key(&account_id) {
                return;
            }
            map.insert(account_id.clone(), Arc::new(Notify::new()));
        }
        let api = self.api.clone();
        let tx = self.tx.clone();
        let root = self.root.clone();
        let cancel = {
            let map = self.receivers.lock().await;
            map.get(&account_id).cloned().unwrap()
        };
        tokio::spawn(async move {
            run_receiver(api, tx, root, account_id, cancel).await;
        });
    }

    /// 停止指定账号的消息接收。
    pub async fn stop_receiver(&self, account_id: &str) {
        let notify = {
            let map = self.receivers.lock().await;
            map.get(account_id).cloned()
        };
        if let Some(n) = notify {
            n.notify_waiters();
        }
        self.receivers.lock().await.remove(account_id);
    }

    /// 退出登录：停接收、清回复上下文与会话状态、删除账号。
    pub async fn logout(&self, account_id: &str) {
        self.stop_receiver(account_id).await;
        clear_reply_contexts(&self.root, account_id);
        delete_session_status(&self.root, account_id);
        delete_account(&self.root, account_id);
    }

    /// 发送文本：校验会话连接与 contextToken（24h 被动窗口）后调用 sendmessage。
    pub async fn send_text(&self, account_id: &str, to: &str, text: &str) -> Result<(), String> {
        let account = load_account(&self.root, account_id).ok_or_else(|| "账号不存在".to_string())?;
        let token = account
            .get("token")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let base_url = account
            .get("baseUrl")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_BASE_URL)
            .to_string();
        let session = load_session_status(&self.root, account_id);
        let status = session.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if status == "session_expired" {
            return Err("微信会话已过期，请重新扫码".into());
        }
        if status != "connected" {
            return Err("微信账号未连接".into());
        }
        let ctx = get_context_token(&self.root, account_id, to)
            .ok_or_else(|| "缺少回复上下文（24 小时被动窗口已过期）".to_string())?;
        let body = json!({
            "msg": {
                "from_user_id": "",
                "to_user_id": to,
                "client_id": generate_id("wechannel"),
                "message_type": 2,
                "message_state": 2,
                "item_list": [ { "type": 1, "text_item": { "text": text } } ],
                "context_token": ctx,
            },
            "base_info": build_base_info(),
        });
        let resp = self.api.send_message(&base_url, &token, body).await?;
        if is_api_error(&resp) {
            if is_session_expired_payload(&resp) {
                let msg = resp
                    .get("errmsg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("session expired")
                    .to_string();
                save_session_status(
                    &self.root,
                    account_id,
                    "session_expired",
                    resp.get("errcode").and_then(|v| v.as_i64()),
                    Some(&msg),
                );
                let _ = self.tx.send(WechatEvent::SessionStatus {
                    account_id: account_id.to_string(),
                    status: "session_expired".into(),
                    error_code: resp.get("errcode").and_then(|v| v.as_i64()),
                    error_message: Some(msg),
                });
                return Err("微信会话已过期，请重新扫码".into());
            }
            let msg = resp
                .get("errmsg")
                .and_then(|v| v.as_str())
                .unwrap_or("发送失败")
                .to_string();
            return Err(msg);
        }
        Ok(())
    }

    /// 取回并缓存对端用户的 typing_ticket（按“账号+用户”缓存，取到即复用）。
    async fn ensure_typing_ticket(&self, account_id: &str, to: &str) -> Result<String, String> {
        let key = (account_id.to_string(), to.to_string());
        if let Some(t) = self.typing_tickets.lock().await.get(&key).cloned() {
            return Ok(t);
        }
        let account = load_account(&self.root, account_id).ok_or_else(|| "账号不存在".to_string())?;
        let token = account
            .get("token")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let base_url = account
            .get("baseUrl")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_BASE_URL)
            .to_string();
        let body = json!({
            "ilink_user_id": to,
            "context_token": get_context_token(&self.root, account_id, to).unwrap_or_default(),
            "base_info": build_base_info(),
        });
        let resp = self.api.get_config(&base_url, &token, body).await?;
        let ticket = resp
            .get("typing_ticket")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if ticket.is_empty() {
            return Err("未获取到 typing_ticket（可能该用户尚无上行消息或会话已失效）".into());
        }
        self.typing_tickets.lock().await.insert(key, ticket.clone());
        Ok(ticket)
    }

    /// 发送“正在输入”状态（status=1 显示 / status=2 取消）。best-effort：账号未连接直接 Ok。
    pub async fn send_typing(&self, account_id: &str, to: &str, status: i32) -> Result<(), String> {
        let account = load_account(&self.root, account_id).ok_or_else(|| "账号不存在".to_string())?;
        let token = account
            .get("token")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let base_url = account
            .get("baseUrl")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_BASE_URL)
            .to_string();
        let session = load_session_status(&self.root, account_id);
        let conn = session.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if conn == "session_expired" || conn != "connected" {
            // 过期或未连接：无意义，直接跳过（不打断调用方）。
            return Ok(());
        }
        let ticket = self.ensure_typing_ticket(account_id, to).await?;
        let body = json!({
            "ilink_user_id": to,
            "typing_ticket": ticket,
            "status": status,
            "base_info": build_base_info(),
        });
        let resp = self.api.send_typing(&base_url, &token, body).await?;
        if is_api_error(&resp) {
            // 任一错误清缓存自愈，下次重取；过期再发事件。
            self.typing_tickets
                .lock()
                .await
                .remove(&(account_id.to_string(), to.to_string()));
            if is_session_expired_payload(&resp) {
                let msg = resp
                    .get("errmsg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("session expired")
                    .to_string();
                save_session_status(
                    &self.root,
                    account_id,
                    "session_expired",
                    resp.get("errcode").and_then(|v| v.as_i64()),
                    Some(&msg),
                );
                let _ = self.tx.send(WechatEvent::SessionStatus {
                    account_id: account_id.to_string(),
                    status: "session_expired".into(),
                    error_code: resp.get("errcode").and_then(|v| v.as_i64()),
                    error_message: Some(msg),
                });
                return Err("微信会话已过期，请重新扫码".into());
            }
            let msg = resp
                .get("errmsg")
                .and_then(|v| v.as_str())
                .unwrap_or("发送 typing 状态失败")
                .to_string();
            return Err(msg);
        }
        Ok(())
    }

    /// 停止全部接收器（应用退出时调用）。
    pub async fn shutdown(&self) {
        let ids: Vec<String> = {
            let map = self.receivers.lock().await;
            map.keys().cloned().collect()
        };
        for id in ids {
            self.stop_receiver(&id).await;
        }
    }
}

/// 单账号长轮询循环：读同步缓冲 → 上报 connected → 轮询 getUpdates。
async fn run_receiver<A: WechatApi>(
    api: Arc<A>,
    tx: mpsc::UnboundedSender<WechatEvent>,
    root: PathBuf,
    account_id: String,
    cancel: Arc<Notify>,
) {
    let Some(account) = load_account(&root, &account_id) else {
        let _ = tx.send(WechatEvent::Error {
            message: format!("账号 {account_id} 不存在，无法接收"),
            kind: "receiver".into(),
        });
        return;
    };
    let token = account
        .get("token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let base_url = account
        .get("baseUrl")
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_BASE_URL)
        .to_string();

    save_session_status(&root, &account_id, "connected", None, None);
    let _ = tx.send(WechatEvent::SessionStatus {
        account_id: account_id.clone(),
        status: "connected".into(),
        error_code: None,
        error_message: None,
    });

    let mut buf = load_sync_buf(&root, &account_id);
    let mut failures = 0u32;
    loop {
        tokio::select! {
            _ = cancel.notified() => {
                save_session_status(&root, &account_id, "disconnected", None, None);
                let _ = tx.send(WechatEvent::SessionStatus {
                    account_id: account_id.clone(),
                    status: "disconnected".into(),
                    error_code: None,
                    error_message: None,
                });
                return;
            }
            r = api.get_updates(&base_url, &token, &buf, LONG_POLL_TIMEOUT.as_millis() as u64) => {
                match r {
                    Ok(resp) => {
                        if is_api_error(&resp) {
                            if is_session_expired_payload(&resp) {
                                let msg = resp.get("errmsg").and_then(|v| v.as_str()).unwrap_or("session expired").to_string();
                                save_session_status(&root, &account_id, "session_expired", resp.get("errcode").and_then(|v| v.as_i64()), Some(&msg));
                                let _ = tx.send(WechatEvent::SessionStatus {
                                    account_id: account_id.clone(),
                                    status: "session_expired".into(),
                                    error_code: resp.get("errcode").and_then(|v| v.as_i64()),
                                    error_message: Some(msg),
                                });
                                return;
                            }
                            failures += 1;
                            sleep_or_cancel(&cancel, retry_delay(failures)).await;
                            continue;
                        }
                        failures = 0;
                        if let Some(next) = resp.get("get_updates_buf").and_then(|v| v.as_str()) {
                            if next != buf {
                                buf = next.to_string();
                                save_sync_buf(&root, &account_id, &buf);
                            }
                        }
                        if let Some(msgs) = resp.get("msgs").and_then(|v| v.as_array()) {
                            for raw in msgs {
                                let from = raw.get("from_user_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                let to = raw.get("to_user_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                let ts = raw.get("create_time_ms").and_then(|v| v.as_u64()).unwrap_or(now_ms());
                                let ctx = raw.get("context_token").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                if !from.is_empty() && !ctx.is_empty() {
                                    set_context_token(&root, &account_id, &from, &ctx, raw.get("message_id").and_then(|v| v.as_str()));
                                }
                                let text = raw.get("item_list").and_then(extract_message_text);
                                let _ = tx.send(WechatEvent::Message {
                                    account_id: account_id.clone(),
                                    from,
                                    to,
                                    timestamp: ts,
                                    context_token: ctx,
                                    text,
                                });
                            }
                        }
                    }
                    Err(e) => {
                        failures += 1;
                        let _ = tx.send(WechatEvent::Error {
                            message: format!("getUpdates 失败: {e}"),
                            kind: "receiver".into(),
                        });
                        sleep_or_cancel(&cancel, retry_delay(failures)).await;
                    }
                }
            }
        }
    }
}

fn retry_delay(failures: u32) -> Duration {
    if failures >= MAX_CONSECUTIVE_FAILURES {
        BACKOFF_DELAY
    } else {
        RETRY_DELAY
    }
}

async fn sleep_or_cancel(cancel: &Notify, dur: Duration) {
    tokio::select! {
        _ = cancel.notified() => {}
        _ = tokio::time::sleep(dur) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockApi {
        // 简单固定响应
    }

    impl WechatApi for MockApi {
        fn get_qr_code(&self, _base: &str, _bot: &str) -> BoxFuture<'_, Result<Value, String>> {
            Box::pin(async { Ok(json!({ "qrcode": "QR-1", "qrcode_img_content": "http://qr/1" })) })
        }
        fn poll_qr_status(&self, _base: &str, _qrcode: &str) -> BoxFuture<'_, Result<Value, String>> {
            Box::pin(async {
                Ok(json!({
                    "status": "confirmed",
                    "ilink_bot_id": "bot-1@im.bot",
                    "bot_token": "tok-1",
                    "baseurl": "https://ilinkai.weixin.qq.com",
                    "ilink_user_id": "u-1@im.wechat",
                }))
            })
        }
        fn get_updates(&self, _base: &str, _tok: &str, _buf: &str, _t: u64) -> BoxFuture<'_, Result<Value, String>> {
            Box::pin(async { Ok(json!({ "ret": 0, "msgs": [], "get_updates_buf": "" })) })
        }
        fn send_message(&self, _base: &str, _tok: &str, _body: Value) -> BoxFuture<'_, Result<Value, String>> {
            Box::pin(async { Ok(json!({ "ret": 0 })) })
        }
        fn get_config(&self, _base: &str, _tok: &str, _body: Value) -> BoxFuture<'_, Result<Value, String>> {
            Box::pin(async { Ok(json!({ "ret": 0, "typing_ticket": "ticket-1" })) })
        }
        fn send_typing(&self, _base: &str, _tok: &str, _body: Value) -> BoxFuture<'_, Result<Value, String>> {
            Box::pin(async { Ok(json!({ "ret": 0 })) })
        }
    }

    #[test]
    fn expired_detection_narrowed() {
        assert!(is_session_expired_payload(&json!({ "errcode": -14 })));
        assert!(is_session_expired_payload(&json!({ "errmsg": "session expired" })));
        assert!(is_session_expired_payload(&json!({ "errmsg": "token expired" })));
        assert!(is_session_expired_payload(&json!({ "errmsg": "xxx session 已经 expired" })));
        // timeout 不再误判为过期
        assert!(!is_session_expired_payload(&json!({ "errmsg": "upstream timeout" })));
        assert!(!is_session_expired_payload(&json!({ "errmsg": "model not found" })));
    }

    #[test]
    fn message_text_extraction() {
        let list = json!([
            { "type": 2, "image_item": {} },
            { "type": 1, "text_item": { "text": "  hello  " } },
        ]);
        assert_eq!(extract_message_text(&list).as_deref(), Some("hello"));
        let media = json!([{ "type": 2, "image_item": {} }]);
        assert!(extract_message_text(&media).is_none());
        let voice = json!([{ "type": 3, "voice_item": { "text": "转写" } }]);
        assert_eq!(extract_message_text(&voice).as_deref(), Some("转写"));
    }

    #[test]
    fn normalize_and_ids() {
        assert_eq!(normalize_account_id("bot-1@im.bot"), "bot-1-im-bot");
        let a = generate_id("wechannel");
        let b = generate_id("wechannel");
        assert_ne!(a, b);
        assert!(!random_uin().is_empty());
    }

    #[tokio::test]
    async fn storage_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        save_account(root, "bot-1@im.bot", "tok", "https://ilinkai.weixin.qq.com", "u-1");
        assert_eq!(load_account_ids(root), vec!["bot-1@im.bot"]);
        assert!(load_account(root, "bot-1@im.bot").is_some());
        save_sync_buf(root, "bot-1@im.bot", "buf-abc");
        assert_eq!(load_sync_buf(root, "bot-1@im.bot"), "buf-abc");
        set_context_token(root, "bot-1@im.bot", "u-1", "ctx-1", None);
        assert_eq!(get_context_token(root, "bot-1@im.bot", "u-1").as_deref(), Some("ctx-1"));
        // 过期窗口：写入过期时间后取不到
        let p = reply_context_path(root, "bot-1@im.bot");
        let mut map = load_reply_context(root, "bot-1@im.bot");
        map.insert(
            "u-2".into(),
            json!({ "peerId": "u-2", "contextToken": "old", "lastInboundAt": 1, "expiresAt": 1 }),
        );
        let obj = map.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<Value>();
        write_json(&p, &obj).unwrap();
        assert!(get_context_token(root, "bot-1@im.bot", "u-2").is_none());
        // 快照
        let snap = accounts_snapshot(root);
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0]["configured"], json!(true));
        assert_eq!(snap[0]["userId"], "u-1");
    }

    #[tokio::test]
    async fn receiver_reports_session_expired_and_stops() {
        struct ExpiredApi;
        impl WechatApi for ExpiredApi {
            fn get_qr_code(&self, _b: &str, _t: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "qrcode": "q", "qrcode_img_content": "u" })) })
            }
            fn poll_qr_status(&self, _b: &str, _q: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "status": "wait" })) })
            }
            fn get_updates(&self, _b: &str, _t: &str, _buf: &str, _to: u64) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "errcode": -14, "errmsg": "session expired" })) })
            }
            fn send_message(&self, _b: &str, _t: &str, _body: Value) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
            fn get_config(&self, _b: &str, _t: &str, _body: Value) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0, "typing_ticket": "ticket-x" })) })
            }
            fn send_typing(&self, _b: &str, _t: &str, _body: Value) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        save_account(&root, "bot-1", "tok", DEFAULT_BASE_URL, "u-1");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let cancel = Arc::new(Notify::new());
        let api = Arc::new(ExpiredApi);
        run_receiver(api, tx, root.clone(), "bot-1".into(), cancel).await;
        let mut expired = false;
        while let Ok(ev) = rx.try_recv() {
            if let WechatEvent::SessionStatus { status, .. } = ev {
                if status == "session_expired" {
                    expired = true;
                }
            }
        }
        assert!(expired);
        assert_eq!(
            load_session_status(&root, "bot-1").get("status").and_then(|v| v.as_str()),
            Some("session_expired")
        );
    }

    #[tokio::test]
    async fn login_flow_emits_qr_and_success() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let client = WechatClient::with_api(root, tx, MockApi {});
        client.start_login("L1".into(), Arc::new(Notify::new()));
        let mut qr_seen = false;
        let mut ok = false;
        let mut acc = None;
        while let Some(ev) = rx.recv().await {
            match ev {
                WechatEvent::Qr(_) => qr_seen = true,
                WechatEvent::LoginResult { success, account_id, .. } => {
                    ok = success;
                    acc = account_id;
                    break;
                }
                _ => {}
            }
        }
        assert!(qr_seen);
        assert!(ok);
        assert_eq!(acc.as_deref(), Some("bot-1@im.bot"));
    }

    #[tokio::test]
    async fn send_typing_fetches_ticket_once_then_reuses_and_skips_when_disconnected() {
        struct CountingApi {
            get_config_calls: Arc<std::sync::Mutex<usize>>,
            send_typing_calls: Arc<std::sync::Mutex<usize>>,
        }
        impl WechatApi for CountingApi {
            fn get_qr_code(&self, _b: &str, _t: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn poll_qr_status(&self, _b: &str, _q: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "status": "wait" })) })
            }
            fn get_updates(&self, _b: &str, _t: &str, _buf: &str, _to: u64) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0, "msgs": [], "get_updates_buf": "" })) })
            }
            fn send_message(&self, _b: &str, _t: &str, _body: Value) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
            fn get_config(&self, _b: &str, _t: &str, _body: Value) -> BoxFuture<'_, Result<Value, String>> {
                *self.get_config_calls.lock().unwrap() += 1;
                Box::pin(async { Ok(json!({ "ret": 0, "typing_ticket": "ticket-1" })) })
            }
            fn send_typing(&self, _b: &str, _t: &str, _body: Value) -> BoxFuture<'_, Result<Value, String>> {
                *self.send_typing_calls.lock().unwrap() += 1;
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        save_account(&root, "bot-1", "tok", DEFAULT_BASE_URL, "u-1");
        save_session_status(&root, "bot-1", "connected", None, None);
        set_context_token(&root, "bot-1", "u-1", "ctx-1", None);
        let gcc = Arc::new(std::sync::Mutex::new(0usize));
        let stc = Arc::new(std::sync::Mutex::new(0usize));
        let (tx, _rx) = mpsc::unbounded_channel();
        let client = WechatClient::with_api(
            root.clone(),
            tx,
            CountingApi { get_config_calls: gcc.clone(), send_typing_calls: stc.clone() },
        );
        client.send_typing("bot-1", "u-1", 1).await.unwrap();
        client.send_typing("bot-1", "u-1", 2).await.unwrap();
        assert_eq!(*gcc.lock().unwrap(), 1); // ticket 只取一次
        assert_eq!(*stc.lock().unwrap(), 2);

        // 未连接账号：直接 Ok，不再发请求
        save_session_status(&root, "bot-1", "disconnected", None, None);
        client.send_typing("bot-1", "u-1", 1).await.unwrap();
        assert_eq!(*stc.lock().unwrap(), 2); // 未新增请求
    }
}
