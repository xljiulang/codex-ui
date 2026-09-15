//! 微信 ilink bot 协议客户端（纯 Rust，替代 Node sidecar）。
//!
//! 实现文本收发链路的完整协议：二维码登录、getUpdates 长轮询、sendmessage、
//! contextToken 管理、账号/会话/同步缓冲/回复上下文的文件存储；入站图片按微信 CDN
//! 协议下载并 AES-128-ECB 解密后落盘（其它媒体仍忽略，媒体上传不在范围内）。
//! 事件经 mpsc 推送给桥（WechatEvent），语义对齐原 sidecar 的 stdio 事件。

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
    let Some(list) = item_list.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in list {
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
fn extract_media_refs(item_list: &Value, item_type: i64, item_key: &str, max: usize) -> Vec<InboundFile> {
    let Some(list) = item_list.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in list {
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

/// item_list 中可下载图片项的数量（与 `extract_message_images` 的口径一致，用于判断是否溢出上限）。
fn count_image_items(item_list: Option<&Value>) -> usize {
    item_list
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter(|i| {
                    i.get("type").and_then(|v| v.as_i64()) == Some(2)
                        && i.pointer("/image_item/media/encrypt_query_param")
                            .and_then(|v| v.as_str())
                            .is_some_and(|s| !s.trim().is_empty())
                })
                .count()
        })
        .unwrap_or(0)
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

/// 附件下载流：按块产出原始字节（`Err` 表示读取中断）；解密与落盘在调用方流式完成。
pub type AttachmentStream = Pin<Box<dyn futures_util::Stream<Item = Result<Vec<u8>, String>> + Send>>;

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
    /// 打开附件下载流（入站附件走微信 CDN）：按块产出原始字节，解密由调用方流式完成。
    /// `timeout` 为整次请求（含读取响应体）的时长上限。
    fn download_stream(&self, url: &str, timeout: Duration)
        -> BoxFuture<'_, Result<AttachmentStream, String>>;
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
            let stream = resp
                .bytes_stream()
                .map(|r| r.map(|b| b.to_vec()).map_err(|e| format!("附件下载失败: {e}")));
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
    Accounts(Vec<Value>),
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
        let bound_threads = self.bound_threads.clone();
        let cancel = {
            let map = self.receivers.lock().await;
            map.get(&account_id).cloned().unwrap()
        };
        tokio::spawn(async move {
            run_receiver(api, tx, root, account_id, cancel, bound_threads).await;
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
    bound_threads: Arc<Mutex<HashMap<String, String>>>,
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

async fn sleep_or_cancel(cancel: &Notify, dur: Duration) {
    tokio::select! {
        _ = cancel.notified() => {}
        _ = tokio::time::sleep(dur) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn sanitize_file_name_cleans_and_limits() {
        assert_eq!(sanitize_file_name("C:\\tmp\\报表.xlsx"), "报表.xlsx");
        assert_eq!(sanitize_file_name("/etc/hosts"), "hosts");
        assert_eq!(sanitize_file_name("a<b>c:d\"e|f?g*h.txt"), "a_b_c_d_e_f_g_h.txt");
        assert_eq!(sanitize_file_name("  ..name..  "), "name");
        assert_eq!(sanitize_file_name("a\u{7}b.txt"), "a_b.txt", "控制字符应替换");
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
        assert_eq!(parse_aes_key(&BASE64.encode(hex_key.as_bytes())).unwrap(), key);
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
        assert!(name.starts_with("wechat-") && name.ends_with(".png"), "{name}");
        assert!(
            resolve_final_name(AttachmentKind::Image, b"not an image", None, now).is_err(),
            "魔数识别不出应报错"
        );
        assert_eq!(
            resolve_final_name(AttachmentKind::File, b"x", Some("C:\\tmp\\预算表.xlsx"), now).unwrap(),
            "预算表.xlsx"
        );
        assert_eq!(
            resolve_final_name(AttachmentKind::File, b"x", None, now).unwrap(),
            "attachment"
        );
        let video = resolve_final_name(AttachmentKind::Video, b"x", Some("ignored.mp4"), now).unwrap();
        assert!(video.starts_with("wechat-video-") && video.ends_with(".mp4"), "{video}");
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
        assert!(name.starts_with("预算表-") && name.ends_with(".xlsx"), "{name}");
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
        fn get_qr_code(&self, _b: &str, _t: &str) -> BoxFuture<'_, Result<Value, String>> {
            Box::pin(async { Ok(json!({})) })
        }
        fn poll_qr_status(&self, _b: &str, _q: &str) -> BoxFuture<'_, Result<Value, String>> {
            Box::pin(async { Ok(json!({ "status": "wait" })) })
        }
        fn get_updates(&self, _b: &str, _t: &str, _buf: &str, _to: u64) -> BoxFuture<'_, Result<Value, String>> {
            Box::pin(async { Ok(json!({ "ret": 0, "msgs": [] })) })
        }
        fn send_message(&self, _b: &str, _t: &str, _body: Value) -> BoxFuture<'_, Result<Value, String>> {
            Box::pin(async { Ok(json!({ "ret": 0 })) })
        }
        fn get_config(&self, _b: &str, _t: &str, _body: Value) -> BoxFuture<'_, Result<Value, String>> {
            Box::pin(async { Ok(json!({ "ret": 0 })) })
        }
        fn send_typing(&self, _b: &str, _t: &str, _body: Value) -> BoxFuture<'_, Result<Value, String>> {
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
                    Ok(Box::pin(futures_util::stream::pending::<Result<Vec<u8>, String>>())
                        as AttachmentStream)
                });
            }
            let items: Vec<Result<Vec<u8>, String>> = self.chunks.iter().cloned().map(Ok).collect();
            Box::pin(async move { Ok(Box::pin(futures_util::stream::iter(items)) as AttachmentStream) })
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
        let session_dir = dir.path().join("wechannel-data").join(MEDIA_DIR_NAME).join(thread);
        assert_eq!(file.parent().unwrap(), session_dir, "应落在 media/<会话id>/");
        let name = file.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with("wechat-") && name.ends_with(".png"), "{name}");
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
            fn get_qr_code(&self, _b: &str, _t: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn poll_qr_status(&self, _b: &str, _q: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "status": "wait" })) })
            }
            fn get_updates(&self, _b: &str, _t: &str, buf: &str, _to: u64) -> BoxFuture<'_, Result<Value, String>> {
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
            fn send_message(&self, _b: &str, _t: &str, _body: Value) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
            fn get_config(&self, _b: &str, _t: &str, _body: Value) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
            fn send_typing(&self, _b: &str, _t: &str, _body: Value) -> BoxFuture<'_, Result<Value, String>> {
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
                    Ok(Box::pin(futures_util::stream::iter(vec![Ok(head), Ok(tail)])) as AttachmentStream)
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
        ciphers.insert("PARAM-VIDEO".to_string(), encrypt_aes_ecb(&video_bytes, &key));
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        save_account(&root, "bot-1", "tok", DEFAULT_BASE_URL, "u-1");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let cancel = Arc::new(Notify::new());
        // 账号已绑定会话：附件应落到 media/<会话id>/ 下。
        let thread_id = "01a0a2c4-2702-7bf0-9f61-9f280145e222";
        let mut bindings = HashMap::new();
        bindings.insert("bot-1".to_string(), thread_id.to_string());
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
        ));
        let got = loop {
            match rx.recv().await {
                Some(WechatEvent::Message { text, images, files, media_error, .. }) => {
                    break (text, images, files, media_error)
                }
                Some(_) => continue,
                None => panic!("事件通道提前关闭"),
            }
        };
        cancel.notify_one();
        let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;

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
            fn get_qr_code(&self, _b: &str, _t: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn poll_qr_status(&self, _b: &str, _q: &str) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "status": "wait" })) })
            }
            fn get_updates(&self, _b: &str, _t: &str, buf: &str, _to: u64) -> BoxFuture<'_, Result<Value, String>> {
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
            fn send_message(&self, _b: &str, _t: &str, _body: Value) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
            fn get_config(&self, _b: &str, _t: &str, _body: Value) -> BoxFuture<'_, Result<Value, String>> {
                Box::pin(async { Ok(json!({ "ret": 0 })) })
            }
            fn send_typing(&self, _b: &str, _t: &str, _body: Value) -> BoxFuture<'_, Result<Value, String>> {
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
        ));
        let got = loop {
            match rx.recv().await {
                Some(WechatEvent::Message { text, images, files, media_error, .. }) => {
                    break (text, images, files, media_error)
                }
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
        run_receiver(
            api,
            tx,
            root.clone(),
            "bot-1".into(),
            cancel,
            Arc::new(Mutex::new(HashMap::new())),
        )
        .await;
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
