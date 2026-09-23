//! Windows 系统通知（WinRT toast）：定时任务终态与 codex 错误提示共用一套实现。
//!
//! - 归属：`notification_app_id` 选择通知的 AppUserModelID——打包安装版用应用 AUMID，
//!   开发版（`target/debug` / `target/release`）回退 PowerShell 默认值以确保能显示；
//! - 激活：「打开会话」按钮与 `launch` 统一携带 [`OPEN_SESSION_SCHEME`] 协议串，
//!   进程内 WinRT `Activated` 事件与未打包应用的快捷方式二次启动两条通道都能
//!   解析出 thread id，再经 Tauri 事件通知前端聚焦窗口并打开该会话；
//! - 节流：同类错误短时间内重复发生（如响应流断线重连 1/5…5/5）只发一条，
//!   见 [`ToastThrottle`]。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};
#[cfg(windows)]
use windows::core::{IInspectable, Interface, HSTRING};
#[cfg(windows)]
use windows::Data::Xml::Dom::XmlDocument;
#[cfg(windows)]
use windows::Foundation::TypedEventHandler;
#[cfg(windows)]
use windows::UI::Notifications::{
    ToastActivatedEventArgs, ToastNotification, ToastNotificationManager,
};

use crate::codex::app_server::CodexServer;

/// 前端事件名：点击 toast 上的「打开会话」按钮后，后端通知前端聚焦窗口并打开绑定会话。
pub const OPEN_SESSION_EVENT: &str = "notification-open-session";

/// Windows 通知归属的应用 AppUserModelID（与 tauri.conf.json `identifier` 一致）。
/// 安装版快捷方式需携带该 AUMID，通知才会以应用图标归属显示；开发版回退 PowerShell。
const APP_USER_MODEL_ID: &str = "com.codexui.app";

/// 开发版（target/debug、target/release）回退 PowerShell 默认 AUMID，确保未注册
/// 快捷方式的场景下 toast 也能显示（其点击路由依赖系统快捷方式，本端不处理）。
#[cfg(windows)]
const DEV_AUMID: &str =
    "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe";

/// toast 前台激活携带的「打开会话」协议串前缀。安装版下点击按钮可能不经进程内
/// WinRT `Activated` 事件，而是由系统按 AUMID 经快捷方式二次启动本 exe，激活参数
/// 随命令行透传；统一用该协议串承载 thread id，进程内回调与命令行两条路径都能解析。
pub const OPEN_SESSION_SCHEME: &str = "codexui://open-session/";

/// toast 标题/正文单行化后的最大字符数（按字符计，UTF-8 安全）；超出截断并追加省略号。
pub const MAX_TITLE_CHARS: usize = 80;
pub const MAX_BODY_CHARS: usize = 200;

/// 相同正文的抑制窗口（秒）：同文反复上报（如响应流断线重连 `Reconnecting… 1/5`…`5/5`
/// 都映射成同一句「响应连接已断开，请重试」）只发一条。
pub const NOTIFY_THROTTLE_SECS: u64 = 10;

/// 同一会话回合的抑制窗口（秒）：只用于压制「`error` 通知 + `turn/completed=failed`」
/// 这对同一次失败的近同时双报（实测相隔约 20ms）。窗口刻意取短——同一回合内稍后出现的
/// **不同**错误（如重连失败之后真正的 401/模型不可用）必须照常通知。
pub const TURN_THROTTLE_SECS: u64 = 2;

/// 从二次启动的命令行参数中解析「打开会话」的 thread id：扫描是否存在
/// [`OPEN_SESSION_SCHEME`] 协议串，命中则取其后的 id（截断可能跟随的空白/引号）。
/// 仅按协议串匹配，避免把 argv[0]（exe 路径）等误判为 thread id。
pub fn parse_activation_thread(args: &[String]) -> Option<String> {
    for a in args {
        if let Some(pos) = a.find(OPEN_SESSION_SCHEME) {
            let rest = &a[pos + OPEN_SESSION_SCHEME.len()..];
            let id = rest
                .split(|c: char| c.is_whitespace() || c == '"' || c == '\'')
                .next()
                .unwrap_or("")
                .trim();
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }
    }
    None
}

/// 选择 Windows toast 的 AUMID：从 `target/debug`、`target/release` 运行视为开发版，
/// 回退 PowerShell 默认（否则未注册快捷方式时 toast 不显示）；打包安装版用应用 AUMID。
pub fn notification_app_id() -> &'static str {
    if let Ok(exe) = std::env::current_exe() {
        let dir = exe
            .parent()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        if dir.ends_with("/target/debug") || dir.ends_with("/target/release") {
            return notification_dev_app_id();
        }
    }
    APP_USER_MODEL_ID
}

/// 开发版（target/debug、target/release）回退 PowerShell 默认 AUMID，确保 toast 可显示。
#[cfg(windows)]
fn notification_dev_app_id() -> &'static str {
    DEV_AUMID
}

/// 非 Windows 平台占位（项目仅面向 Windows，此分支不会在目标构建中用到）。
#[cfg(not(windows))]
fn notification_dev_app_id() -> &'static str {
    APP_USER_MODEL_ID
}

/// 让本进程显式声明 Windows AppUserModelID：安装版下 toast 按钮的前台激活因此会被
/// Windows 投递到「当前已运行进程」的 `ToastNotification.Activated` 事件（`on_activated`），
/// 从而聚焦窗口并打开绑定会话；否则 Windows 会经安装版快捷方式尝试拉起新实例，
/// 被单实例插件截获后仅「聚焦窗口」，会话打开逻辑永远不执行。
///
/// 仅安装版（exe 不在 `target/debug`、`target/release`）设置；开发版 toast 仍回退
/// PowerShell AUMID 仅为「能显示」，其点击路由本就不在本端，故不为其设置进程 AUMID，
/// 避免把本进程伪装成 PowerShell。
pub fn ensure_process_app_user_model_id() {
    #[cfg(windows)]
    {
        if notification_app_id() == APP_USER_MODEL_ID {
            // 手动构造以 NUL 结尾的 UTF-16 宽字符串，避免 windows / windows-core
            // 两个版本不一致导致 HSTRING 无法传入 Param<PCWSTR>；进程 AUMID 声明一次，
            // 既是任务栏分组归属，也是 toast 激活路由的依据，失败仅记日志不影响功能。
            let mut wide: Vec<u16> = APP_USER_MODEL_ID.encode_utf16().collect();
            wide.push(0);
            unsafe {
                let _ = windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(
                    windows::core::PCWSTR::from_raw(wide.as_ptr()),
                );
            }
        }
    }
    #[cfg(not(windows))]
    {
        // 项目仅面向 Windows，此分支不会在目标构建中用到。
    }
}

/// 常驻的 `ToastNotification` 队列：WinRT 要求在 toast 停留期间其对象存活，否则点击
/// 「打开会话」按钮触发的 `Activated` 事件不再回调（对象 Drop 后事件回调失效）。
/// 仅按 FIFO 淘汰最旧的（早已被系统关闭的 toast），避免长期运行无限增长。
#[cfg(windows)]
static ALIVE_TOASTS: Mutex<VecDeque<ToastNotification>> = Mutex::new(VecDeque::new());

/// 常驻队列上限：超过则弹出最旧 toast 并释放其引用。通常同一时刻仅 0~2 条 toast。
#[cfg(windows)]
const MAX_ALIVE_TOASTS: usize = 128;

/// 用 `windows` crate 直接发送 WinRT toast，并把 `ToastNotification` 常驻以接收点击事件。
/// 在带消息泵的主线程调用。点击「打开会话」有两条投递通道，均已埋日志：
/// 1) 进程内 WinRT `Activated` 事件（toast 对象常驻才有效）；
/// 2) 未打包 Win32 应用的前台激活：系统按 AUMID 经快捷方式二次启动本 exe，
///    激活串（`launch` / 按钮 `arguments`）随命令行透传，由单实例回调
///    [`parse_activation_thread`] 解析后经事件通知前端。
///
/// `source` 为通知来源（`scheduled-task` / `codex-error`），仅用于会话日志区分。
#[cfg(windows)]
pub fn show_winrt_toast(
    app: &AppHandle,
    server: &Arc<CodexServer>,
    app_id: &str,
    title: &str,
    body: &str,
    thread_id: &str,
    source: &str,
) -> Result<(), String> {
    let activation = format!("{}{}", OPEN_SESSION_SCHEME, thread_id);
    let xml = format!(
        r#"<toast duration="short" launch="{}"><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual><actions><action content="打开会话" arguments="{}" activationType="foreground"/></actions></toast>"#,
        xml_escape(&activation),
        xml_escape(title),
        xml_escape(body),
        xml_escape(&activation),
    );
    let doc = XmlDocument::new().map_err(|e| e.to_string())?;
    doc.LoadXml(&HSTRING::from(xml))
        .map_err(|e| e.to_string())?;
    let toast = ToastNotification::CreateToastNotification(&doc).map_err(|e| e.to_string())?;

    let app_for_cb = app.clone();
    let server_for_cb = server.clone();
    let source_for_cb = source.to_string();
    let activated =
        TypedEventHandler::<ToastNotification, IInspectable>::new(move |_sender, insp| {
            // 通道 1：进程内 WinRT 激活事件。能进到这里说明 toast 对象常驻生效。
            // arguments / launch 均为协议串，需解析出 thread id 再通知前端。
            let raw = insp
                .as_ref()
                .and_then(|i| i.cast::<ToastActivatedEventArgs>().ok())
                .and_then(|args| args.Arguments().ok())
                .map(|a| a.to_string())
                .unwrap_or_default();
            server_for_cb.session_log(
                "info".into(),
                None,
                "toast-activated".into(),
                Some(format!("source={source_for_cb} raw={raw}")).into(),
            );
            if let Some(thread) = parse_activation_thread(&[raw]) {
                server_for_cb.session_log(
                    "info".into(),
                    None,
                    "toast-open-session".into(),
                    Some(format!(
                        "via=winrt-activated source={source_for_cb} thread={thread}"
                    ))
                    .into(),
                );
                let _ = app_for_cb.emit(OPEN_SESSION_EVENT, &thread);
            }
            Ok(())
        });
    toast.Activated(&activated).map_err(|e| e.to_string())?;

    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id))
        .map_err(|e| e.to_string())?;
    notifier.Show(&toast).map_err(|e| e.to_string())?;
    server.session_log(
        "info".into(),
        None,
        "toast-shown".into(),
        Some(format!(
            "source={source} app_id={app_id} thread={thread_id}"
        ))
        .into(),
    );

    // 关键：把 ToastNotification 存入常驻队列，防止 Drop 后点击回调失效。
    if let Ok(mut alive) = ALIVE_TOASTS.lock() {
        alive.push_back(toast);
        if alive.len() > MAX_ALIVE_TOASTS {
            alive.pop_front();
        }
    }
    Ok(())
}

/// XML 转义（toast 标题/正文可能来自用户任务名与执行结果，需保证 XML 合法）。
#[cfg(windows)]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// 通知标题/正文清洗：把换行、制表与连续空白压成单空格并去首尾空白，再按字符数截断
/// （超出追加 `…`）。toast 正文只有两行文本节点，多行原文会挤在一起，压成单行更易读。
pub fn sanitize_toast_text(text: &str, max_chars: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max_chars {
        return flat;
    }
    let mut out: String = flat.chars().take(max_chars).collect();
    out.push('…');
    out
}

/// 一条节流键及其抑制窗口。
#[derive(Debug, Clone)]
pub struct ThrottleEntry {
    pub key: String,
    pub window: Duration,
}

/// 节流器：窗口期内已放行过的键再次出现时拒绝。
///
/// 用于抑制两类重复：codex 的 `error` 通知与 `turn/completed=failed` 对同一次失败
/// 各报一次（按会话回合键，短窗口）；以及响应流断线重连（`Reconnecting… 1/5`…`5/5`）
/// 这类同文反复上报（按正文键，长窗口）。
#[derive(Default)]
pub struct ToastThrottle {
    /// 最近放行的键 → (放行时刻, 抑制窗口)；每个键在各自窗口期内只放行一次。
    recent: HashMap<String, (Instant, Duration)>,
}

impl ToastThrottle {
    /// 本次是否放行：任一条目命中窗口期内的记录则拒绝，否则记录全部条目并放行。
    /// 顺带清理过期记录，长时间运行不会无限增长。
    pub fn allow(&mut self, entries: &[ThrottleEntry], now: Instant) -> bool {
        self.recent
            .retain(|_, (at, window)| now.saturating_duration_since(*at) < *window);
        if entries.iter().any(|e| self.recent.contains_key(&e.key)) {
            return false;
        }
        for e in entries {
            self.recent.insert(e.key.clone(), (now, e.window));
        }
        true
    }
}

/// 节流条目：`text:<归一化正文>`（同文去重，10 秒）+ 回合存在时追加
/// `turn:<会话>:<回合>`（同一次失败的双报去重，2 秒）。
pub fn throttle_entries(thread_id: &str, turn_id: Option<&str>, body: &str) -> Vec<ThrottleEntry> {
    let normalized = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut entries = vec![ThrottleEntry {
        key: format!("text:{}", normalized.to_lowercase()),
        window: Duration::from_secs(NOTIFY_THROTTLE_SECS),
    }];
    if let Some(turn) = turn_id.map(str::trim).filter(|t| !t.is_empty()) {
        entries.push(ThrottleEntry {
            key: format!("turn:{thread_id}:{turn}"),
            window: Duration::from_secs(TURN_THROTTLE_SECS),
        });
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn xml_escape_escapes_special_chars() {
        assert_eq!(
            xml_escape("a&b<c>d\"e'f"),
            "a&amp;b&lt;c&gt;d&quot;e&apos;f"
        );
        assert_eq!(xml_escape("无特殊字符"), "无特殊字符");
    }

    #[test]
    fn parse_activation_thread_scans_scheme() {
        assert_eq!(
            parse_activation_thread(&[
                "C:\\app\\codex-ui.exe".into(),
                "codexui://open-session/task-thread-123".into(),
            ]),
            Some("task-thread-123".into()),
        );
        // 命中后截断紧随其后的引号/空白（激活器可能包裹或拼接其它 token）
        assert_eq!(
            parse_activation_thread(&["codexui://open-session/abc\"extra".into()]),
            Some("abc".into()),
        );
        // 未命中协议串
        assert_eq!(
            parse_activation_thread(&["-AppUserModelId".into(), "codex-ui".into()]),
            None,
        );
    }

    #[test]
    fn sanitize_toast_text_flattens_whitespace() {
        assert_eq!(
            sanitize_toast_text("第一行\n第二行\t尾部  ", MAX_BODY_CHARS),
            "第一行 第二行 尾部"
        );
        assert_eq!(sanitize_toast_text("   ", MAX_BODY_CHARS), "");
    }

    #[test]
    fn sanitize_toast_text_truncates_by_chars() {
        let long = "错".repeat(MAX_BODY_CHARS + 5);
        let out = sanitize_toast_text(&long, MAX_BODY_CHARS);
        assert_eq!(out.chars().count(), MAX_BODY_CHARS + 1);
        assert!(out.ends_with('…'));
        // 未超长时原样返回，不加省略号
        assert_eq!(sanitize_toast_text("短", MAX_BODY_CHARS), "短");
        assert_eq!(sanitize_toast_text("标题", MAX_TITLE_CHARS), "标题");
    }

    #[test]
    fn throttle_entries_include_text_and_turn_windows() {
        let entries = throttle_entries("t1", Some("turn1"), "响应连接已断开，请重试");
        assert_eq!(entries.len(), 2);
        assert!(entries[0].key.starts_with("text:"));
        assert_eq!(entries[0].window, Duration::from_secs(NOTIFY_THROTTLE_SECS));
        assert_eq!(entries[1].key, "turn:t1:turn1");
        assert_eq!(entries[1].window, Duration::from_secs(TURN_THROTTLE_SECS));
        // 无回合 id 时只有正文条目；空白与大小写归一化
        let entries = throttle_entries("t1", Some("  "), "Boom  X");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "text:boom x");
    }

    #[test]
    fn throttle_suppresses_same_text_within_window() {
        let mut throttle = ToastThrottle::default();
        let now = Instant::now();
        let entries = throttle_entries("t1", None, "响应连接已断开，请重试");
        assert!(throttle.allow(&entries, now));
        // 窗口内重复：同文只发一条
        assert!(!throttle.allow(&entries, now + Duration::from_secs(3)));
        // 不同正文仍放行
        let other = throttle_entries("t1", None, "认证失效，请重新登录");
        assert!(throttle.allow(&other, now + Duration::from_secs(4)));
        // 超出窗口后同文再次放行
        assert!(throttle.allow(
            &entries,
            now + Duration::from_secs(NOTIFY_THROTTLE_SECS + 1)
        ));
    }

    #[test]
    fn throttle_suppresses_turn_pair_but_not_later_distinct_error() {
        let mut throttle = ToastThrottle::default();
        let now = Instant::now();
        // codex error 通知与 turn/completed=failed 对同一回合的两种文案（相隔约 20ms）
        let first = throttle_entries("t1", Some("turn1"), "响应连接已断开，请重试");
        let second = throttle_entries("t1", Some("turn1"), "服务内部错误");
        assert!(throttle.allow(&first, now));
        assert!(!throttle.allow(&second, now + Duration::from_millis(20)));
        // 同一回合内稍后出现的**不同**错误（重连失败 → 最终认证失败）仍要通知
        assert!(throttle.allow(&second, now + Duration::from_secs(5)));
        // 同一回合 id 但换了会话、正文也不同：属于另一次失败，放行
        let other_thread = throttle_entries("t2", Some("turn1"), "模型不可用");
        assert!(throttle.allow(&other_thread, now + Duration::from_secs(5)));
    }
}
