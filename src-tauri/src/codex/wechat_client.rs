//! 微信 ilink bot 协议客户端（纯 Rust，替代 Node sidecar）。
//!
//! 实现二维码登录、getUpdates 长轮询、文本与媒体消息收发、contextToken 管理，以及
//! 账号/会话/同步缓冲/回复上下文的文件存储；图片、文件和视频按微信 CDN 协议流式
//! 下载并 AES-128-ECB 解密，出站媒体加密后上传。
//! 事件经 mpsc 推送给桥（WechatEvent），语义对齐原 sidecar 的 stdio 事件。

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch, Mutex, Notify};

/// 默认 API 基地址（与 wechat-channel 一致）。
pub const DEFAULT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
/// 媒体 CDN 基地址（与 wechat-channel 一致）：下载地址为 `{CDN}/download?encrypted_query_param=…`。
pub const CDN_BASE_URL: &str = "https://novac2c.cdn.weixin.qq.com/c2c";
/// ilink bot 类型。
const BOT_TYPE: &str = "3";
/// 微信接入协议名（「关于」展示用）：走微信官方 ClawBot 通道的 ilink bot 接口。
pub const PROTOCOL_NAME: &str = "ilink bot API";
/// base_info 里的通道版本（沿用库版本串，保持兼容）。
pub const CHANNEL_VERSION: &str = "1.1.0";
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
/// 入站附件下载总时长上限（不限制大小；超时按接收失败处理，不拖住长轮询）。
const ATTACHMENT_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// 单条消息最多处理的各类附件数量（微信通常单附件，多余忽略）。
const MAX_IMAGES_PER_MESSAGE: usize = 4;
const MAX_FILES_PER_MESSAGE: usize = 4;
const MAX_VIDEOS_PER_MESSAGE: usize = 2;
/// 附件存放目录名（位于 wechannel-data 下，与 wechat-channel 的原布局一致）。
const MEDIA_DIR_NAME: &str = "media";

/// 出站单个文件的明文大小上限（保护性阈值，非平台限制）：超限直接拒绝并回提示，
/// 避免误发超大文件长时间占用。加密与上传均为流式，内存占用与文件大小无关。
const MAX_OUTBOUND_BYTES: u64 = 200 * 1024 * 1024;
/// CDN 上传的可重试次数（5xx / 网络错误；4xx 视为致命不重试）。
const UPLOAD_RETRY_ATTEMPTS: u32 = 3;
/// CDN 上传单次尝试的超时。
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// 入站 `message_id` 去重环上限（进程内）：作为 sync-buf 之外的第二道防线，
/// 覆盖 `save_sync_buf` 写盘失败后平台按空 buf 重放队列（会重复执行回合）的场景。
const INBOUND_DEDUP_CAPACITY: usize = 500;
/// 附件文件名净化后的最大字符数（含扩展名）。
const MAX_FILE_NAME_CHARS: usize = 120;
/// 落盘时保留的解密头部字节数（供图片魔数探测）。
const HEAD_PROBE_BYTES: usize = 64;

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
    s.find(a).is_some_and(|i| s[i + a.len()..].contains(b))
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

/// 入站消息 id：协议声明为数字（`message_id?: number`），实际需兼容字符串形态。
///
/// 此前的实现按字符串读取（`as_str()`），导致数字型 id 一律读不到（返回 None）——
/// 依赖它的入站去重环因此在真实流量上从未生效。数字统一转十进制字符串后返回。
fn message_id_of(message: &Value) -> Option<String> {
    match message.get("message_id") {
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

/// 展开「候选 item」：每个 item 之后追加其 `ref_msg.message_item`（**仅一层，不递归**）。
///
/// 用户引用一条含附件（图片/文件/视频）的消息转发时，附件挂在这层嵌套里；
/// 只解一层既覆盖该场景，又避免递归展开带来的配额混乱与无界遍历。
/// 文本提取**不走**本函数（被引用消息的正文是上下文，不是本轮指令）。
fn candidate_items(item_list: Option<&Value>) -> Vec<&Value> {
    let Some(list) = item_list.and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(list.len());
    for item in list {
        out.push(item);
        if let Some(nested) = item.pointer("/ref_msg/message_item") {
            out.push(nested);
        }
    }
    out
}

/// 入站图片引用：CDN 下载参数 + AES 密钥原文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundImage {
    /// 原图 CDN 引用（`image_item.media.encrypt_query_param`）。
    pub encrypt_query_param: String,
    /// AES-128 密钥原文：优先 `image_item.aeskey`（hex），其次 `media.aes_key`（base64）；为空视为明文。
    pub aes_key: String,
}

/// 从消息 item_list 提取入站图片：IMAGE（type=2）且带 CDN 下载参数，最多 `MAX_IMAGES_PER_MESSAGE` 张。
pub fn extract_message_images(item_list: &Value) -> Vec<InboundImage> {
    let mut out = Vec::new();
    for item in candidate_items(Some(item_list)) {
        if out.len() >= MAX_IMAGES_PER_MESSAGE {
            break;
        }
        if item.get("type").and_then(|v| v.as_i64()) != Some(2) {
            continue;
        }
        let Some(param) = item
            .pointer("/image_item/media/encrypt_query_param")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let key = item
            .pointer("/image_item/aeskey")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                item.pointer("/image_item/media/aes_key")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or("");
        out.push(InboundImage {
            encrypt_query_param: param.to_string(),
            aes_key: key.to_string(),
        });
    }
    out
}

/// 入站文件/视频引用：CDN 下载参数 + AES 密钥原文 + 原始文件名（视频没有文件名）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundFile {
    /// 附件 CDN 引用（`{file_item|video_item}/media/encrypt_query_param`）。
    pub encrypt_query_param: String,
    /// AES-128 密钥原文（`media.aes_key`，base64）；为空视为明文。
    pub aes_key: String,
    /// 原始文件名（仅文件消息有；视频固定用生成名）。
    pub file_name: Option<String>,
}

/// 按 item 类型提取附件引用（type=4 文件 / type=5 视频），最多 `max` 个。
fn extract_media_refs(
    item_list: &Value,
    item_type: i64,
    item_key: &str,
    max: usize,
) -> Vec<InboundFile> {
    let mut out = Vec::new();
    for item in candidate_items(Some(item_list)) {
        if out.len() >= max {
            break;
        }
        if item.get("type").and_then(|v| v.as_i64()) != Some(item_type) {
            continue;
        }
        let Some(param) = item
            .pointer(&format!("/{item_key}/media/encrypt_query_param"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let aes_key = item
            .pointer(&format!("/{item_key}/media/aes_key"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("");
        let file_name = item
            .pointer(&format!("/{item_key}/file_name"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        out.push(InboundFile {
            encrypt_query_param: param.to_string(),
            aes_key: aes_key.to_string(),
            file_name,
        });
    }
    out
}

/// 从消息 item_list 提取入站文件（type=4），最多 `MAX_FILES_PER_MESSAGE` 个。
pub fn extract_message_files(item_list: &Value) -> Vec<InboundFile> {
    extract_media_refs(item_list, 4, "file_item", MAX_FILES_PER_MESSAGE)
}

/// 从消息 item_list 提取入站视频（type=5），最多 `MAX_VIDEOS_PER_MESSAGE` 个。
pub fn extract_message_videos(item_list: &Value) -> Vec<InboundFile> {
    extract_media_refs(item_list, 5, "video_item", MAX_VIDEOS_PER_MESSAGE)
}

/// 图片 CDN 下载地址（与 wechat-channel 的 buildCdnDownloadUrl 一致）。
pub fn cdn_download_url(encrypt_query_param: &str) -> String {
    format!(
        "{CDN_BASE_URL}/download?encrypted_query_param={}",
        urlencode(encrypt_query_param)
    )
}

/// 媒体上传地址（与 wechat-channel 的 buildCdnUploadUrl 一致）。
pub fn cdn_upload_url(upload_param: &str, filekey: &str) -> String {
    format!(
        "{CDN_BASE_URL}/upload?encrypted_query_param={}&filekey={}",
        urlencode(upload_param),
        urlencode(filekey)
    )
}

/// 组装出站媒体消息 item（image / video / file 三种形状）。
///
/// `aes_key` 在 JSON 里是 **base64(32 字符 hex 串)**，与官方实现一致，也是微信端唯一认得
/// 的形态；填 base64(16 字节密钥原文) 会让接收端下载停在「下载中 0%」或下完仍是密文
/// （2026-09-24 真机 A/B 实测：只有改成 base64(hex) 后接收端才能下载并还原明文）。
/// 大小字段按参考实现：图片用密文大小、视频用密文大小、文件用**明文**大小的字符串。
pub fn build_media_item(
    kind: OutboundMediaKind,
    download_param: &str,
    aes_key: &[u8; 16],
    cipher_size: u64,
    raw_size: u64,
    file_name: &str,
) -> Value {
    let media = json!({
        "encrypt_query_param": download_param,
        "aes_key": BASE64.encode(hex_lower(aes_key).as_bytes()),
        "encrypt_type": 1,
    });
    let key = kind.item_key();
    let mut item = json!({ "type": kind.item_type() });
    let mut detail = json!({ "media": media });
    match kind {
        OutboundMediaKind::Image => detail["mid_size"] = json!(cipher_size),
        OutboundMediaKind::Video => detail["video_size"] = json!(cipher_size),
        OutboundMediaKind::File => {
            detail["file_name"] = json!(file_name);
            detail["len"] = json!(raw_size.to_string());
        }
    }
    item[key] = detail;
    item
}

/// item_list 中可下载图片项的数量（与 `extract_message_images` 的口径一致，用于判断是否溢出上限）。
fn count_image_items(item_list: Option<&Value>) -> usize {
    candidate_items(item_list)
        .into_iter()
        .filter(|i| {
            i.get("type").and_then(|v| v.as_i64()) == Some(2)
                && i.pointer("/image_item/media/encrypt_query_param")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| !s.trim().is_empty())
        })
        .count()
}

/// 解析 AES-128 密钥：支持 32 字符 hex、base64(16 字节原文)、base64(32 字符 hex 串) 三种形态。
pub fn parse_aes_key(raw: &str) -> Result<[u8; 16], String> {
    let s = raw.trim();
    if s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return hex_to_key(s);
    }
    let decoded = BASE64
        .decode(s)
        .map_err(|e| format!("aes_key base64 解码失败: {e}"))?;
    if decoded.len() == 16 {
        let mut key = [0u8; 16];
        key.copy_from_slice(&decoded);
        return Ok(key);
    }
    if decoded.len() == 32 {
        let ascii = std::str::from_utf8(&decoded)
            .map_err(|_| "aes_key 非法：既不是 16 字节密钥也不是 hex 串".to_string())?;
        return hex_to_key(ascii);
    }
    Err(format!("aes_key 长度非法（{} 字节）", decoded.len()))
}

/// 32 字符 hex → 16 字节密钥。
fn hex_to_key(s: &str) -> Result<[u8; 16], String> {
    let bytes = s.as_bytes();
    if bytes.len() != 32 {
        return Err("aes_key hex 长度不是 32".into());
    }
    let mut key = [0u8; 16];
    for (i, slot) in key.iter_mut().enumerate() {
        let hi = (bytes[i * 2] as char)
            .to_digit(16)
            .ok_or_else(|| "aes_key 含非 hex 字符".to_string())?;
        let lo = (bytes[i * 2 + 1] as char)
            .to_digit(16)
            .ok_or_else(|| "aes_key 含非 hex 字符".to_string())?;
        *slot = (hi * 16 + lo) as u8;
    }
    Ok(key)
}

/// 图片扩展名：按魔数识别，仅接受 codex 能直接读入的常见格式。
fn image_extension_of(bytes: &[u8]) -> Option<&'static str> {
    match image::guess_format(bytes).ok()? {
        image::ImageFormat::Png => Some("png"),
        image::ImageFormat::Jpeg => Some("jpg"),
        image::ImageFormat::Gif => Some("gif"),
        image::ImageFormat::WebP => Some("webp"),
        image::ImageFormat::Bmp => Some("bmp"),
        _ => None,
    }
}

/// 入站图片文件名：`wechat-{yyyyMMdd-HHmmss}-{seq}.{ext}`（同秒多图靠进程内自增区分）。
fn wechat_image_file_name(ext: &str, now: chrono::DateTime<chrono::Local>) -> String {
    let seq = ID_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("wechat-{}-{seq}.{ext}", now.format("%Y%m%d-%H%M%S"))
}

/// 会话目录名净化：只保留字母数字与 `._-`，其余替换为 `_`；空值或纯点兜底 `_unknown`。
pub fn sanitize_path_segment(id: &str) -> String {
    let cleaned: String = id
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        return "_unknown".to_string();
    }
    cleaned
}

/// 附件目录：`<root>/wechannel-data/media/<会话id>`（扁平，图片/文件/视频混放）。
fn media_dir(root: &Path, thread_id: &str) -> PathBuf {
    state_dir(root)
        .join(MEDIA_DIR_NAME)
        .join(sanitize_path_segment(thread_id))
}

/// 目录内唯一文件名：已存在则给主名追加 `-<seq>`（`seq` 为进程内自增）。
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    if !dir.join(name).exists() {
        return dir.join(name);
    }
    let (stem, ext) = split_name(name);
    let seq = ID_SEQ.fetch_add(1, Ordering::Relaxed);
    if ext.is_empty() {
        dir.join(format!("{stem}-{seq}"))
    } else {
        dir.join(format!("{stem}-{seq}.{ext}"))
    }
}

/// 拆分主名与扩展名（按最后一个点；点开头或无点视为无扩展名）。
fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => (&name[..i], &name[i + 1..]),
        _ => (name, ""),
    }
}

/// 按字符数限长，超长时截断主名、保留扩展名。
fn truncate_file_name(name: &str, max_chars: usize) -> String {
    if name.chars().count() <= max_chars {
        return name.to_string();
    }
    let (stem, ext) = split_name(name);
    let keep = if ext.is_empty() {
        max_chars
    } else {
        max_chars.saturating_sub(ext.chars().count() + 1)
    };
    let stem: String = stem.chars().take(keep.max(1)).collect();
    if ext.is_empty() {
        stem
    } else {
        format!("{stem}.{ext}")
    }
}

/// 文件名净化：剥离路径部分、替换 Windows 非法字符与控制字符、去首尾空白与点、限长（保留扩展名）。
pub fn sanitize_file_name(hint: &str) -> String {
    let base = hint.rsplit(['\\', '/']).next().unwrap_or(hint).trim();
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches(|c: char| c == ' ' || c == '.');
    if trimmed.is_empty() {
        return "attachment".to_string();
    }
    truncate_file_name(trimmed, MAX_FILE_NAME_CHARS)
}

/// 入站视频文件名：协议没有原始文件名，固定 `wechat-video-{yyyyMMdd-HHmmss}-{seq}.mp4`。
fn wechat_video_file_name(now: chrono::DateTime<chrono::Local>) -> String {
    let seq = ID_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("wechat-video-{}-{seq}.mp4", now.format("%Y%m%d-%H%M%S"))
}

/// 入站附件的类别：决定最终命名策略与日志标签。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttachmentKind {
    Image,
    File,
    Video,
}

impl AttachmentKind {
    fn label(self) -> &'static str {
        match self {
            AttachmentKind::Image => "图片",
            AttachmentKind::File => "文件",
            AttachmentKind::Video => "视频",
        }
    }
}

/// 附件最终文件名：图片按 `head` 魔数推断扩展名，文件用净化后的原名，视频用生成名。
fn resolve_final_name(
    kind: AttachmentKind,
    head: &[u8],
    name_hint: Option<&str>,
    now: chrono::DateTime<chrono::Local>,
) -> Result<String, String> {
    match kind {
        AttachmentKind::Image => {
            let ext = image_extension_of(head)
                .ok_or_else(|| "不支持的图片格式（仅支持 png/jpg/gif/webp/bmp）".to_string())?;
            Ok(wechat_image_file_name(ext, now))
        }
        AttachmentKind::File => Ok(name_hint
            .map(sanitize_file_name)
            .unwrap_or_else(|| "attachment".to_string())),
        AttachmentKind::Video => Ok(wechat_video_file_name(now)),
    }
}

/// 流式解密状态机（AES-128-ECB + PKCS7）：只解出"确定不是最后一块"的完整块，
/// 末块留到 `finish` 再解密并去 padding；无密钥时直通。
struct EcbStreamDecryptor {
    key: Option<[u8; 16]>,
    /// 未满一块、或需等"是否还有后续数据"的尾部字节。
    pending: Vec<u8>,
}

impl EcbStreamDecryptor {
    /// 按 `aes_key` 构造；密钥为空表示明文直通。
    fn new(aes_key: &str) -> Result<Self, String> {
        let key = if aes_key.trim().is_empty() {
            None
        } else {
            Some(parse_aes_key(aes_key)?)
        };
        Ok(Self {
            key,
            pending: Vec::new(),
        })
    }

    /// 喂入一块密文，返回可立即写出的明文（可能为空）。
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<u8>, String> {
        let Some(key) = self.key else {
            return Ok(chunk.to_vec());
        };
        self.pending.extend_from_slice(chunk);
        // 保留最后一块：它可能带着 PKCS7 padding，需等流结束后再处理。
        let blocks = self.pending.len() / 16;
        if blocks <= 1 {
            return Ok(Vec::new());
        }
        let take = (blocks - 1) * 16;
        let mut buf: Vec<u8> = self.pending.drain(..take).collect();
        decrypt_blocks(&mut buf, &key);
        Ok(buf)
    }

    /// 流结束：解密末块并去掉 PKCS7 padding。
    fn finish(&mut self) -> Result<Vec<u8>, String> {
        let Some(key) = self.key else {
            return Ok(std::mem::take(&mut self.pending));
        };
        if self.pending.is_empty() {
            return Err("附件密文为空".into());
        }
        if !self.pending.len().is_multiple_of(16) {
            return Err(format!(
                "附件密文长度不是 16 的整数倍（{} 字节）",
                self.pending.len()
            ));
        }
        use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyInit};
        let mut buf = std::mem::take(&mut self.pending);
        ecb::Decryptor::<aes::Aes128>::new((&key).into())
            .decrypt_padded_mut::<Pkcs7>(&mut buf)
            .map(|plain| plain.to_vec())
            .map_err(|e| format!("附件解密失败: {e}"))
    }
}

/// 就地解密若干完整块（不做 padding 处理；末块由 `EcbStreamDecryptor::finish` 负责）。
fn decrypt_blocks(buf: &mut [u8], key: &[u8; 16]) {
    use aes::cipher::{generic_array::GenericArray, BlockDecrypt, KeyInit};
    let cipher = aes::Aes128::new(key.into());
    for block in buf.chunks_exact_mut(16) {
        cipher.decrypt_block(GenericArray::from_mut_slice(block));
    }
}

/// 就地加密若干完整块（ECB 无状态；PKCS7 padding 由调用方在末块补齐）。
fn encrypt_blocks(buf: &mut [u8], key: &[u8; 16]) {
    use aes::cipher::{generic_array::GenericArray, BlockEncrypt, KeyInit};
    let cipher = aes::Aes128::new(key.into());
    for block in buf.chunks_exact_mut(16) {
        cipher.encrypt_block(GenericArray::from_mut_slice(block));
    }
}

/// 出站媒体类型（对应协议 `UploadMediaType`：IMAGE=1 / VIDEO=2 / FILE=3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboundMediaKind {
    Image,
    Video,
    File,
}

impl OutboundMediaKind {
    /// `getuploadurl` 的 `media_type`
    fn upload_media_type(self) -> i64 {
        match self {
            Self::Image => 1,
            Self::Video => 2,
            Self::File => 3,
        }
    }

    /// 发送消息的 item 类型（与入站一致：图片 2 / 视频 5 / 文件 4）
    fn item_type(self) -> i64 {
        match self {
            Self::Image => 2,
            Self::Video => 5,
            Self::File => 4,
        }
    }

    /// item 内的字段名
    fn item_key(self) -> &'static str {
        match self {
            Self::Image => "image_item",
            Self::Video => "video_item",
            Self::File => "file_item",
        }
    }
}

/// 按扩展名判定出站媒体类型（大小写不敏感；未识别一律按文件发送）。
pub fn outbound_media_kind(path: &str) -> OutboundMediaKind {
    let ext = path
        .rsplit('.')
        .next()
        .filter(|e| !e.contains(['/', '\\']))
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" => OutboundMediaKind::Image,
        "mp4" | "mov" | "avi" | "mkv" | "webm" => OutboundMediaKind::Video,
        _ => OutboundMediaKind::File,
    }
}

/// PKCS7 填充后的密文大小：`((raw / 16) + 1) * 16`。
fn padded_size(raw: u64) -> u64 {
    (raw / 16 + 1) * 16
}

/// 小写 hex 串：出站 `getuploadurl` 的 `aeskey` 字段与消息项 `aes_key` 载荷共用同一形态。
fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 随机 AES-128 密钥（操作系统熵源）。
fn random_key() -> Result<[u8; 16], String> {
    let mut key = [0u8; 16];
    getrandom::fill(&mut key).map_err(|e| format!("生成随机密钥失败: {e}"))?;
    Ok(key)
}

/// 随机 filekey：16 字节 hex（与参考实现一致）。
fn random_filekey() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| format!("生成 filekey 失败: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// 流式读取源文件：同时算明文 MD5、明文大小，并把 AES-128-ECB + PKCS7 密文写入 `dst`。
/// 返回 `(明文大小, 密文大小, 明文 md5 hex)`；内存占用与文件大小无关。
///
/// ECB 按块加密，故可流式推进：始终保留末尾不足一块（≤16 字节）的数据，
/// EOF 时再补 PKCS7 并加密最后一（或两）块。
async fn encrypt_file_to(
    src: &Path,
    dst: &Path,
    key: &[u8; 16],
) -> Result<(u64, u64, String), String> {
    use crate::codex::path_util::clean_path;
    use md5::Digest;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut input = tokio::fs::File::open(src)
        .await
        .map_err(|e| format!("打开待发送文件失败 {}: {e}", clean_path(src)))?;
    let mut output = tokio::fs::File::create(dst)
        .await
        .map_err(|e| format!("创建上传临时文件失败 {}: {e}", clean_path(dst)))?;

    let mut hasher = md5::Md5::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut pending: Vec<u8> = Vec::with_capacity(64 * 1024 + 16);
    let mut raw: u64 = 0;
    loop {
        let n = input
            .read(&mut buf)
            .await
            .map_err(|e| format!("读取待发送文件失败 {}: {e}", clean_path(src)))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        raw += n as u64;
        pending.extend_from_slice(&buf[..n]);
        // 至少保留 1 字节，保证 EOF 时仍有数据可做 PKCS7 填充
        let blocks = (pending.len().saturating_sub(1)) / 16;
        if blocks > 0 {
            let take = blocks * 16;
            encrypt_blocks(&mut pending[..take], key);
            output
                .write_all(&pending[..take])
                .await
                .map_err(|e| format!("写入上传临时文件失败: {e}"))?;
            pending.drain(..take);
        }
    }
    // PKCS7：补 1..=16 字节，使总长为 16 的整数倍（空文件也补满一整块）
    let pad = 16 - (pending.len() % 16);
    pending.extend(std::iter::repeat_n(pad as u8, pad));
    encrypt_blocks(&mut pending, key);
    output
        .write_all(&pending)
        .await
        .map_err(|e| format!("写入上传临时文件失败: {e}"))?;
    output
        .flush()
        .await
        .map_err(|e| format!("写入上传临时文件失败: {e}"))?;

    let cipher_size = padded_size(raw);
    let md5_hex: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok((raw, cipher_size, md5_hex))
}

/// 流式下载 + 解密写入 `.part`，返回已解密数据的前 `HEAD_PROBE_BYTES` 字节（供图片格式探测）。
async fn write_stream_to_part<A: WechatApi>(
    api: &A,
    url: &str,
    aes_key: &str,
    part: &Path,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    let mut stream = api.download_stream(url, timeout).await?;
    let mut decryptor = EcbStreamDecryptor::new(aes_key)?;
    let mut file = tokio::fs::File::create(part)
        .await
        .map_err(|e| format!("创建附件临时文件失败 {}: {e}", part.display()))?;
    let mut head: Vec<u8> = Vec::new();
    let mut write_out = |plain: &[u8]| {
        if head.len() < HEAD_PROBE_BYTES {
            let need = HEAD_PROBE_BYTES - head.len();
            head.extend_from_slice(&plain[..need.min(plain.len())]);
        }
    };
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        let plain = decryptor.push(&chunk)?;
        if plain.is_empty() {
            continue;
        }
        write_out(&plain);
        file.write_all(&plain)
            .await
            .map_err(|e| format!("写入附件失败 {}: {e}", part.display()))?;
    }
    let tail = decryptor.finish()?;
    if !tail.is_empty() {
        write_out(&tail);
        file.write_all(&tail)
            .await
            .map_err(|e| format!("写入附件失败 {}: {e}", part.display()))?;
    }
    file.flush()
        .await
        .map_err(|e| format!("写入附件失败 {}: {e}", part.display()))?;
    Ok(head)
}

/// 流式下载 + 解密 + 落盘到 `<root>/wechannel-data/media/<会话id>/`：
/// 先写同目录 `.part`，定名后改名；任何失败/超时都删除临时文件，不留半截文件。
/// 不限制附件大小，`timeout` 为下载总时长上限（测试可注入小值）。
#[allow(clippy::too_many_arguments)]
async fn save_attachment<A: WechatApi>(
    api: &A,
    root: &Path,
    thread_id: &str,
    url: &str,
    aes_key: &str,
    kind: AttachmentKind,
    name_hint: Option<&str>,
    timeout: Duration,
) -> Result<String, String> {
    use crate::codex::path_util::clean_path;

    let dir = media_dir(root, thread_id);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("创建附件目录失败 {}: {e}", dir.display()))?;
    let part = dir.join(format!(
        ".wechat-{}-{}.part",
        std::process::id(),
        ID_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    // 超时由本层强制：mock/异常上游不遵守请求超时时也不会挂住长轮询。
    let head = match tokio::time::timeout(
        timeout,
        write_stream_to_part(api, url, aes_key, &part, timeout),
    )
    .await
    {
        Ok(Ok(head)) => head,
        Ok(Err(e)) => {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(e);
        }
        Err(_) => {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(format!("附件下载超时（超过 {} 秒）", timeout.as_secs()));
        }
    };
    let name = match resolve_final_name(kind, &head, name_hint, chrono::Local::now()) {
        Ok(name) => name,
        Err(e) => {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(e);
        }
    };
    let target = unique_path(&dir, &name);
    if let Err(e) = tokio::fs::rename(&part, &target).await {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(format!("附件改名失败 {}: {e}", target.display()));
    }
    Ok(clean_path(&target))
}

/// 下载并解密单个入站附件（图片/文件/视频），落盘到会话目录后返回绝对路径。
async fn fetch_inbound_attachment<A: WechatApi>(
    api: &A,
    root: &Path,
    thread_id: &str,
    encrypt_query_param: &str,
    aes_key: &str,
    kind: AttachmentKind,
    name_hint: Option<&str>,
) -> Result<String, String> {
    save_attachment(
        api,
        root,
        thread_id,
        &cdn_download_url(encrypt_query_param),
        aes_key,
        kind,
        name_hint,
        ATTACHMENT_DOWNLOAD_TIMEOUT,
    )
    .await
}

/// 附件接收失败：首个失败原因留在消息事件里（供桥回提示），每次都推一条日志事件。
fn report_media_error(
    tx: &mpsc::UnboundedSender<WechatEvent>,
    first: &mut Option<String>,
    kind: AttachmentKind,
    err: String,
) {
    if first.is_none() {
        *first = Some(err.clone());
    }
    let _ = tx.send(WechatEvent::Error {
        message: format!("微信{}接收失败: {err}", kind.label()),
        kind: "media".into(),
    });
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
    let _ = write_json(
        &sync_buf_path(root, account_id),
        &json!({ "get_updates_buf": buf }),
    );
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
    // 写入前剪掉已过期的对端记录：被动窗口（24h）外的条目对回复没有意义，
    // 不清理会随着历史对端数量持续累积（整表每次重写的体积也随之增长）。
    let now = now_ms();
    let obj = map
        .iter()
        .filter(|(_, v)| {
            v.get("expiresAt")
                .and_then(|e| e.as_u64())
                .is_none_or(|expires_at| expires_at > now)
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect::<Value>();
    let _ = write_json(&reply_context_path(root, account_id), &obj);
}

/// 保存入站消息的 contextToken（24h 被动回复窗口）。
fn set_context_token(
    root: &Path,
    account_id: &str,
    user_id: &str,
    token: &str,
    message_id: Option<&str>,
) {
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
    entry
        .get("contextToken")
        .and_then(|v| v.as_str())
        .map(str::to_string)
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

/// 出站发送失败的可恢复性：桥据此决定是否值得重试（纯分类，便于单测）。
///
/// `Fatal` 是确定性失败（会话过期、账号未连接、缺少 24 小时被动窗口上下文）——
/// 重试只会立刻拿到同样的错误，且会拖长整条回复的发送时间；
/// `Retryable` 是瞬时失败（网络错误、超时、上游非致命错误）——短重试有机会补齐缺失分片。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WechatSendFailure {
    Fatal(String),
    Retryable(String),
}

impl WechatSendFailure {
    /// 是否值得重试。
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Retryable(_))
    }

    /// 失败原因文本（用于日志与提示）。
    pub fn message(&self) -> &str {
        match self {
            Self::Fatal(m) | Self::Retryable(m) => m,
        }
    }
}

/// CDN 上传**单次尝试**的结果分类（是否重试由调用方决定，便于单测）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CdnUploadOutcome {
    /// 上传成功：携带响应头 `x-encrypted-param`（媒体项要用的 `encrypt_query_param`）。
    Ok(String),
    /// 4xx 客户端错误：重试无意义
    Fatal(String),
    /// 5xx / 网络错误 / 缺响应头：可重试
    Retryable(String),
}

/// 附件下载流：按块产出原始字节（`Err` 表示读取中断）；解密与落盘在调用方流式完成。
pub type AttachmentStream =
    Pin<Box<dyn futures_util::Stream<Item = Result<Vec<u8>, String>> + Send>>;

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
    fn send_message(
        &self,
        base_url: &str,
        token: &str,
        body: Value,
    ) -> BoxFuture<'_, Result<Value, String>>;
    fn get_config(
        &self,
        base_url: &str,
        token: &str,
        body: Value,
    ) -> BoxFuture<'_, Result<Value, String>>;
    fn send_typing(
        &self,
        base_url: &str,
        token: &str,
        body: Value,
    ) -> BoxFuture<'_, Result<Value, String>>;
    /// 申请媒体上传地址（`ilink/bot/getuploadurl`）；成功返回含 `upload_param` 的响应体。
    fn get_upload_url(
        &self,
        base_url: &str,
        token: &str,
        body: Value,
    ) -> BoxFuture<'_, Result<Value, String>>;
    /// 把密文文件 POST 到微信 CDN（`Content-Type: application/octet-stream`）。
    /// 成功时从响应头 `x-encrypted-param` 取媒体项的 `encrypt_query_param`（与参考实现一致）；
    /// 失败按 [`CdnUploadOutcome`] 分类。
    fn upload_to_cdn(
        &self,
        url: &str,
        ciphertext_path: PathBuf,
        len: u64,
        timeout: Duration,
    ) -> BoxFuture<'_, CdnUploadOutcome>;
    /// 打开附件下载流（入站附件走微信 CDN）：按块产出原始字节，解密由调用方流式完成。
    /// `timeout` 为整次请求（含读取响应体）的时长上限。
    fn download_stream(
        &self,
        url: &str,
        timeout: Duration,
    ) -> BoxFuture<'_, Result<AttachmentStream, String>>;
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
    fn get_qr_code(&self, base_url: &str, bot_type: &str) -> BoxFuture<'_, Result<Value, String>> {
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

    fn poll_qr_status(&self, base_url: &str, qrcode: &str) -> BoxFuture<'_, Result<Value, String>> {
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

    fn get_upload_url(
        &self,
        base_url: &str,
        token: &str,
        body: Value,
    ) -> BoxFuture<'_, Result<Value, String>> {
        self.post_json(
            base_url,
            "ilink/bot/getuploadurl",
            Some(token),
            body,
            Duration::from_secs(15),
        )
    }

    fn upload_to_cdn(
        &self,
        url: &str,
        ciphertext_path: PathBuf,
        len: u64,
        timeout: Duration,
    ) -> BoxFuture<'_, CdnUploadOutcome> {
        let http = self.http.clone();
        let url = url.to_string();
        Box::pin(async move {
            // 用文件作为请求体：reqwest 的 `From<tokio::fs::File> for Body` 是流式的，
            // 加上显式 Content-Length，内存占用与文件大小无关。
            let file = match tokio::fs::File::open(&ciphertext_path).await {
                Ok(f) => f,
                Err(e) => return CdnUploadOutcome::Fatal(format!("打开上传临时文件失败: {e}")),
            };
            let resp = http
                .post(&url)
                .header("Content-Type", "application/octet-stream")
                .header("Content-Length", len.to_string())
                .body(reqwest::Body::from(file))
                .timeout(timeout)
                .send()
                .await;
            let resp = match resp {
                Ok(r) => r,
                // 网络错误 / 超时：可重试
                Err(e) => return CdnUploadOutcome::Retryable(format!("CDN 上传失败: {e}")),
            };
            let status = resp.status();
            if status.is_client_error() {
                // 平台把错误说明放在 x-error-message 头里
                let detail = resp
                    .headers()
                    .get("x-error-message")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
                    .unwrap_or_else(|| resp.status().to_string());
                return CdnUploadOutcome::Fatal(format!("CDN 拒绝上传（{status}）: {detail}"));
            }
            if !status.is_success() {
                return CdnUploadOutcome::Retryable(format!("CDN 上传服务器错误: {status}"));
            }
            match resp.headers().get("x-encrypted-param") {
                Some(v) => match v.to_str() {
                    Ok(s) if !s.trim().is_empty() => CdnUploadOutcome::Ok(s.to_string()),
                    _ => CdnUploadOutcome::Retryable(
                        "CDN 响应的 x-encrypted-param 不可用".to_string(),
                    ),
                },
                None => {
                    CdnUploadOutcome::Retryable("CDN 响应缺少 x-encrypted-param 头".to_string())
                }
            }
        })
    }

    fn download_stream(
        &self,
        url: &str,
        timeout: Duration,
    ) -> BoxFuture<'_, Result<AttachmentStream, String>> {
        use futures_util::StreamExt;

        let http = self.http.clone();
        let url = url.to_string();
        Box::pin(async move {
            let resp = http
                .get(&url)
                .timeout(timeout)
                .send()
                .await
                .map_err(|e| format!("附件下载失败: {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("附件下载失败: CDN 返回 {}", resp.status()));
            }
            let stream = resp.bytes_stream().map(|r| {
                r.map(|b| b.to_vec())
                    .map_err(|e| format!("附件下载失败: {e}"))
            });
            Ok(Box::pin(stream) as AttachmentStream)
        })
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
pub enum WechatEvent {
    Qr {
        login_id: String,
        url: String,
    },
    LoginResult {
        login_id: String,
        success: bool,
        message: String,
        account_id: Option<String>,
        user_id: Option<String>,
        credentials: Option<LoginCredentials>,
    },
    Message {
        account_id: String,
        from: String,
        to: String,
        timestamp: u64,
        context_token: String,
        text: Option<String>,
        /// 已下载并解密的图片绝对路径（按 item_list 顺序）。
        images: Vec<String>,
        /// 已下载并解密的文件/视频绝对路径（按 item_list 顺序）。
        files: Vec<String>,
        /// 附件接收失败原因（同一消息多个附件时取首个失败原因）。
        media_error: Option<String>,
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
    /// 单账号接收循环已退出（`reason`：cancel｜session_expired｜account_missing）。
    /// 桥据此决定是否需要重启接收（可恢复退出自动重启一次，过期退出等重新扫码）。
    /// 仅供桥内部消费，不影响前端 `wechat/event` 快照形状。
    ReceiverStopped {
        account_id: String,
        reason: String,
    },
    Accounts(Vec<Value>),
}

/// 登录成功后交由桥接层确认归属，再持久化的凭据。
///
/// 不实现 `Debug`，避免意外日志输出访问令牌。
pub struct LoginCredentials {
    pub token: String,
    pub base_url: String,
}

/// 等待本次扫码登录被取消。`watch` 会保留最新值，因此取消先于任务首次轮询时也不会丢失。
async fn wait_for_login_cancel(cancel: &mut watch::Receiver<bool>) {
    loop {
        if *cancel.borrow_and_update() {
            return;
        }
        if cancel.changed().await.is_err() {
            return;
        }
    }
}

/// 等待一次登录 API 请求或取消信号；取消优先，确保已取消任务不再发布事件或提交结果。
async fn cancellable_login_request<T>(
    cancel: &mut watch::Receiver<bool>,
    request: impl Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        biased;
        _ = wait_for_login_cancel(cancel) => None,
        result = request => Some(result),
    }
}

/// 一次出站媒体上传+发送所需的参数（避免函数签名过长）。
struct UploadRequest<'a> {
    /// 已写入密文的临时文件
    temp: &'a Path,
    filekey: &'a str,
    aes_key: &'a [u8; 16],
    kind: OutboundMediaKind,
    file_name: &'a str,
    /// 明文大小
    raw_size: u64,
    /// 密文大小（PKCS7 填充后）
    cipher_size: u64,
    /// 明文 MD5（hex）
    md5_hex: &'a str,
}

pub struct WechatClient<A: WechatApi = ReqwestApi> {
    root: PathBuf,
    api: Arc<A>,
    tx: mpsc::UnboundedSender<WechatEvent>,
    receivers: Arc<Mutex<HashMap<String, Arc<Notify>>>>,
    /// typing_ticket 内存缓存：键为 (account_id, peer)。
    typing_tickets: Mutex<HashMap<(String, String), String>>,
    /// 账号 → 绑定会话 id（由桥同步）：决定附件落盘目录；缺失即不下载附件。
    bound_threads: Arc<Mutex<HashMap<String, String>>>,
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
            bound_threads: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 整体替换「账号 → 会话」映射（桥在装载绑定 / 绑定成功 / 解绑后调用）。
    pub async fn set_bindings(&self, map: HashMap<String, String>) {
        *self.bound_threads.lock().await = map;
    }

    pub fn accounts(&self) -> Vec<Value> {
        accounts_snapshot(&self.root)
    }

    /// 持久化由当前有效登录绑定所确认的凭据。
    pub fn persist_login(&self, account_id: &str, user_id: &str, credentials: LoginCredentials) {
        save_account(
            &self.root,
            account_id,
            &credentials.token,
            &credentials.base_url,
            user_id,
        );
    }

    /// 发起二维码登录（结果与二维码经事件推送，可用 cancel 取消）。
    pub fn start_login(&self, login_id: String, mut cancel: watch::Receiver<bool>) {
        let api = self.api.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let Some(qr_res) =
                cancellable_login_request(&mut cancel, api.get_qr_code(DEFAULT_BASE_URL, BOT_TYPE))
                    .await
            else {
                return;
            };
            let qr = match qr_res {
                Ok(v) => v,
                Err(e) => {
                    let _ = tx.send(WechatEvent::LoginResult {
                        login_id,
                        success: false,
                        message: e,
                        account_id: None,
                        user_id: None,
                        credentials: None,
                    });
                    return;
                }
            };
            let mut qrcode = qr
                .get("qrcode")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
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
                    credentials: None,
                });
                return;
            }
            let _ = tx.send(WechatEvent::Qr {
                login_id: login_id.clone(),
                url,
            });
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
                        credentials: None,
                    });
                    return;
                }
                let Some(status) = cancellable_login_request(
                    &mut cancel,
                    api.poll_qr_status(DEFAULT_BASE_URL, &qrcode),
                )
                .await
                else {
                    return;
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
                            credentials: None,
                        });
                        return;
                    }
                };
                match st.get("status").and_then(|v| v.as_str()).unwrap_or("") {
                    "confirmed" => {
                        let bot_id = st
                            .get("ilink_bot_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let bot_token = st
                            .get("bot_token")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let base_url = st
                            .get("baseurl")
                            .and_then(|v| v.as_str())
                            .unwrap_or(DEFAULT_BASE_URL)
                            .to_string();
                        let user_id = st
                            .get("ilink_user_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if bot_id.is_empty() || bot_token.is_empty() {
                            let _ = tx.send(WechatEvent::LoginResult {
                                login_id,
                                success: false,
                                message: "登录确认但缺少账号信息".into(),
                                account_id: None,
                                user_id: None,
                                credentials: None,
                            });
                            return;
                        }
                        let _ = tx.send(WechatEvent::LoginResult {
                            login_id,
                            success: true,
                            message: "与微信连接成功！".into(),
                            account_id: Some(bot_id),
                            user_id: Some(user_id),
                            credentials: Some(LoginCredentials {
                                token: bot_token,
                                base_url,
                            }),
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
                                credentials: None,
                            });
                            return;
                        }
                        let Some(refresh) = cancellable_login_request(
                            &mut cancel,
                            api.get_qr_code(DEFAULT_BASE_URL, BOT_TYPE),
                        )
                        .await
                        else {
                            return;
                        };
                        match refresh {
                            Ok(nq) => {
                                let new_url = nq
                                    .get("qrcode_img_content")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let new_code = nq
                                    .get("qrcode")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                if !new_url.is_empty() {
                                    let _ = tx.send(WechatEvent::Qr {
                                        login_id: login_id.clone(),
                                        url: new_url,
                                    });
                                }
                                if new_code.is_empty() {
                                    let _ = tx.send(WechatEvent::LoginResult {
                                        login_id,
                                        success: false,
                                        message: "刷新二维码失败".into(),
                                        account_id: None,
                                        user_id: None,
                                        credentials: None,
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
                                    credentials: None,
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
        let cancel = Arc::new(Notify::new());
        {
            let mut map = self.receivers.lock().await;
            if map.contains_key(&account_id) {
                return;
            }
            map.insert(account_id.clone(), cancel.clone());
        }
        let api = self.api.clone();
        let tx = self.tx.clone();
        let root = self.root.clone();
        let bound_threads = self.bound_threads.clone();
        let receivers = self.receivers.clone();
        tokio::spawn(async move {
            run_receiver(api, tx, root, account_id, cancel, bound_threads, receivers).await;
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
    /// 失败按可恢复性分类（[`WechatSendFailure`]）：确定性失败不重试，瞬时失败可短重试。
    pub async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
    ) -> Result<(), WechatSendFailure> {
        use WechatSendFailure::Fatal;
        let account =
            load_account(&self.root, account_id).ok_or_else(|| Fatal("账号不存在".to_string()))?;
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
            return Err(Fatal("微信会话已过期，请重新扫码".into()));
        }
        if status != "connected" {
            return Err(Fatal("微信账号未连接".into()));
        }
        let ctx = get_context_token(&self.root, account_id, to)
            .ok_or_else(|| Fatal("缺少回复上下文（24 小时被动窗口已过期）".to_string()))?;
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
        self.post_message(&base_url, &token, account_id, body).await
    }

    /// 发送一条已组装好的消息，并按可恢复性分类失败（文本与媒体共用）。
    ///
    /// 网络错误/超时 → `Retryable`；会话过期 → 落盘状态 + 广播事件后返回 `Fatal`；
    /// 其它上游业务错误（频控/内部错误等）→ `Retryable`。
    async fn post_message(
        &self,
        base_url: &str,
        token: &str,
        account_id: &str,
        body: Value,
    ) -> Result<(), WechatSendFailure> {
        use WechatSendFailure::{Fatal, Retryable};
        let resp = self
            .api
            .send_message(base_url, token, body)
            .await
            .map_err(Retryable)?;
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
                return Err(Fatal("微信会话已过期，请重新扫码".into()));
            }
            let msg = resp
                .get("errmsg")
                .and_then(|v| v.as_str())
                .unwrap_or("发送失败")
                .to_string();
            return Err(Retryable(msg));
        }
        Ok(())
    }

    /// 把本地文件发到微信（出站媒体）：加密上传到微信 CDN 后再发一条媒体消息。
    ///
    /// 与文本一致按可恢复性分类失败：路径不存在/是目录/超限、会话过期、缺被动窗口
    /// 属确定性失败；网络与 5xx 属可重试。
    pub async fn send_file(
        &self,
        account_id: &str,
        to: &str,
        path: &Path,
    ) -> Result<(), WechatSendFailure> {
        use crate::codex::path_util::clean_path;
        use WechatSendFailure::Fatal;

        // 前置校验（均在本地完成，避免白传一遍文件）
        let account =
            load_account(&self.root, account_id).ok_or_else(|| Fatal("账号不存在".to_string()))?;
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
        let status = load_session_status(&self.root, account_id);
        match status.get("status").and_then(|v| v.as_str()).unwrap_or("") {
            "session_expired" => return Err(Fatal("微信会话已过期，请重新扫码".into())),
            "connected" => {}
            _ => return Err(Fatal("微信账号未连接".into())),
        }
        let context_token = get_context_token(&self.root, account_id, to)
            .ok_or_else(|| Fatal("缺少回复上下文（24 小时被动窗口已过期）".to_string()))?;

        let meta = tokio::fs::metadata(path)
            .await
            .map_err(|e| Fatal(format!("无法读取文件 {}: {e}", clean_path(path))))?;
        if !meta.is_file() {
            return Err(Fatal(format!("不是文件: {}", clean_path(path))));
        }
        let raw_size = meta.len();
        if raw_size > MAX_OUTBOUND_BYTES {
            return Err(Fatal(format!(
                "文件过大（{} MB，上限 {} MB）",
                raw_size / (1024 * 1024),
                MAX_OUTBOUND_BYTES / (1024 * 1024)
            )));
        }

        let kind = outbound_media_kind(&path.to_string_lossy());
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "attachment".to_string());
        let aes_key = random_key().map_err(Fatal)?;
        let filekey = random_filekey().map_err(Fatal)?;

        // 加密到临时文件（流式：内存与文件大小无关），结束后无论成败都清理
        let temp = self.root.join(format!(
            ".wechat-upload-{}-{}.part",
            std::process::id(),
            ID_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let encrypted = encrypt_file_to(path, &temp, &aes_key).await;
        let (raw, cipher_size, md5_hex) = match encrypted {
            Ok(v) => v,
            Err(e) => {
                let _ = tokio::fs::remove_file(&temp).await;
                return Err(Fatal(e));
            }
        };
        let result = self
            .upload_and_send_media(
                account_id,
                to,
                &token,
                &base_url,
                &context_token,
                UploadRequest {
                    temp: &temp,
                    filekey: &filekey,
                    aes_key: &aes_key,
                    kind,
                    file_name: &file_name,
                    // 用实际扫描到的大小（与 md5/密文同源），而非元数据里的 size
                    raw_size: raw,
                    cipher_size,
                    md5_hex: &md5_hex,
                },
            )
            .await;
        let _ = tokio::fs::remove_file(&temp).await;
        result
    }

    /// 申请上传地址 → 上传密文（可重试）→ 发媒体消息。
    async fn upload_and_send_media(
        &self,
        account_id: &str,
        to: &str,
        token: &str,
        base_url: &str,
        context_token: &str,
        req: UploadRequest<'_>,
    ) -> Result<(), WechatSendFailure> {
        use WechatSendFailure::{Fatal, Retryable};

        let upload_body = json!({
            "filekey": req.filekey,
            "media_type": req.kind.upload_media_type(),
            "to_user_id": to,
            "rawsize": req.raw_size,
            "rawfilemd5": req.md5_hex,
            "filesize": req.cipher_size,
            // 不需要缩略图上传 URL：跳过缩略图生成与上传（参考实现同样传 true）
            "no_need_thumb": true,
            "aeskey": hex_lower(req.aes_key),
            "base_info": build_base_info(),
        });
        let resp = self
            .api
            .get_upload_url(base_url, token, upload_body)
            .await
            .map_err(|e| Retryable(format!("申请上传地址失败: {e}")))?;
        // 平台按上报的 `channel_version` 决定返回形态：旧版给 `upload_param`（与 filekey 自己拼
        // URL），新版给带 `taskid` 的完整 `upload_full_url`——后者必须原样使用，自己拼会丢 taskid。
        let upload_url = match resp
            .get("upload_full_url")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(full_url) => full_url.to_string(),
            None => {
                let upload_param = resp
                    .get("upload_param")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        Retryable("上传地址响应缺少 upload_full_url / upload_param".to_string())
                    })?;
                cdn_upload_url(upload_param, req.filekey)
            }
        };

        // 上传密文：4xx 致命不重试；5xx / 网络错误重试
        let mut download_param: Option<String> = None;
        let mut last_err = String::new();
        for attempt in 0..UPLOAD_RETRY_ATTEMPTS {
            match self
                .api
                .upload_to_cdn(
                    &upload_url,
                    req.temp.to_path_buf(),
                    req.cipher_size,
                    UPLOAD_TIMEOUT,
                )
                .await
            {
                CdnUploadOutcome::Ok(p) => {
                    download_param = Some(p);
                    break;
                }
                CdnUploadOutcome::Fatal(e) => return Err(Fatal(e)),
                CdnUploadOutcome::Retryable(e) => {
                    last_err = e;
                    if attempt + 1 < UPLOAD_RETRY_ATTEMPTS {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }
            }
        }
        let download_param = download_param.ok_or(Retryable(last_err))?;

        let body = json!({
            "msg": {
                "from_user_id": "",
                "to_user_id": to,
                "client_id": generate_id("wechannel"),
                "message_type": 2,
                "message_state": 2,
                "item_list": [build_media_item(
                    req.kind,
                    // 媒体项填上传响应头 `x-encrypted-param`（与参考实现一致）。
                    // 真机实测：这一项决定消息能否在微信里显示；换成 `upload_param`
                    // 消息会显示不出来（但 `upload_param` 却能取回对象，两者当前无法兼得）。
                    &download_param,
                    req.aes_key,
                    req.cipher_size,
                    req.raw_size,
                    req.file_name,
                )],
                "context_token": context_token,
            },
            "base_info": build_base_info(),
        });
        self.post_message(base_url, token, account_id, body).await
    }

    /// 取回并缓存对端用户的 typing_ticket（按“账号+用户”缓存，取到即复用）。
    async fn ensure_typing_ticket(&self, account_id: &str, to: &str) -> Result<String, String> {
        let key = (account_id.to_string(), to.to_string());
        if let Some(t) = self.typing_tickets.lock().await.get(&key).cloned() {
            return Ok(t);
        }
        let account =
            load_account(&self.root, account_id).ok_or_else(|| "账号不存在".to_string())?;
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
        let account =
            load_account(&self.root, account_id).ok_or_else(|| "账号不存在".to_string())?;
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
    bound_threads: Arc<Mutex<HashMap<String, String>>>,
    receivers: Arc<Mutex<HashMap<String, Arc<Notify>>>>,
) {
    let Some(account) = load_account(&root, &account_id) else {
        let _ = tx.send(WechatEvent::Error {
            message: format!("账号 {account_id} 不存在，无法接收"),
            kind: "receiver".into(),
        });
        finish_receiver(&tx, &receivers, &account_id, &cancel, "account_missing").await;
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
    let mut dedup = InboundDedup::new();
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
                finish_receiver(&tx, &receivers, &account_id, &cancel, "cancel").await;
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
                                finish_receiver(&tx, &receivers, &account_id, &cancel, "session_expired").await;
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
                                // 协议里 message_id 是数字；按「字符串或数字」读取，否则读不到
                                // 数字型 id，去重环会因恒为 None 而形同虚设。
                                let message_id = message_id_of(raw);
                                // 去重：平台在 sync-buf 写盘失败后可能重放队列，重复驱动回合
                                // （微信回合是「完全访问」，重复执行代价高）。命中即跳过该条。
                                if !dedup.accept(message_id.as_deref()) {
                                    continue;
                                }
                                if !from.is_empty() && !ctx.is_empty() {
                                    set_context_token(
                                        &root,
                                        &account_id,
                                        &from,
                                        &ctx,
                                        message_id.as_deref(),
                                    );
                                }
                                let items = raw.get("item_list");
                                let text = items.and_then(extract_message_text);
                                // 附件只在「该账号已绑定会话」时下载：目录即 media/<会话id>；
                                // 未绑定时不下载（消息照旧发出，由桥按「谁扫谁白」忽略）。
                                let thread_id = {
                                    let map = bound_threads.lock().await;
                                    map.get(&account_id).cloned()
                                };
                                let mut images: Vec<String> = Vec::new();
                                let mut files: Vec<String> = Vec::new();
                                let mut media_error: Option<String> = None;
                                if let Some(thread_id) = thread_id {
                                    // 附件：图片 → 文件 → 视频，顺序下载解密落盘（保持消息顺序）；
                                    // 单个失败不阻断其它附件与后续轮询。
                                    let image_refs = items.map(extract_message_images).unwrap_or_default();
                                    if image_refs.len() < count_image_items(items) {
                                        let _ = tx.send(WechatEvent::Error {
                                            message: format!(
                                                "单条消息图片超过 {MAX_IMAGES_PER_MESSAGE} 张，多余的已忽略"
                                            ),
                                            kind: "media".into(),
                                        });
                                    }
                                    let file_refs = items.map(extract_message_files).unwrap_or_default();
                                    let video_refs = items.map(extract_message_videos).unwrap_or_default();
                                    for img in &image_refs {
                                        match fetch_inbound_attachment(
                                            api.as_ref(),
                                            &root,
                                            &thread_id,
                                            &img.encrypt_query_param,
                                            &img.aes_key,
                                            AttachmentKind::Image,
                                            None,
                                        )
                                        .await
                                        {
                                            Ok(path) => images.push(path),
                                            Err(e) => {
                                                report_media_error(&tx, &mut media_error, AttachmentKind::Image, e)
                                            }
                                        }
                                    }
                                    for file in &file_refs {
                                        match fetch_inbound_attachment(
                                            api.as_ref(),
                                            &root,
                                            &thread_id,
                                            &file.encrypt_query_param,
                                            &file.aes_key,
                                            AttachmentKind::File,
                                            file.file_name.as_deref(),
                                        )
                                        .await
                                        {
                                            Ok(path) => files.push(path),
                                            Err(e) => {
                                                report_media_error(&tx, &mut media_error, AttachmentKind::File, e)
                                            }
                                        }
                                    }
                                    for video in &video_refs {
                                        match fetch_inbound_attachment(
                                            api.as_ref(),
                                            &root,
                                            &thread_id,
                                            &video.encrypt_query_param,
                                            &video.aes_key,
                                            AttachmentKind::Video,
                                            None,
                                        )
                                        .await
                                        {
                                            Ok(path) => files.push(path),
                                            Err(e) => {
                                                report_media_error(&tx, &mut media_error, AttachmentKind::Video, e)
                                            }
                                        }
                                    }
                                }
                                let _ = tx.send(WechatEvent::Message {
                                    account_id: account_id.clone(),
                                    from,
                                    to,
                                    timestamp: ts,
                                    context_token: ctx,
                                    text,
                                    images,
                                    files,
                                    media_error,
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

/// 入站消息去重环（进程内，FIFO 上限 [`INBOUND_DEDUP_CAPACITY`]）。
///
/// `message_id` 已见过 → 返回 `false`（调用方跳过该条）；否则登记并返回 `true`，
/// 超出上限时淘汰最旧的一条。空 id 不做去重（无 id 时无法判断重复，宁可放行）。
struct InboundDedup {
    seen: HashSet<String>,
    order: VecDeque<String>,
}

impl InboundDedup {
    fn new() -> Self {
        Self {
            seen: HashSet::new(),
            order: VecDeque::new(),
        }
    }

    /// 是否应处理该条消息（首次出现为 true，重复/空 id 为 false）。
    fn accept(&mut self, message_id: Option<&str>) -> bool {
        let Some(id) = message_id.map(str::trim).filter(|s| !s.is_empty()) else {
            return true;
        };
        if self.seen.contains(id) {
            return false;
        }
        self.seen.insert(id.to_string());
        self.order.push_back(id.to_string());
        while self.order.len() > INBOUND_DEDUP_CAPACITY {
            if let Some(old) = self.order.pop_front() {
                self.seen.remove(&old);
            }
        }
        true
    }
}

async fn sleep_or_cancel(cancel: &Notify, dur: Duration) {
    tokio::select! {
        _ = cancel.notified() => {}
        _ = tokio::time::sleep(dur) => {}
    }
}

/// 接收循环退出收尾：广播 [`WechatEvent::ReceiverStopped`] 并清理 `receivers` 表中的自身条目。
///
/// 清理是必需的——`start_receiver` 以「表中已有该账号」作为幂等短路条件，残留条目会让后续
/// 任何重启尝试静默失效（账号因 `session_expired` 退出后重新扫码也无法恢复接收）。
///
/// 仅当表中仍是本次运行注册的那个 `Notify` 时才删除（`Arc::ptr_eq`）：否则会把
/// 「stop_receiver 已清、随后 start_receiver 又注册」的新条目误删。
async fn finish_receiver(
    tx: &mpsc::UnboundedSender<WechatEvent>,
    receivers: &Arc<Mutex<HashMap<String, Arc<Notify>>>>,
    account_id: &str,
    cancel: &Arc<Notify>,
    reason: &str,
) {
    let _ = tx.send(WechatEvent::ReceiverStopped {
        account_id: account_id.to_string(),
        reason: reason.to_string(),
    });
    let mut map = receivers.lock().await;
    let mine = map.get(account_id).is_some_and(|n| Arc::ptr_eq(n, cancel));
    if mine {
        map.remove(account_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 出站媒体接口的测试桩：不涉及媒体上传的 mock 直接展开这段，
    /// 免得每个 `impl WechatApi` 都重复写两个用不到的方法。
    macro_rules! unsupported_media_api {
        () => {
            fn get_upload_url(
                &self,
                _base: &str,
                _tok: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Err("本用例不涉及媒体上传".to_string()) })
            }

            fn upload_to_cdn(
                &self,
                _url: &str,
                _path: PathBuf,
                _len: u64,
                _timeout: Duration,
            ) -> BoxFuture<'_, CdnUploadOutcome> {
                Box::pin(async {
                    CdnUploadOutcome::Fatal("本用例不涉及媒体上传".to_string())
                })
            }
        };
    }

    #[test]
    fn base_info_reports_channel_version_constant() {
        // 「关于」展示的协议版本取自同一个常量：这里保证它确实就是每次请求上报的值
        assert_eq!(PROTOCOL_NAME, "ilink bot API");
        assert_eq!(
            build_base_info()
                .get("channel_version")
                .and_then(|v| v.as_str()),
            Some(CHANNEL_VERSION)
        );
        assert!(!CHANNEL_VERSION.trim().is_empty());
    }

    #[derive(Default)]
    struct LoginApiGate {
        started: Notify,
        release: Notify,
    }

    #[derive(Default)]
    struct MockApi {
        // 可选门闩用于验证扫码请求等待期间取消不会丢失。
        qr_gate: Option<Arc<LoginApiGate>>,
        poll_gate: Option<Arc<LoginApiGate>>,
    }

    impl WechatApi for MockApi {
        unsupported_media_api!();
        fn get_qr_code(&self, _base: &str, _bot: &str) -> BoxFuture<'_, Result<Value, String>> {
            let gate = self.qr_gate.clone();
            Box::pin(async move {
                if let Some(gate) = gate {
                    gate.started.notify_one();
                    gate.release.notified().await;
                }
                Ok(json!({ "qrcode": "QR-1", "qrcode_img_content": "http://qr/1" }))
            })
        }
        fn poll_qr_status(
            &self,
            _base: &str,
            _qrcode: &str,
        ) -> BoxFuture<'_, Result<Value, String>> {
            let gate = self.poll_gate.clone();
            Box::pin(async move {
                if let Some(gate) = gate {
                    gate.started.notify_one();
                    gate.release.notified().await;
                }
                Ok(json!({
                    "status": "confirmed",
                    "ilink_bot_id": "bot-1@im.bot",
                    "bot_token": "tok-1",
                    "baseurl": "https://ilinkai.weixin.qq.com",
                    "ilink_user_id": "u-1@im.wechat",
                }))
            })
        }
        fn get_updates(
            &self,
            _base: &str,
            _tok: &str,
            _buf: &str,
            _t: u64,
        ) -> BoxFuture<'_, Result<Value, String>> {
            Box::pin(async { Ok(json!({ "ret": 0, "msgs": [], "get_updates_buf": "" })) })
        }
        fn send_message(
            &self,
            _base: &str,
            _tok: &str,
            _body: Value,
        ) -> BoxFuture<'_, Result<Value, String>> {
            Box::pin(async { Ok(json!({ "ret": 0 })) })
        }
        fn get_config(
            &self,
            _base: &str,
            _tok: &str,
            _body: Value,
        ) -> BoxFuture<'_, Result<Value, String>> {
            Box::pin(async { Ok(json!({ "ret": 0, "typing_ticket": "ticket-1" })) })
        }
        fn send_typing(
            &self,
            _base: &str,
            _tok: &str,
            _body: Value,
        ) -> BoxFuture<'_, Result<Value, String>> {
            Box::pin(async { Ok(json!({ "ret": 0 })) })
        }
        fn download_stream(
            &self,
            _url: &str,
            _timeout: Duration,
        ) -> BoxFuture<'_, Result<AttachmentStream, String>> {
            Box::pin(async { Err("测试未实现附件下载".to_string()) })
        }
    }

    #[test]
    fn expired_detection_narrowed() {
        assert!(is_session_expired_payload(&json!({ "errcode": -14 })));
        assert!(is_session_expired_payload(
            &json!({ "errmsg": "session expired" })
        ));
        assert!(is_session_expired_payload(
            &json!({ "errmsg": "token expired" })
        ));
        assert!(is_session_expired_payload(
            &json!({ "errmsg": "xxx session 已经 expired" })
        ));
        // timeout 不再误判为过期
        assert!(!is_session_expired_payload(
            &json!({ "errmsg": "upstream timeout" })
        ));
        assert!(!is_session_expired_payload(
            &json!({ "errmsg": "model not found" })
        ));
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

    /// 1x1 PNG：图片解密/落盘用例的明文素材。
    fn tiny_png() -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    /// AES-128-ECB + PKCS7 加密：测试侧构造微信 CDN 密文。
    fn encrypt_aes_ecb(plain: &[u8], key: &[u8; 16]) -> Vec<u8> {
        use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyInit};
        let mut buf = vec![0u8; plain.len() + 16];
        buf[..plain.len()].copy_from_slice(plain);
        ecb::Encryptor::<aes::Aes128>::new(key.into())
            .encrypt_padded_mut::<Pkcs7>(&mut buf, plain.len())
            .unwrap()
            .to_vec()
    }

    fn hex_of(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn message_image_extraction_prefers_hex_key_and_caps_count() {
        let list = json!([
            { "type": 1, "text_item": { "text": "看图" } },
            { "type": 2, "image_item": {
                "aeskey": "00112233445566778899aabbccddeeff",
                "media": { "encrypt_query_param": "P1", "aes_key": "QUJD" },
            } },
            { "type": 2, "image_item": { "media": { "encrypt_query_param": "P2", "aes_key": " QUJD " } } },
            // 缺 encrypt_query_param → 跳过
            { "type": 2, "image_item": { "media": {} } },
            { "type": 3, "voice_item": { "text": "转写" } },
        ]);
        let imgs = extract_message_images(&list);
        assert_eq!(imgs.len(), 2);
        assert_eq!(imgs[0].encrypt_query_param, "P1");
        assert_eq!(imgs[0].aes_key, "00112233445566778899aabbccddeeff");
        assert_eq!(imgs[1].encrypt_query_param, "P2");
        assert_eq!(imgs[1].aes_key, "QUJD");

        // 超过上限：只取前 MAX_IMAGES_PER_MESSAGE 张，计数函数反映真实数量
        let many = Value::Array(
            (0..6)
                .map(|i| json!({ "type": 2, "image_item": { "media": { "encrypt_query_param": format!("P{i}") } } }))
                .collect(),
        );
        assert_eq!(extract_message_images(&many).len(), MAX_IMAGES_PER_MESSAGE);
        assert_eq!(count_image_items(Some(&many)), 6);
        assert!(extract_message_images(&json!(null)).is_empty());
    }

    #[test]
    fn cdn_download_url_encodes_param() {
        assert_eq!(
            cdn_download_url("a b/c"),
            format!("{CDN_BASE_URL}/download?encrypted_query_param=a%20b%2Fc")
        );
    }

    #[test]
    fn message_file_and_video_extraction() {
        let list = json!([
            { "type": 1, "text_item": { "text": "看附件" } },
            { "type": 4, "file_item": {
                "file_name": "预算表.xlsx",
                "media": { "encrypt_query_param": "F1", "aes_key": "QUJD" },
            } },
            // 缺 encrypt_query_param → 跳过
            { "type": 4, "file_item": { "file_name": "空.xlsx", "media": {} } },
            // 缺 aes_key → 仍提取（按明文落盘）
            { "type": 4, "file_item": { "file_name": "无密钥.txt", "media": { "encrypt_query_param": "F2" } } },
            { "type": 5, "video_item": { "media": { "encrypt_query_param": "V1", "aes_key": "QUJD" } } },
            { "type": 5, "video_item": { "media": { "encrypt_query_param": "V2", "aes_key": "QUJD" } } },
            { "type": 5, "video_item": { "media": { "encrypt_query_param": "V3", "aes_key": "QUJD" } } },
        ]);
        let files = extract_message_files(&list);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].encrypt_query_param, "F1");
        assert_eq!(files[0].file_name.as_deref(), Some("预算表.xlsx"));
        assert_eq!(files[0].aes_key, "QUJD");
        assert_eq!(files[1].encrypt_query_param, "F2");
        assert_eq!(files[1].aes_key, "", "缺密钥应留空串（按明文处理）");

        let videos = extract_message_videos(&list);
        assert_eq!(videos.len(), MAX_VIDEOS_PER_MESSAGE, "视频最多取前 2 个");
        assert_eq!(videos[0].encrypt_query_param, "V1");
        assert!(videos[0].file_name.is_none(), "视频协议里没有原始文件名");

        assert!(extract_message_files(&json!(null)).is_empty());
        assert!(extract_message_videos(&json!([])).is_empty());
    }

    #[test]
    fn message_id_accepts_number_and_string_forms() {
        // 协议声明 message_id 为数字——按字符串读会一律返回 None（去重因此失效）
        assert_eq!(
            message_id_of(&json!({ "message_id": 1790180059616i64 })).as_deref(),
            Some("1790180059616")
        );
        assert_eq!(
            message_id_of(&json!({ "message_id": "m-1" })).as_deref(),
            Some("m-1")
        );
        assert_eq!(
            message_id_of(&json!({ "message_id": "  m-2  " })).as_deref(),
            Some("m-2"),
            "字符串应 trim"
        );
        // 缺失 / 空 / 空白 → None（调用方按「无法判断重复」放行）
        assert_eq!(message_id_of(&json!({})), None);
        assert_eq!(message_id_of(&json!({ "message_id": "" })), None);
        assert_eq!(message_id_of(&json!({ "message_id": "   " })), None);
        assert_eq!(message_id_of(&json!({ "message_id": null })), None);
        // 非字符串非数字 → None（不 panic、不误当 id）
        assert_eq!(message_id_of(&json!({ "message_id": { "a": 1 } })), None);
        assert_eq!(message_id_of(&json!({ "message_id": [1, 2] })), None);
    }

    #[test]
    fn ref_msg_attachments_are_extracted_one_level_only() {
        // 引用一条含图片 + 文件的消息转发：两层附件都应被提取
        let quoted = json!([
            {
                "type": 1,
                "text_item": { "text": "外层文字" },
                "ref_msg": {
                    "title": "摘要",
                    "message_item": {
                        "type": 2,
                        "image_item": { "media": { "encrypt_query_param": "REF-IMG", "aes_key": "QUJD" } },
                    },
                },
            },
            {
                "type": 4,
                "file_item": {
                    "file_name": "外层.xlsx",
                    "media": { "encrypt_query_param": "OUT-FILE", "aes_key": "QUJD" },
                },
                "ref_msg": {
                    "message_item": {
                        "type": 4,
                        "file_item": {
                            "file_name": "被引用.pdf",
                            "media": { "encrypt_query_param": "REF-FILE", "aes_key": "QUJD" },
                        },
                    },
                },
            },
        ]);

        let imgs = extract_message_images(&quoted);
        assert_eq!(imgs.len(), 1, "被引用消息里的图片应被提取");
        assert_eq!(imgs[0].encrypt_query_param, "REF-IMG");

        let files = extract_message_files(&quoted);
        assert_eq!(files.len(), 2, "外层与引用的文件都应被提取");
        assert_eq!(files[0].encrypt_query_param, "OUT-FILE");
        assert_eq!(
            files[1].encrypt_query_param, "REF-FILE",
            "引用里的文件排在自身之后"
        );
        assert_eq!(files[1].file_name.as_deref(), Some("被引用.pdf"));
    }

    #[test]
    fn ref_msg_is_not_traversed_recursively() {
        // 引用里再引用（两层）：只解一层，深层附件不应被提取，避免无限遍历
        let deep = json!([
            {
                "ref_msg": {
                    "message_item": {
                        "ref_msg": {
                            "message_item": {
                                "type": 2,
                                "image_item": { "media": { "encrypt_query_param": "DEEP" } },
                            },
                        },
                    },
                },
            },
        ]);
        assert!(
            extract_message_images(&deep).is_empty(),
            "两层嵌套不应被提取（只解一层）"
        );
    }

    #[test]
    fn ref_msg_attachments_share_message_quota() {
        // 外层 3 张 + 引用 3 张：配额共享，仍只取前 MAX_IMAGES_PER_MESSAGE 张
        let mut items: Vec<Value> = (0..3)
            .map(|i| json!({ "type": 2, "image_item": { "media": { "encrypt_query_param": format!("OUT-{i}") } } }))
            .collect();
        items.push(json!({
            "ref_msg": {
                "message_item": {
                    "type": 2,
                    "image_item": { "media": { "encrypt_query_param": "REF-0" } },
                },
            },
        }));
        // 再补两张引用图片，凑满 6 张
        items.push(json!({
            "ref_msg": {
                "message_item": {
                    "type": 2,
                    "image_item": { "media": { "encrypt_query_param": "REF-1" } },
                },
            },
        }));
        items.push(json!({
            "ref_msg": {
                "message_item": {
                    "type": 2,
                    "image_item": { "media": { "encrypt_query_param": "REF-2" } },
                },
            },
        }));
        let list = Value::Array(items);

        let imgs = extract_message_images(&list);
        assert_eq!(imgs.len(), MAX_IMAGES_PER_MESSAGE, "配额应共享");
        assert_eq!(imgs[0].encrypt_query_param, "OUT-0");
        assert_eq!(
            imgs[MAX_IMAGES_PER_MESSAGE - 1].encrypt_query_param,
            "REF-0",
            "第 4 张应来自引用"
        );
        // 溢出计数同样覆盖引用：用于「超过 N 张，多余的已忽略」提示
        assert_eq!(count_image_items(Some(&list)), 6);
    }

    #[test]
    fn text_extraction_ignores_quoted_body() {
        // 被引用消息的正文是上下文，不是本轮指令——不应被当成用户输入
        let only_quoted = json!([
            {
                "type": 0,
                "ref_msg": {
                    "message_item": { "type": 1, "text_item": { "text": "被引用的文字" } },
                },
            },
        ]);
        assert!(extract_message_text(&only_quoted).is_none());

        // 外层有文字时取外层；引用文字不参与
        let outer_text = json!([
            {
                "type": 1,
                "text_item": { "text": " 本轮指令 " },
                "ref_msg": {
                    "message_item": { "type": 1, "text_item": { "text": "被引用的文字" } },
                },
            },
        ]);
        assert_eq!(
            extract_message_text(&outer_text).as_deref(),
            Some("本轮指令")
        );
    }

    #[test]
    fn sanitize_file_name_cleans_and_limits() {
        assert_eq!(sanitize_file_name("C:\\tmp\\报表.xlsx"), "报表.xlsx");
        assert_eq!(sanitize_file_name("/etc/hosts"), "hosts");
        assert_eq!(
            sanitize_file_name("a<b>c:d\"e|f?g*h.txt"),
            "a_b_c_d_e_f_g_h.txt"
        );
        assert_eq!(sanitize_file_name("  ..name..  "), "name");
        assert_eq!(
            sanitize_file_name("a\u{7}b.txt"),
            "a_b.txt",
            "控制字符应替换"
        );
        assert_eq!(sanitize_file_name("   "), "attachment");
        assert_eq!(sanitize_file_name("dir\\"), "attachment");
        assert_eq!(sanitize_file_name("noext"), "noext");

        let long = format!("{}.txt", "长".repeat(200));
        let cleaned = sanitize_file_name(&long);
        assert_eq!(cleaned.chars().count(), MAX_FILE_NAME_CHARS);
        assert!(cleaned.ends_with(".txt"), "限长应保留扩展名: {cleaned}");
    }

    #[test]
    fn aes_key_parsing_accepts_three_encodings() {
        let key = [0x42u8; 16];
        let hex_key = hex_of(&key);
        assert_eq!(parse_aes_key(&hex_key).unwrap(), key);
        assert_eq!(parse_aes_key(&BASE64.encode(key)).unwrap(), key);
        assert_eq!(
            parse_aes_key(&BASE64.encode(hex_key.as_bytes())).unwrap(),
            key
        );
        assert!(parse_aes_key("QUJD").is_err());
        assert!(parse_aes_key("").is_err());
    }

    #[test]
    fn ecb_stream_decryptor_handles_chunk_boundaries() {
        let key = [0x42u8; 16];
        let hex_key = hex_of(&key);
        let plain = b"streamed attachment payload".to_vec();
        let cipher = encrypt_aes_ecb(&plain, &key);
        // 各种切块边界：明文与一次性解密一致
        for chunk_size in [1usize, 7, 15, 16, 17, 64] {
            let mut d = EcbStreamDecryptor::new(&hex_key).unwrap();
            let mut out = Vec::new();
            for chunk in cipher.chunks(chunk_size) {
                out.extend(d.push(chunk).unwrap());
            }
            out.extend(d.finish().unwrap());
            assert_eq!(out, plain, "chunk_size={chunk_size}");
        }
        // 无密钥：明文直通
        let mut d = EcbStreamDecryptor::new("").unwrap();
        assert_eq!(d.push(b"abc").unwrap(), b"abc");
        assert!(d.finish().unwrap().is_empty());
        // 密文长度不是 16 的整数倍 / 空密文：报错
        let mut d = EcbStreamDecryptor::new(&hex_key).unwrap();
        assert!(d.push(&[0u8; 3]).unwrap().is_empty());
        assert!(d.finish().is_err());
        assert!(EcbStreamDecryptor::new(&hex_key).unwrap().finish().is_err());
    }

    #[test]
    fn resolve_final_name_applies_kind_rules() {
        let now = chrono::Local::now();
        let png = tiny_png();
        let name = resolve_final_name(AttachmentKind::Image, &png, None, now).unwrap();
        assert!(
            name.starts_with("wechat-") && name.ends_with(".png"),
            "{name}"
        );
        assert!(
            resolve_final_name(AttachmentKind::Image, b"not an image", None, now).is_err(),
            "魔数识别不出应报错"
        );
        assert_eq!(
            resolve_final_name(
                AttachmentKind::File,
                b"x",
                Some("C:\\tmp\\预算表.xlsx"),
                now
            )
            .unwrap(),
            "预算表.xlsx"
        );
        assert_eq!(
            resolve_final_name(AttachmentKind::File, b"x", None, now).unwrap(),
            "attachment"
        );
        let video =
            resolve_final_name(AttachmentKind::Video, b"x", Some("ignored.mp4"), now).unwrap();
        assert!(
            video.starts_with("wechat-video-") && video.ends_with(".mp4"),
            "{video}"
        );
    }

    #[test]
    fn unique_path_dedupes_within_dir() {
        let dir = tempfile::tempdir().unwrap();
        let first = unique_path(dir.path(), "预算表.xlsx");
        assert_eq!(first, dir.path().join("预算表.xlsx"));
        std::fs::write(&first, b"one").unwrap();
        let second = unique_path(dir.path(), "预算表.xlsx");
        assert_ne!(second, first, "同名应去重");
        let name = second.file_name().unwrap().to_string_lossy().to_string();
        assert!(
            name.starts_with("预算表-") && name.ends_with(".xlsx"),
            "{name}"
        );
        // 无扩展名同样处理
        assert_eq!(unique_path(dir.path(), "noext"), dir.path().join("noext"));
    }

    #[test]
    fn sanitize_path_segment_rules() {
        assert_eq!(
            sanitize_path_segment("01a0a2c4-2702-7bf0-9f61-9f280145e222"),
            "01a0a2c4-2702-7bf0-9f61-9f280145e222"
        );
        assert_eq!(sanitize_path_segment("a/b:c"), "a_b_c");
        assert_eq!(sanitize_path_segment("..\\.."), ".._..");
        assert_eq!(sanitize_path_segment("  "), "_unknown");
        assert_eq!(sanitize_path_segment("..."), "_unknown");
    }

    /// 只实现下载流的测试 API：按给定分块返回数据，或返回永不产出的流（超时用例）。
    struct StreamApi {
        url_match: String,
        chunks: Vec<Vec<u8>>,
        pending: bool,
    }

    impl WechatApi for StreamApi {
        unsupported_media_api!();
        fn get_qr_code(&self, _b: &str, _t: &str) -> BoxFuture<'_, Result<Value, String>> {
            Box::pin(async { Ok(json!({})) })
        }
        fn poll_qr_status(&self, _b: &str, _q: &str) -> BoxFuture<'_, Result<Value, String>> {
            Box::pin(async { Ok(json!({ "status": "wait" })) })
        }
        fn get_updates(
            &self,
            _b: &str,
            _t: &str,
            _buf: &str,
            _to: u64,
        ) -> BoxFuture<'_, Result<Value, String>> {
            Box::pin(async { Ok(json!({ "ret": 0, "msgs": [] })) })
        }
        fn send_message(
            &self,
            _b: &str,
            _t: &str,
            _body: Value,
        ) -> BoxFuture<'_, Result<Value, String>> {
            Box::pin(async { Ok(json!({ "ret": 0 })) })
        }
        fn get_config(
            &self,
            _b: &str,
            _t: &str,
            _body: Value,
        ) -> BoxFuture<'_, Result<Value, String>> {
            Box::pin(async { Ok(json!({ "ret": 0 })) })
        }
        fn send_typing(
            &self,
            _b: &str,
            _t: &str,
            _body: Value,
        ) -> BoxFuture<'_, Result<Value, String>> {
            Box::pin(async { Ok(json!({ "ret": 0 })) })
        }
        fn download_stream(
            &self,
            url: &str,
            _timeout: Duration,
        ) -> BoxFuture<'_, Result<AttachmentStream, String>> {
            assert!(
                url.contains(&self.url_match),
                "下载 URL 应带参数 {}: {url}",
                self.url_match
            );
            if self.pending {
                return Box::pin(async {
                    Ok(
                        Box::pin(futures_util::stream::pending::<Result<Vec<u8>, String>>())
                            as AttachmentStream,
                    )
                });
            }
            let items: Vec<Result<Vec<u8>, String>> = self.chunks.iter().cloned().map(Ok).collect();
            Box::pin(
                async move { Ok(Box::pin(futures_util::stream::iter(items)) as AttachmentStream) },
            )
        }
    }

    /// 按固定大小切块（覆盖流式解密的跨块边界）。
    fn split_chunks(data: &[u8], size: usize) -> Vec<Vec<u8>> {
        data.chunks(size.max(1)).map(|c| c.to_vec()).collect()
    }

    #[tokio::test]
    async fn save_attachment_streams_image_into_session_dir() {
        let png = tiny_png();
        let key = [3u8; 16];
        let cipher = encrypt_aes_ecb(&png, &key);
        let api = StreamApi {
            url_match: "encrypted_query_param=PARAM-IMG".into(),
            chunks: split_chunks(&cipher, 7),
            pending: false,
        };
        let dir = tempfile::tempdir().unwrap();
        let thread = "01a0a2c4-2702-7bf0-9f61-9f280145e222";
        let path = save_attachment(
            &api,
            dir.path(),
            thread,
            &cdn_download_url("PARAM-IMG"),
            &BASE64.encode(key),
            AttachmentKind::Image,
            None,
            Duration::from_secs(30),
        )
        .await
        .unwrap();
        let file = std::path::PathBuf::from(&path);
        let session_dir = dir
            .path()
            .join("wechannel-data")
            .join(MEDIA_DIR_NAME)
            .join(thread);
        assert_eq!(
            file.parent().unwrap(),
            session_dir,
            "应落在 media/<会话id>/"
        );
        let name = file.file_name().unwrap().to_string_lossy().to_string();
        assert!(
            name.starts_with("wechat-") && name.ends_with(".png"),
            "{name}"
        );
        assert_eq!(std::fs::read(&file).unwrap(), png);
        assert!(
            !std::fs::read_dir(&session_dir)
                .unwrap()
                .flatten()
                .any(|e| e.file_name().to_string_lossy().ends_with(".part")),
            "不应残留 .part"
        );
    }

    #[tokio::test]
    async fn save_attachment_writes_file_with_original_name() {
        let content = b"quarterly report".to_vec();
        let api = StreamApi {
            url_match: "encrypted_query_param=PARAM-F".into(),
            chunks: split_chunks(&content, 4),
            pending: false,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = save_attachment(
            &api,
            dir.path(),
            "t-1",
            &cdn_download_url("PARAM-F"),
            "",
            AttachmentKind::File,
            Some("C:\\微信\\季度报表.xlsx"),
            Duration::from_secs(30),
        )
        .await
        .unwrap();
        let file = std::path::PathBuf::from(&path);
        assert_eq!(file.file_name().unwrap().to_string_lossy(), "季度报表.xlsx");
        assert_eq!(std::fs::read(&file).unwrap(), content);
    }

    #[tokio::test]
    async fn save_attachment_timeout_cleans_part() {
        let api = StreamApi {
            url_match: "encrypted_query_param=PARAM-T".into(),
            chunks: Vec::new(),
            pending: true,
        };
        let dir = tempfile::tempdir().unwrap();
        let err = save_attachment(
            &api,
            dir.path(),
            "t-timeout",
            &cdn_download_url("PARAM-T"),
            "",
            AttachmentKind::Video,
            None,
            Duration::from_millis(50),
        )
        .await
        .unwrap_err();
        assert!(err.contains("超时"), "{err}");
        let session_dir = dir
            .path()
            .join("wechannel-data")
            .join(MEDIA_DIR_NAME)
            .join("t-timeout");
        let leftovers: Vec<String> = std::fs::read_dir(&session_dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(leftovers.is_empty(), "超时后应清理临时文件: {leftovers:?}");
    }

    #[tokio::test]
    async fn save_attachment_accepts_large_stream() {
        // 60 MiB（超过改造前 50 MB 的文件上限）：回归「已取消大小上限」且分块落盘正常。
        const MIB: usize = 1024 * 1024;
        let api = StreamApi {
            url_match: "encrypted_query_param=PARAM-BIG".into(),
            chunks: vec![vec![7u8; MIB]; 60],
            pending: false,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = save_attachment(
            &api,
            dir.path(),
            "t-big",
            &cdn_download_url("PARAM-BIG"),
            "",
            AttachmentKind::File,
            Some("big.bin"),
            Duration::from_secs(120),
        )
        .await
        .unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), (60 * MIB) as u64);
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
        save_account(
            root,
            "bot-1@im.bot",
            "tok",
            "https://ilinkai.weixin.qq.com",
            "u-1",
        );
        assert_eq!(load_account_ids(root), vec!["bot-1@im.bot"]);
        assert!(load_account(root, "bot-1@im.bot").is_some());
        save_sync_buf(root, "bot-1@im.bot", "buf-abc");
        assert_eq!(load_sync_buf(root, "bot-1@im.bot"), "buf-abc");
        set_context_token(root, "bot-1@im.bot", "u-1", "ctx-1", None);
        assert_eq!(
            get_context_token(root, "bot-1@im.bot", "u-1").as_deref(),
            Some("ctx-1")
        );
        // 过期窗口：写入过期时间后取不到
        let p = reply_context_path(root, "bot-1@im.bot");
        let mut map = load_reply_context(root, "bot-1@im.bot");
        map.insert(
            "u-2".into(),
            json!({ "peerId": "u-2", "contextToken": "old", "lastInboundAt": 1, "expiresAt": 1 }),
        );
        let obj = map
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<Value>();
        write_json(&p, &obj).unwrap();
        assert!(get_context_token(root, "bot-1@im.bot", "u-2").is_none());
        // 快照
        let snap = accounts_snapshot(root);
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0]["configured"], json!(true));
        assert_eq!(snap[0]["userId"], "u-1");
    }

    /// 入站附件端到端：getUpdates 带图片/文件/视频 → CDN 下载 → 解密 → 落盘 → 事件带路径。
    /// 用多线程 runtime + 后续轮询 yield，避免立即返回的 mock 把长轮询循环变成忙循环。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn receiver_downloads_attachments_into_message_event() {
        struct MediaApi {
            /// 加密参数 → 密文
            ciphers: std::collections::HashMap<String, Vec<u8>>,
            key_hex: String,
            key_b64: String,
        }
        impl WechatApi for MediaApi {
            unsupported_media_api!();
            fn get_qr_code(&self, _b: &str, _t: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn poll_qr_status(&self, _b: &str, _q: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "status": "wait" })) })
            }
            fn get_updates(
                &self,
                _b: &str,
                _t: &str,
                buf: &str,
                _to: u64,
            ) -> BoxFuture<'_, Result<Value, String>> {
                // 首次返回一条含图片/文件/视频的消息，之后返回空，避免测试忙循环反复投递。
                let first = buf.is_empty();
                let key_hex = self.key_hex.clone();
                let key_b64 = self.key_b64.clone();
                Box::pin(async move {
                    if first {
                        Ok(json!({
                            "ret": 0,
                            "get_updates_buf": "b1",
                            "msgs": [{
                                "from_user_id": "u-1",
                                "to_user_id": "bot-1",
                                "create_time_ms": 1,
                                "context_token": "ctx-1",
                                "message_id": "m-1",
                                "item_list": [
                                    { "type": 2, "image_item": {
                                        "aeskey": key_hex,
                                        "media": { "encrypt_query_param": "PARAM-IMG" },
                                    } },
                                    { "type": 4, "file_item": {
                                        "file_name": "季度报表.xlsx",
                                        "media": { "encrypt_query_param": "PARAM-FILE", "aes_key": key_b64.clone() },
                                    } },
                                    { "type": 5, "video_item": {
                                        "media": { "encrypt_query_param": "PARAM-VIDEO", "aes_key": key_b64 },
                                    } },
                                ],
                            }],
                        }))
                    } else {
                        // 真实长轮询会挂起；此处 sleep 让出调度，避免忙循环独占 runtime。
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        Ok(json!({ "ret": 0, "get_updates_buf": "b1", "msgs": [] }))
                    }
                })
            }
            fn send_message(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
            fn get_config(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
            fn send_typing(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
            fn download_stream(
                &self,
                url: &str,
                _timeout: Duration,
            ) -> BoxFuture<'_, Result<AttachmentStream, String>> {
                let param = ["PARAM-IMG", "PARAM-FILE", "PARAM-VIDEO"]
                    .into_iter()
                    .find(|p| url.contains(&format!("encrypted_query_param={p}")))
                    .unwrap_or_else(|| panic!("未预期的下载 URL: {url}"));
                let cipher = self.ciphers.get(param).expect("缺少对应密文").clone();
                // 分两块返回，顺带覆盖流式解密的跨块拼接。
                let mid = cipher.len() / 2;
                let tail = cipher[mid..].to_vec();
                let head = cipher[..mid].to_vec();
                Box::pin(async move {
                    Ok(
                        Box::pin(futures_util::stream::iter(vec![Ok(head), Ok(tail)]))
                            as AttachmentStream,
                    )
                })
            }
        }

        let png = tiny_png();
        let file_bytes = b"xlsx-content".to_vec();
        let video_bytes = b"video-content".to_vec();
        let key = [7u8; 16];
        let mut ciphers = std::collections::HashMap::new();
        ciphers.insert("PARAM-IMG".to_string(), encrypt_aes_ecb(&png, &key));
        ciphers.insert("PARAM-FILE".to_string(), encrypt_aes_ecb(&file_bytes, &key));
        ciphers.insert(
            "PARAM-VIDEO".to_string(),
            encrypt_aes_ecb(&video_bytes, &key),
        );
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        save_account(&root, "bot-1", "tok", DEFAULT_BASE_URL, "u-1");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let cancel = Arc::new(Notify::new());
        // 账号已绑定会话：附件应落到 media/<会话id>/ 下。
        let thread_id = "01a0a2c4-2702-7bf0-9f61-9f280145e222";
        let mut bindings = HashMap::new();
        bindings.insert("bot-1".to_string(), thread_id.to_string());
        // receivers 表预置本次的 Notify：验证退出时会清理自身条目
        let receivers = Arc::new(Mutex::new(HashMap::from([(
            "bot-1".to_string(),
            cancel.clone(),
        )])));
        let handle = tokio::spawn(run_receiver(
            Arc::new(MediaApi {
                ciphers,
                key_hex: hex_of(&key),
                key_b64: BASE64.encode(key),
            }),
            tx,
            root.clone(),
            "bot-1".into(),
            cancel.clone(),
            Arc::new(Mutex::new(bindings)),
            receivers.clone(),
        ));
        let got = loop {
            match rx.recv().await {
                Some(WechatEvent::Message {
                    text,
                    images,
                    files,
                    media_error,
                    ..
                }) => break (text, images, files, media_error),
                Some(_) => continue,
                None => panic!("事件通道提前关闭"),
            }
        };
        cancel.notify_one();
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("接收器应在收到取消信号后退出")
            .expect("接收器任务不应 panic");
        // 退出时必须清掉 receivers 表中的自身条目，否则 start_receiver 的幂等短路
        // 会让该账号再也无法重启接收（账号过期后重新扫码也收不到消息）。
        assert!(
            receivers.lock().await.is_empty(),
            "接收器退出后应清理 receivers 表中的自身条目"
        );

        let (text, images, files, media_error) = got;
        assert!(text.is_none(), "纯附件消息不应有文本");
        assert_eq!(media_error, None);
        assert_eq!(images.len(), 1);
        assert_eq!(files.len(), 2, "应落盘 1 个文件 + 1 个视频");
        // 落盘目录为 media/<会话id>/
        let media_root = root
            .join("wechannel-data")
            .join(MEDIA_DIR_NAME)
            .join(thread_id);
        for p in images.iter().chain(files.iter()) {
            assert!(
                crate::codex::path_util::is_inside_path(&media_root, &std::path::PathBuf::from(p)),
                "应落盘到 <root>/wechannel-data/media/<会话id>: {p}"
            );
        }
        // 临时 .part 不应残留
        let leftovers: Vec<String> = std::fs::read_dir(&media_root)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".part"))
            .collect();
        assert!(leftovers.is_empty(), "不应残留临时文件: {leftovers:?}");
        assert_eq!(
            std::fs::read(&images[0]).unwrap(),
            png,
            "图片内容应为解密后的原图"
        );
        // 文件保留微信原文件名；视频没有原名，用生成名
        let name_of = |p: &str| {
            std::path::PathBuf::from(p)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string()
        };
        let report = files
            .iter()
            .find(|p| name_of(p.as_str()) == "季度报表.xlsx")
            .expect("文件应保留原始文件名");
        assert_eq!(std::fs::read(report).unwrap(), file_bytes);
        let video = files
            .iter()
            .find(|p| {
                let n = name_of(p.as_str());
                n.starts_with("wechat-video-") && n.ends_with(".mp4")
            })
            .unwrap_or_else(|| panic!("视频应用生成名: {files:?}"));
        assert_eq!(std::fs::read(video).unwrap(), video_bytes);
    }

    /// 未绑定账号：附件一律不下载、不落盘，事件也不带附件。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn receiver_skips_attachments_when_unbound() {
        struct UnboundApi {
            key_hex: String,
        }
        impl WechatApi for UnboundApi {
            unsupported_media_api!();
            fn get_qr_code(&self, _b: &str, _t: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn poll_qr_status(&self, _b: &str, _q: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "status": "wait" })) })
            }
            fn get_updates(
                &self,
                _b: &str,
                _t: &str,
                buf: &str,
                _to: u64,
            ) -> BoxFuture<'_, Result<Value, String>> {
                let first = buf.is_empty();
                let key_hex = self.key_hex.clone();
                Box::pin(async move {
                    if first {
                        Ok(json!({
                            "ret": 0,
                            "get_updates_buf": "b1",
                            "msgs": [{
                                "from_user_id": "u-1",
                                "to_user_id": "bot-1",
                                "create_time_ms": 1,
                                "context_token": "ctx-1",
                                "message_id": "m-1",
                                "item_list": [
                                    { "type": 2, "image_item": {
                                        "aeskey": key_hex,
                                        "media": { "encrypt_query_param": "PARAM-IMG" },
                                    } },
                                ],
                            }],
                        }))
                    } else {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        Ok(json!({ "ret": 0, "get_updates_buf": "b1", "msgs": [] }))
                    }
                })
            }
            fn send_message(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
            fn get_config(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
            fn send_typing(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
            fn download_stream(
                &self,
                url: &str,
                _timeout: Duration,
            ) -> BoxFuture<'_, Result<AttachmentStream, String>> {
                panic!("未绑定账号不应触发附件下载: {url}");
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        save_account(&root, "bot-1", "tok", DEFAULT_BASE_URL, "u-1");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let cancel = Arc::new(Notify::new());
        let handle = tokio::spawn(run_receiver(
            Arc::new(UnboundApi {
                key_hex: hex_of(&[7u8; 16]),
            }),
            tx,
            root.clone(),
            "bot-1".into(),
            cancel.clone(),
            // 空映射：该账号没有绑定会话
            Arc::new(Mutex::new(HashMap::new())),
            Arc::new(Mutex::new(HashMap::new())),
        ));
        let got = loop {
            match rx.recv().await {
                Some(WechatEvent::Message {
                    text,
                    images,
                    files,
                    media_error,
                    ..
                }) => break (text, images, files, media_error),
                Some(_) => continue,
                None => panic!("事件通道提前关闭"),
            }
        };
        cancel.notify_one();
        let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;

        let (text, images, files, media_error) = got;
        assert!(text.is_none());
        assert!(images.is_empty() && files.is_empty(), "未绑定不应落盘附件");
        assert_eq!(media_error, None);
        assert!(
            !root.join("wechannel-data").join(MEDIA_DIR_NAME).exists(),
            "未绑定不应创建媒体目录"
        );
    }

    #[tokio::test]
    async fn receiver_reports_session_expired_and_stops() {
        struct ExpiredApi;
        impl WechatApi for ExpiredApi {
            unsupported_media_api!();
            fn get_qr_code(&self, _b: &str, _t: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "qrcode": "q", "qrcode_img_content": "u" })) })
            }
            fn poll_qr_status(&self, _b: &str, _q: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "status": "wait" })) })
            }
            fn get_updates(
                &self,
                _b: &str,
                _t: &str,
                _buf: &str,
                _to: u64,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "errcode": -14, "errmsg": "session expired" })) })
            }
            fn send_message(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
            fn get_config(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0, "typing_ticket": "ticket-x" })) })
            }
            fn send_typing(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
            fn download_stream(
                &self,
                _url: &str,
                _timeout: Duration,
            ) -> BoxFuture<'_, Result<AttachmentStream, String>> {
                Box::pin(async { Err("测试未实现附件下载".to_string()) })
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        save_account(&root, "bot-1", "tok", DEFAULT_BASE_URL, "u-1");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let cancel = Arc::new(Notify::new());
        let api = Arc::new(ExpiredApi);
        // receivers 表预置本次的 Notify：验证过期退出会清理自身条目
        let receivers = Arc::new(Mutex::new(HashMap::from([(
            "bot-1".to_string(),
            cancel.clone(),
        )])));
        run_receiver(
            api,
            tx,
            root.clone(),
            "bot-1".into(),
            cancel,
            Arc::new(Mutex::new(HashMap::new())),
            receivers.clone(),
        )
        .await;
        let mut expired = false;
        let mut stopped_reason = None;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                WechatEvent::SessionStatus { status, .. } if status == "session_expired" => {
                    expired = true;
                }
                WechatEvent::ReceiverStopped { reason, .. } => stopped_reason = Some(reason),
                _ => {}
            }
        }
        assert!(expired);
        // 过期退出需上报原因，桥据此判定「不自动重启、等重新扫码」
        assert_eq!(stopped_reason.as_deref(), Some("session_expired"));
        // 退出必须清表：否则账号重新扫码后 start_receiver 会被幂等短路拦住、再也收不到消息
        assert!(
            receivers.lock().await.is_empty(),
            "过期退出后应清理 receivers 表中的自身条目"
        );
        assert_eq!(
            load_session_status(&root, "bot-1")
                .get("status")
                .and_then(|v| v.as_str()),
            Some("session_expired")
        );
    }

    #[tokio::test]
    async fn login_flow_emits_qr_and_success() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let client = WechatClient::with_api(root, tx, MockApi::default());
        let (cancel_tx, cancel_rx) = watch::channel(false);
        client.start_login("L1".into(), cancel_rx);
        let mut qr_seen = false;
        let mut ok = false;
        let mut acc = None;
        let mut credentials = None;
        while let Some(ev) = rx.recv().await {
            match ev {
                WechatEvent::Qr { login_id, .. } => {
                    assert_eq!(login_id, "L1");
                    qr_seen = true;
                }
                WechatEvent::LoginResult {
                    success,
                    account_id,
                    credentials: result_credentials,
                    ..
                } => {
                    ok = success;
                    acc = account_id;
                    credentials = result_credentials;
                    break;
                }
                _ => {}
            }
        }
        assert!(qr_seen);
        assert!(ok);
        assert_eq!(acc.as_deref(), Some("bot-1@im.bot"));
        assert!(client.accounts().is_empty(), "凭据应等待桥接层确认后再落盘");
        client.persist_login(
            acc.as_deref().unwrap(),
            "u-1@im.wechat",
            credentials.expect("登录成功应携带待提交凭据"),
        );
        assert_eq!(client.accounts().len(), 1);
        drop(cancel_tx);
    }

    #[tokio::test]
    async fn cancel_during_qr_request_is_not_lost() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let gate = Arc::new(LoginApiGate::default());
        let client = WechatClient::with_api(
            root,
            tx,
            MockApi {
                qr_gate: Some(gate.clone()),
                poll_gate: None,
            },
        );
        let (cancel_tx, cancel_rx) = watch::channel(false);
        client.start_login("L-qr".into(), cancel_rx);

        gate.started.notified().await;
        cancel_tx.send_replace(true);

        assert!(
            tokio::time::timeout(Duration::from_millis(50), rx.recv())
                .await
                .is_err(),
            "取消后的旧登录不应再发布二维码或结果"
        );
        assert!(client.accounts().is_empty());

        // 旧任务被取消后立刻重开：只有新登录 id 的二维码和结果应继续流动。
        gate.release.notify_one();
        let (_new_cancel_tx, new_cancel_rx) = watch::channel(false);
        client.start_login("L-new".into(), new_cancel_rx);
        let mut new_qr_seen = false;
        let mut new_login_succeeded = false;
        while let Some(ev) = rx.recv().await {
            match ev {
                WechatEvent::Qr { login_id, .. } => {
                    assert_eq!(login_id, "L-new");
                    new_qr_seen = true;
                }
                WechatEvent::LoginResult {
                    login_id, success, ..
                } => {
                    assert_eq!(login_id, "L-new");
                    new_login_succeeded = success;
                    break;
                }
                _ => {}
            }
        }
        assert!(new_qr_seen);
        assert!(new_login_succeeded);
        assert!(client.accounts().is_empty(), "仅客户端确认不能提交登录凭据");
    }

    #[tokio::test]
    async fn cancel_during_qr_poll_is_not_lost() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let gate = Arc::new(LoginApiGate::default());
        let client = WechatClient::with_api(
            root,
            tx,
            MockApi {
                qr_gate: None,
                poll_gate: Some(gate.clone()),
            },
        );
        let (cancel_tx, cancel_rx) = watch::channel(false);
        client.start_login("L-poll".into(), cancel_rx);

        assert!(matches!(
            rx.recv().await,
            Some(WechatEvent::Qr { login_id, .. }) if login_id == "L-poll"
        ));
        gate.started.notified().await;
        cancel_tx.send_replace(true);

        assert!(
            tokio::time::timeout(Duration::from_millis(50), rx.recv())
                .await
                .is_err(),
            "取消后的轮询不应再发布登录结果"
        );
        assert!(client.accounts().is_empty());
    }

    #[tokio::test]
    async fn send_typing_fetches_ticket_once_then_reuses_and_skips_when_disconnected() {
        struct CountingApi {
            get_config_calls: Arc<std::sync::Mutex<usize>>,
            send_typing_calls: Arc<std::sync::Mutex<usize>>,
        }
        impl WechatApi for CountingApi {
            unsupported_media_api!();
            fn get_qr_code(&self, _b: &str, _t: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn poll_qr_status(&self, _b: &str, _q: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "status": "wait" })) })
            }
            fn get_updates(
                &self,
                _b: &str,
                _t: &str,
                _buf: &str,
                _to: u64,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0, "msgs": [], "get_updates_buf": "" })) })
            }
            fn send_message(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
            fn get_config(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                *self.get_config_calls.lock().unwrap() += 1;
                Box::pin(async { Ok(json!({ "ret": 0, "typing_ticket": "ticket-1" })) })
            }
            fn send_typing(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                *self.send_typing_calls.lock().unwrap() += 1;
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
            fn download_stream(
                &self,
                _url: &str,
                _timeout: Duration,
            ) -> BoxFuture<'_, Result<AttachmentStream, String>> {
                Box::pin(async { Err("测试未实现附件下载".to_string()) })
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
            CountingApi {
                get_config_calls: gcc.clone(),
                send_typing_calls: stc.clone(),
            },
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

    #[test]
    fn inbound_dedup_accepts_once_per_message_id() {
        let mut dedup = InboundDedup::new();
        assert!(dedup.accept(Some("m-1")));
        // 同一 id 重复投递（平台按空 buf 重放队列）→ 只处理一次
        assert!(!dedup.accept(Some("m-1")));
        // 不同 id 正常放行
        assert!(dedup.accept(Some("m-2")));
        // 空 id / 缺失 id：无法判断重复，宁可放行
        assert!(dedup.accept(None));
        assert!(dedup.accept(Some("")));
        assert!(dedup.accept(Some("   ")));
    }

    #[test]
    fn inbound_dedup_evicts_oldest_beyond_capacity() {
        let mut dedup = InboundDedup::new();
        for i in 0..INBOUND_DEDUP_CAPACITY {
            assert!(dedup.accept(Some(&format!("m-{i}"))));
        }
        // 环刚好装满：最早的 id 仍在环内，视为已见
        assert!(!dedup.accept(Some("m-0")));
        // 再塞一条把最旧的（m-0）挤出环，容量保持不变
        assert!(dedup.accept(Some("m-new")));
        assert_eq!(dedup.order.len(), INBOUND_DEDUP_CAPACITY);
        assert_eq!(dedup.seen.len(), INBOUND_DEDUP_CAPACITY);
        // m-0 已被淘汰 → 视为新消息放行；重新登记又会挤掉下一条（m-1）
        assert!(dedup.accept(Some("m-0")));
        assert!(dedup.accept(Some("m-1")));
        // 仍在环内的 id 依旧是重复消息
        assert!(!dedup.accept(Some("m-3")));
        assert!(!dedup.accept(Some("m-new")));
    }

    /// 端到端：平台用**数字** `message_id` 重放同一条消息时，只应驱动一次回合。
    /// 此前的实现按字符串读取 id（`as_str()`），数字一律读成 None → 去重形同虚设。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn receiver_dedups_replayed_numeric_message_id() {
        struct ReplayApi {
            calls: Arc<std::sync::atomic::AtomicUsize>,
        }
        impl WechatApi for ReplayApi {
            unsupported_media_api!();
            fn get_qr_code(&self, _b: &str, _t: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn poll_qr_status(&self, _b: &str, _q: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "status": "wait" })) })
            }
            fn get_updates(
                &self,
                _b: &str,
                _t: &str,
                _buf: &str,
                _to: u64,
            ) -> BoxFuture<'_, Result<Value, String>> {
                // 前两次返回同一条（数字 id）消息，模拟 sync-buf 写盘失败后的队列重放；
                // 之后返回空并短睡，避免把长轮询循环变成忙循环。
                let n = self.calls.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    if n < 2 {
                        Ok(json!({
                            "ret": 0,
                            "get_updates_buf": "b1",
                            "msgs": [{
                                "from_user_id": "u-1",
                                "to_user_id": "bot-1",
                                "create_time_ms": 1,
                                "context_token": "ctx-1",
                                "message_id": 1790180059616i64,
                                "item_list": [{ "type": 1, "text_item": { "text": "hi" } }],
                            }],
                        }))
                    } else {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        Ok(json!({ "ret": 0, "get_updates_buf": "b1", "msgs": [] }))
                    }
                })
            }
            fn send_message(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
            fn get_config(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
            fn send_typing(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
            fn download_stream(
                &self,
                _url: &str,
                _timeout: Duration,
            ) -> BoxFuture<'_, Result<AttachmentStream, String>> {
                Box::pin(async { Err("本用例不涉及附件下载".to_string()) })
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        save_account(&root, "bot-1", "tok", DEFAULT_BASE_URL, "u-1");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let cancel = Arc::new(Notify::new());
        let mut bindings = HashMap::new();
        bindings.insert("bot-1".to_string(), "t-1".to_string());
        let handle = tokio::spawn(run_receiver(
            Arc::new(ReplayApi {
                calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            }),
            tx,
            root.clone(),
            "bot-1".into(),
            cancel.clone(),
            Arc::new(Mutex::new(bindings)),
            Arc::new(Mutex::new(HashMap::new())),
        ));

        // 第一条消息应当到达
        let first = loop {
            match rx.recv().await {
                Some(WechatEvent::Message { text, .. }) => break text,
                Some(_) => continue,
                None => panic!("事件通道提前关闭"),
            }
        };
        assert_eq!(first.as_deref(), Some("hi"));

        // 重放的第二条（同数字 id）应被去重拦下：在窗口内不应再出现 Message 事件
        let deadline = tokio::time::Instant::now() + Duration::from_millis(400);
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                break;
            }
            match tokio::time::timeout(left, rx.recv()).await {
                Ok(Some(WechatEvent::Message { text, .. })) => {
                    panic!("重复的数字 message_id 未被去重，收到第二条消息: {text:?}")
                }
                Ok(Some(_)) => continue,
                Ok(None) => panic!("事件通道提前关闭"),
                Err(_) => break, // 超时：窗口内没有第二条消息 → 去重生效
            }
        }

        cancel.notify_one();
        let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
    }

    #[test]
    fn send_failure_classification_is_retryable_only_for_transient() {
        // 确定性失败：重试只会立刻拿到同样错误，且拖长整条回复的发送时间
        assert!(!WechatSendFailure::Fatal("微信账号未连接".into()).is_retryable());
        assert!(!WechatSendFailure::Fatal("缺少回复上下文".into()).is_retryable());
        // 瞬时失败：值得短重试
        assert!(WechatSendFailure::Retryable("connection reset".into()).is_retryable());
        // 文案透传（用于日志与提示）
        assert_eq!(
            WechatSendFailure::Fatal("会话已过期".into()).message(),
            "会话已过期"
        );
        assert_eq!(
            WechatSendFailure::Retryable("timeout".into()).message(),
            "timeout"
        );
    }

    #[tokio::test]
    async fn send_text_returns_fatal_for_disconnected_account() {
        struct NoopApi;
        impl WechatApi for NoopApi {
            unsupported_media_api!();
            fn get_qr_code(&self, _b: &str, _t: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn poll_qr_status(&self, _b: &str, _q: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn get_updates(
                &self,
                _b: &str,
                _t: &str,
                _buf: &str,
                _to: u64,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn send_message(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                panic!("未连接账号不应发出 sendmessage 请求");
            }
            fn get_config(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn send_typing(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn download_stream(
                &self,
                _url: &str,
                _timeout: Duration,
            ) -> BoxFuture<'_, Result<AttachmentStream, String>> {
                Box::pin(async { Err("未实现".to_string()) })
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        save_account(&root, "bot-1", "tok", DEFAULT_BASE_URL, "u-1");
        let (tx, _rx) = mpsc::unbounded_channel();
        let client = WechatClient::with_api(root.clone(), tx, NoopApi);

        // 未连接 → Fatal（桥不重试）
        save_session_status(&root, "bot-1", "disconnected", None, None);
        let err = client.send_text("bot-1", "u-1", "hi").await.unwrap_err();
        assert!(!err.is_retryable(), "未连接属确定性失败，不应重试");

        // 会话过期 → Fatal
        save_session_status(&root, "bot-1", "session_expired", None, None);
        let err = client.send_text("bot-1", "u-1", "hi").await.unwrap_err();
        assert!(!err.is_retryable(), "会话过期属确定性失败，不应重试");

        // 已连接但缺 contextToken（24 小时窗口过期）→ Fatal
        save_session_status(&root, "bot-1", "connected", None, None);
        let err = client.send_text("bot-1", "u-1", "hi").await.unwrap_err();
        assert!(!err.is_retryable(), "缺回复上下文属确定性失败，不应重试");
    }

    #[tokio::test]
    async fn reply_context_prunes_expired_entries_on_write() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let now = now_ms();
        // 直接构造一份「一条已过期 + 一条仍有效」的表，模拟跨过 24 小时窗口后的磁盘状态
        let mut map = HashMap::new();
        map.insert(
            "u-stale".to_string(),
            json!({
                "peerId": "u-stale",
                "contextToken": "old",
                "lastInboundAt": 1,
                "expiresAt": now.saturating_sub(1),
            }),
        );
        map.insert(
            "u-live".to_string(),
            json!({
                "peerId": "u-live",
                "contextToken": "new",
                "lastInboundAt": now,
                "expiresAt": now + REPLY_WINDOW_MS,
            }),
        );
        save_reply_context(&root, "bot-1", &map);

        // 写盘后过期条目应被剔除，仍有效的条目保留
        let reloaded = load_reply_context(&root, "bot-1");
        assert!(!reloaded.contains_key("u-stale"), "过期条目应被剪掉");
        assert!(reloaded.contains_key("u-live"), "未过期条目应保留");
        assert_eq!(
            get_context_token(&root, "bot-1", "u-live").as_deref(),
            Some("new")
        );
        assert!(get_context_token(&root, "bot-1", "u-stale").is_none());
    }

    #[test]
    fn outbound_media_kind_maps_by_extension() {
        for p in ["a.png", "A.JPG", "b.webp", "c.BMP", "d.gif", "e.jpeg"] {
            assert_eq!(outbound_media_kind(p), OutboundMediaKind::Image, "{p}");
        }
        for p in ["a.mp4", "B.MOV", "c.mkv", "d.webm", "e.avi"] {
            assert_eq!(outbound_media_kind(p), OutboundMediaKind::Video, "{p}");
        }
        // 未识别扩展名、无扩展名、目录形态都按文件发送
        for p in ["a.pdf", "b.zip", "noext", "dir.d\\x", ""] {
            assert_eq!(outbound_media_kind(p), OutboundMediaKind::File, "{p}");
        }
    }

    #[test]
    fn media_item_shapes_match_reference() {
        let key = [7u8; 16];
        let img = build_media_item(OutboundMediaKind::Image, "DL", &key, 32, 20, "a.png");
        assert_eq!(img["type"], 2);
        assert_eq!(img["image_item"]["media"]["encrypt_query_param"], "DL");
        assert_eq!(img["image_item"]["media"]["encrypt_type"], 1);
        assert_eq!(
            img["image_item"]["media"]["aes_key"],
            BASE64.encode(hex_lower(&key)),
            "aes_key 必须是 base64(hex 串)，不是 base64(密钥原文)"
        );
        assert_eq!(img["image_item"]["mid_size"], 32, "图片用密文大小");

        let video = build_media_item(OutboundMediaKind::Video, "DL", &key, 32, 20, "a.mp4");
        assert_eq!(video["type"], 5);
        assert_eq!(video["video_item"]["video_size"], 32, "视频用密文大小");

        let file = build_media_item(OutboundMediaKind::File, "DL", &key, 32, 20, "报 告.pdf");
        assert_eq!(file["type"], 4);
        assert_eq!(file["file_item"]["file_name"], "报 告.pdf");
        assert_eq!(file["file_item"]["len"], "20", "文件用明文字符串长度");
        assert!(
            file["file_item"].get("mid_size").is_none(),
            "文件项不带图片/视频的大小字段"
        );
    }

    #[test]
    fn padded_size_matches_pkcs7_growth() {
        assert_eq!(padded_size(0), 16, "空文件补满一整块");
        assert_eq!(padded_size(1), 16);
        assert_eq!(padded_size(15), 16);
        assert_eq!(padded_size(16), 32, "恰好整块也要补满一块");
        assert_eq!(padded_size(17), 32);
        assert_eq!(padded_size(32), 48);
    }

    #[tokio::test]
    async fn encrypt_file_streams_and_round_trips() {
        use md5::Digest;

        let dir = tempfile::tempdir().unwrap();
        // 覆盖：空文件 / 恰好一块 / 非整块 / 跨 64KB 读取边界
        let cases: Vec<Vec<u8>> = vec![
            Vec::new(),
            vec![b'A'; 16],
            vec![b'B'; 17],
            (0..(64 * 1024 + 7)).map(|n| (n % 251) as u8).collect(),
        ];
        for (i, plain) in cases.into_iter().enumerate() {
            let src = dir.path().join(format!("src-{i}.bin"));
            let dst = dir.path().join(format!("dst-{i}.bin"));
            std::fs::write(&src, &plain).unwrap();
            let key = [i as u8 + 1; 16];

            let (raw, cipher_size, md5_hex) = encrypt_file_to(&src, &dst, &key).await.unwrap();
            assert_eq!(raw, plain.len() as u64);
            assert_eq!(cipher_size, padded_size(raw));

            let cipher = std::fs::read(&dst).unwrap();
            assert_eq!(
                cipher.len() as u64,
                cipher_size,
                "落盘密文长度必须与上报的 filesize 一致"
            );
            // 用生产侧的解密器解回（同时验证流式加密与入站解密口径一致）
            let mut decryptor = EcbStreamDecryptor::new(&hex_of(&key)).unwrap();
            let mut decrypted = decryptor.push(&cipher).unwrap();
            decrypted.extend(decryptor.finish().unwrap());
            assert_eq!(decrypted, plain, "解密应还原明文");

            let expect_md5: String = md5::Md5::digest(&plain)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            assert_eq!(md5_hex, expect_md5, "md5 应为明文摘要");
        }
    }

    #[tokio::test]
    async fn send_file_uploads_expected_params_and_sends_media_message() {
        struct UpApi {
            upload_body: Arc<std::sync::Mutex<Option<Value>>>,
            cipher: Arc<std::sync::Mutex<Vec<u8>>>,
            sent: Arc<std::sync::Mutex<Vec<Value>>>,
        }
        impl WechatApi for UpApi {
            fn get_qr_code(&self, _b: &str, _t: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn poll_qr_status(&self, _b: &str, _q: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn get_updates(
                &self,
                _b: &str,
                _t: &str,
                _buf: &str,
                _to: u64,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn send_message(
                &self,
                _b: &str,
                _t: &str,
                body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                self.sent.lock().unwrap().push(body);
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
            fn get_config(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn send_typing(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn get_upload_url(
                &self,
                _b: &str,
                _t: &str,
                body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                *self.upload_body.lock().unwrap() = Some(body);
                Box::pin(async { Ok(json!({ "ret": 0, "upload_param": "UP-1" })) })
            }
            fn upload_to_cdn(
                &self,
                url: &str,
                path: PathBuf,
                len: u64,
                _timeout: Duration,
            ) -> BoxFuture<'_, CdnUploadOutcome> {
                assert!(url.contains("/upload?encrypted_query_param=UP-1"), "{url}");
                assert!(url.contains("filekey="), "上传地址应带 filekey: {url}");
                let bytes = std::fs::read(&path).expect("临时密文文件应存在");
                assert_eq!(bytes.len() as u64, len, "Content-Length 应等于密文长度");
                *self.cipher.lock().unwrap() = bytes;
                Box::pin(async { CdnUploadOutcome::Ok("DL-1".to_string()) })
            }
            fn download_stream(
                &self,
                _url: &str,
                _timeout: Duration,
            ) -> BoxFuture<'_, Result<AttachmentStream, String>> {
                Box::pin(async { Err("本用例不涉及下载".to_string()) })
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        save_account(&root, "bot-1", "tok", DEFAULT_BASE_URL, "u-1");
        save_session_status(&root, "bot-1", "connected", None, None);
        set_context_token(&root, "bot-1", "u-1", "ctx-1", None);

        let plain = b"hello wechat media".to_vec();
        let src = dir.path().join("报告.png");
        std::fs::write(&src, &plain).unwrap();

        let upload_body = Arc::new(std::sync::Mutex::new(None));
        let cipher = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sent = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (tx, _rx) = mpsc::unbounded_channel();
        let client = WechatClient::with_api(
            root.clone(),
            tx,
            UpApi {
                upload_body: upload_body.clone(),
                cipher: cipher.clone(),
                sent: sent.clone(),
            },
        );
        client.send_file("bot-1", "u-1", &src).await.unwrap();

        // getuploadurl 请求体逐字段核对
        let body = upload_body
            .lock()
            .unwrap()
            .clone()
            .expect("应调用 getuploadurl");
        let filekey = body["filekey"].as_str().unwrap().to_string();
        assert_eq!(filekey.len(), 32, "filekey 为 16 字节 hex");
        assert_eq!(body["media_type"], 1, "png 应为 IMAGE(1)");
        assert_eq!(body["to_user_id"], "u-1");
        assert_eq!(body["rawsize"], plain.len() as u64);
        assert_eq!(body["filesize"], padded_size(plain.len() as u64));
        assert_eq!(body["no_need_thumb"], true, "跳过缩略图上传");
        let aes_hex = body["aeskey"].as_str().unwrap().to_string();
        assert_eq!(aes_hex.len(), 32, "aeskey 为 16 字节 hex");
        let expect_md5: String = {
            use md5::Digest;
            md5::Md5::digest(&plain)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect()
        };
        assert_eq!(body["rawfilemd5"], expect_md5);

        // 上传的密文应能用上报的密钥（hex）解回明文
        let uploaded = cipher.lock().unwrap().clone();
        let mut decryptor = EcbStreamDecryptor::new(&aes_hex).unwrap();
        let mut decrypted = decryptor.push(&uploaded).unwrap();
        decrypted.extend(decryptor.finish().unwrap());
        assert_eq!(decrypted, plain, "上传内容必须是该文件的密文");

        // 媒体消息 item 形状
        let msgs = sent.lock().unwrap().clone();
        assert_eq!(msgs.len(), 1, "应恰好发一条媒体消息");
        let msg = &msgs[0]["msg"];
        assert_eq!(msg["to_user_id"], "u-1");
        assert_eq!(msg["message_type"], 2);
        assert_eq!(msg["message_state"], 2);
        assert_eq!(msg["context_token"], "ctx-1");
        let item = &msg["item_list"][0];
        assert_eq!(item["type"], 2);
        // 媒体项填上传响应头 `x-encrypted-param`（DL-1，与官方实现一致）：
        // 真机实测这一项决定消息能否在微信里显示；上传参数 UP-1 虽能取回对象，
        // 但用它填消息会导致消息不显示。
        assert_eq!(
            item["image_item"]["media"]["encrypt_query_param"], "DL-1",
            "媒体项应使用上传响应头参数"
        );
        assert_eq!(item["image_item"]["media"]["encrypt_type"], 1);
        assert_eq!(
            item["image_item"]["media"]["aes_key"],
            BASE64.encode(aes_hex.as_bytes()),
            "item 里的 aes_key 应为 base64(hex 串)"
        );
        assert_eq!(
            item["image_item"]["mid_size"],
            padded_size(plain.len() as u64)
        );
    }

    /// 平台返回带 `taskid` 的完整上传地址时必须原样使用——自己用 `upload_param` 拼 URL 会丢
    /// taskid，上传虽返回 200，但接收端取不回对象（2026-09-24 真机实测：用完整地址上传后
    /// 接收端才能下载成功）。
    #[tokio::test]
    async fn send_file_prefers_upload_full_url() {
        const FULL_URL: &str =
            "https://novac2c.cdn.weixin.qq.com/c2c/upload?encrypted_query_param=P&filekey=F&taskid=T";

        struct FullUrlApi {
            posts: Arc<std::sync::Mutex<Vec<String>>>,
        }
        impl WechatApi for FullUrlApi {
            fn get_qr_code(&self, _b: &str, _t: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn poll_qr_status(&self, _b: &str, _q: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn get_updates(
                &self,
                _b: &str,
                _t: &str,
                _buf: &str,
                _to: u64,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn send_message(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
            fn get_config(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn send_typing(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn get_upload_url(
                &self,
                _b: &str,
                _t: &str,
                _body: Value,
            ) -> BoxFuture<'_, Result<Value, String>> {
                // 只给完整地址、不带 upload_param：自拼 URL 的分支会因此报错
                Box::pin(async { Ok(json!({ "ret": 0, "upload_full_url": FULL_URL })) })
            }
            fn upload_to_cdn(
                &self,
                url: &str,
                _path: PathBuf,
                _len: u64,
                _timeout: Duration,
            ) -> BoxFuture<'_, CdnUploadOutcome> {
                self.posts.lock().unwrap().push(url.to_string());
                Box::pin(async { CdnUploadOutcome::Ok("DL-1".to_string()) })
            }
            fn download_stream(
                &self,
                _url: &str,
                _timeout: Duration,
            ) -> BoxFuture<'_, Result<AttachmentStream, String>> {
                Box::pin(async { Err("本用例不涉及下载".to_string()) })
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        save_account(&root, "bot-1", "tok", DEFAULT_BASE_URL, "u-1");
        save_session_status(&root, "bot-1", "connected", None, None);
        set_context_token(&root, "bot-1", "u-1", "ctx-1", None);
        let src = dir.path().join("report.txt");
        std::fs::write(&src, b"hello").unwrap();

        let posts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (tx, _rx) = mpsc::unbounded_channel();
        let client = WechatClient::with_api(
            root.clone(),
            tx,
            FullUrlApi {
                posts: posts.clone(),
            },
        );
        client.send_file("bot-1", "u-1", &src).await.unwrap();

        assert_eq!(
            posts.lock().unwrap().clone(),
            vec![FULL_URL.to_string()],
            "应把 upload_full_url 原样作为上传地址"
        );
    }

    #[tokio::test]
    async fn send_file_rejects_invalid_paths_without_uploading() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        save_account(&root, "bot-1", "tok", DEFAULT_BASE_URL, "u-1");
        save_session_status(&root, "bot-1", "connected", None, None);
        set_context_token(&root, "bot-1", "u-1", "ctx-1", None);
        let (tx, _rx) = mpsc::unbounded_channel();
        let client = WechatClient::with_api(root.clone(), tx, MockApi::default());

        // 不存在
        let missing = dir.path().join("nope.png");
        let err = client
            .send_file("bot-1", "u-1", &missing)
            .await
            .unwrap_err();
        assert!(!err.is_retryable(), "路径不存在属确定性失败");

        // 目录不是文件
        let err = client
            .send_file("bot-1", "u-1", dir.path())
            .await
            .unwrap_err();
        assert!(!err.is_retryable(), "目录属确定性失败");

        // 未连接账号：本地前置校验即失败，不进入上传
        save_session_status(&root, "bot-1", "disconnected", None, None);
        let ok = dir.path().join("ok.png");
        std::fs::write(&ok, b"x").unwrap();
        let err = client.send_file("bot-1", "u-1", &ok).await.unwrap_err();
        assert!(!err.is_retryable(), "未连接属确定性失败");
    }
}
