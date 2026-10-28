use serde::Serialize;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::sync::Arc;
#[cfg(windows)]
use tauri::{AppHandle, Manager};
#[cfg(windows)]
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2ProcessFailedEventArgs2, ICoreWebView2ProcessFailedEventArgs3,
    COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_PPAPI_BROKER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_PPAPI_PLUGIN_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
    COREWEBVIEW2_PROCESS_FAILED_KIND_SANDBOX_HELPER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_UNKNOWN_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_UTILITY_PROCESS_EXITED, COREWEBVIEW2_PROCESS_FAILED_REASON,
    COREWEBVIEW2_PROCESS_FAILED_REASON_CRASHED, COREWEBVIEW2_PROCESS_FAILED_REASON_LAUNCH_FAILED,
    COREWEBVIEW2_PROCESS_FAILED_REASON_OUT_OF_MEMORY,
    COREWEBVIEW2_PROCESS_FAILED_REASON_PROFILE_DELETED,
    COREWEBVIEW2_PROCESS_FAILED_REASON_TERMINATED, COREWEBVIEW2_PROCESS_FAILED_REASON_UNRESPONSIVE,
};
#[cfg(windows)]
use webview2_com::ProcessFailedEventHandler;
#[cfg(windows)]
use windows::Win32::System::Com::CoTaskMemFree;
#[cfg(windows)]
use windows_core::{Interface, HSTRING, PWSTR};

const REPEAT_FAILURE_WINDOW: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FailureInfo {
    pub failure_kind: String,
    pub exit_code: Option<i32>,
    pub reason: String,
    pub process_description: String,
    pub failure_module: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecoveryInfo {
    pub recovered: bool,
    pub failure_kind: String,
    pub exit_code: Option<i32>,
    pub happened_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryAction {
    Reload,
    Fallback,
    LogOnly,
}

#[derive(Debug, Default)]
pub(crate) struct RecoveryTracker {
    last_failure: Option<Instant>,
    pending_info: Option<RecoveryInfo>,
    unhealthy: bool,
    fallback_active: bool,
}

impl RecoveryTracker {
    pub(crate) fn record_failure(
        &mut self,
        now: Instant,
        happened_at_ms: i64,
        failure: FailureInfo,
    ) -> RecoveryAction {
        if failure.failure_kind == "browserProcessExited" {
            self.unhealthy = true;
            self.fallback_active = true;
            self.pending_info = Some(RecoveryInfo {
                recovered: true,
                failure_kind: failure.failure_kind,
                exit_code: failure.exit_code,
                happened_at_ms,
            });
            return RecoveryAction::Fallback;
        }
        if !matches!(
            failure.failure_kind.as_str(),
            "renderProcessExited" | "renderProcessUnresponsive" | "frameRenderProcessExited"
        ) {
            return RecoveryAction::LogOnly;
        }

        let repeated = self
            .last_failure
            .is_some_and(|last| now.duration_since(last) < REPEAT_FAILURE_WINDOW);
        self.last_failure = Some(now);
        self.unhealthy = true;
        self.fallback_active = repeated;
        self.pending_info = Some(RecoveryInfo {
            recovered: true,
            failure_kind: failure.failure_kind,
            exit_code: failure.exit_code,
            happened_at_ms,
        });
        if repeated {
            RecoveryAction::Fallback
        } else {
            RecoveryAction::Reload
        }
    }

    pub(crate) fn take_info(&mut self) -> Option<RecoveryInfo> {
        self.pending_info.take()
    }

    pub(crate) fn mark_page_loaded(&mut self) {
        if !self.fallback_active {
            self.unhealthy = false;
        }
    }

    pub(crate) fn is_unhealthy(&self) -> bool {
        self.unhealthy
    }
}

fn tracker() -> &'static Mutex<RecoveryTracker> {
    static TRACKER: OnceLock<Mutex<RecoveryTracker>> = OnceLock::new();
    TRACKER.get_or_init(|| Mutex::new(RecoveryTracker::default()))
}

pub(crate) fn record_failure(happened_at_ms: i64, failure: FailureInfo) -> RecoveryAction {
    tracker()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .record_failure(Instant::now(), happened_at_ms, failure)
}

pub(crate) fn take_info() -> Option<RecoveryInfo> {
    tracker()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take_info()
}

pub(crate) fn mark_page_loaded() {
    tracker()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .mark_page_loaded();
}

pub(crate) fn is_unhealthy() -> bool {
    tracker()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .is_unhealthy()
}

#[cfg(windows)]
const FALLBACK_HTML: &str = r#"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Codex UI · 界面恢复</title>
<style>
:root{color-scheme:dark;font-family:"Microsoft YaHei UI",sans-serif;background:#0d1420;color:#e7edf7}
body{display:grid;place-items:center;min-height:100vh;margin:0;background:radial-gradient(circle at 30% 20%,#1e477f55,transparent 45%),#0d1420}
main{width:min(560px,calc(100vw - 56px));padding:28px;border:1px solid #ffffff22;border-radius:16px;background:#111c2bdd;box-shadow:0 20px 60px #0008}
h1{font-size:20px;margin:0 0 14px}p{line-height:1.75;color:#b9c5d6;margin:8px 0}.hint{margin-top:18px;color:#8fa1b8;font-size:13px}
</style>
</head>
<body><main>
<h1>界面渲染进程连续异常</h1>
<p>为避免重复崩溃，Codex UI 已停止自动恢复。</p>
<p>你的会话与命令输出仍保存在 Codex 本地数据中，没有丢失。</p>
<p class="hint">请从系统托盘菜单选择“退出”，然后重新启动应用。</p>
</main></body></html>"#;

#[cfg(windows)]
fn process_kind_name(kind: COREWEBVIEW2_PROCESS_FAILED_KIND) -> &'static str {
    match kind {
        COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED => "browserProcessExited",
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED => "renderProcessExited",
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE => "renderProcessUnresponsive",
        COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED => "frameRenderProcessExited",
        COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED => "gpuProcessExited",
        COREWEBVIEW2_PROCESS_FAILED_KIND_PPAPI_BROKER_PROCESS_EXITED => "ppapiBrokerProcessExited",
        COREWEBVIEW2_PROCESS_FAILED_KIND_PPAPI_PLUGIN_PROCESS_EXITED => "ppapiPluginProcessExited",
        COREWEBVIEW2_PROCESS_FAILED_KIND_SANDBOX_HELPER_PROCESS_EXITED => {
            "sandboxHelperProcessExited"
        }
        COREWEBVIEW2_PROCESS_FAILED_KIND_UTILITY_PROCESS_EXITED => "utilityProcessExited",
        COREWEBVIEW2_PROCESS_FAILED_KIND_UNKNOWN_PROCESS_EXITED => "unknownProcessExited",
        _ => "unknownProcessFailed",
    }
}

#[cfg(windows)]
fn failure_reason_name(reason: COREWEBVIEW2_PROCESS_FAILED_REASON) -> &'static str {
    match reason {
        COREWEBVIEW2_PROCESS_FAILED_REASON_UNRESPONSIVE => "unresponsive",
        COREWEBVIEW2_PROCESS_FAILED_REASON_TERMINATED => "terminated",
        COREWEBVIEW2_PROCESS_FAILED_REASON_CRASHED => "crashed",
        COREWEBVIEW2_PROCESS_FAILED_REASON_LAUNCH_FAILED => "launchFailed",
        COREWEBVIEW2_PROCESS_FAILED_REASON_OUT_OF_MEMORY => "outOfMemory",
        COREWEBVIEW2_PROCESS_FAILED_REASON_PROFILE_DELETED => "profileDeleted",
        _ => "unexpected",
    }
}

#[cfg(windows)]
fn take_pwstr(value: PWSTR) -> String {
    if value.is_null() {
        return String::new();
    }
    let text = unsafe { value.to_string().unwrap_or_default() };
    unsafe { CoTaskMemFree(Some(value.as_ptr().cast())) };
    text
}

#[cfg(windows)]
fn log_failure(
    app: &AppHandle,
    failure: &FailureInfo,
    action: RecoveryAction,
    happened_at_ms: i64,
) {
    let detail = format!(
        "failureKind={} reason={} exitCode={:?} process={} module={} action={action:?} atMs={happened_at_ms}",
        failure.failure_kind,
        failure.reason,
        failure.exit_code,
        failure.process_description,
        failure.failure_module,
    );
    if let Some(server) = app.try_state::<Arc<crate::codex::app_server::CodexServer>>() {
        server.session_log(
            "error".to_string(),
            None,
            "webview-process-failed".to_string(),
            Some(detail),
        );
    }
}

#[cfg(windows)]
pub(crate) fn install(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let app_for_event = app.clone();
    let _ = window.with_webview(move |webview| {
        let controller = webview.controller();
        let handler = ProcessFailedEventHandler::create(Box::new(move |sender, args| {
            let Some(args) = args else {
                return Ok(());
            };
            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND_UNKNOWN_PROCESS_EXITED;
            let _ = unsafe { args.ProcessFailedKind(&mut kind) };
            let mut reason = COREWEBVIEW2_PROCESS_FAILED_REASON(0);
            let mut exit_code = None;
            let mut process_description = String::new();
            if let Ok(args2) = args.cast::<ICoreWebView2ProcessFailedEventArgs2>() {
                let mut value = 0;
                let _ = unsafe { args2.Reason(&mut reason) };
                let _ = unsafe { args2.ExitCode(&mut value) };
                exit_code = Some(value);
                let mut description = PWSTR::null();
                if unsafe { args2.ProcessDescription(&mut description) }.is_ok() {
                    process_description = take_pwstr(description);
                }
            }
            let failure_module = args
                .cast::<ICoreWebView2ProcessFailedEventArgs3>()
                .ok()
                .and_then(|args3| {
                    let mut value = PWSTR::null();
                    unsafe { args3.FailureSourceModulePath(&mut value) }
                        .ok()
                        .map(|_| take_pwstr(value))
                })
                .unwrap_or_default();
            let failure_kind = process_kind_name(kind).to_string();
            let failure = FailureInfo {
                process_description: if process_description.is_empty() {
                    failure_kind.clone()
                } else {
                    process_description
                },
                failure_kind,
                exit_code,
                reason: failure_reason_name(reason).to_string(),
                failure_module,
            };
            let happened_at_ms = chrono::Local::now().timestamp_millis();
            let action = record_failure(happened_at_ms, failure.clone());
            log_failure(&app_for_event, &failure, action, happened_at_ms);
            match action {
                RecoveryAction::Reload => {
                    if let Some(window) = app_for_event.get_webview_window("main") {
                        let _ = window.reload();
                    }
                }
                RecoveryAction::Fallback => {
                    if let Some(sender) = sender {
                        let html = HSTRING::from(FALLBACK_HTML);
                        let _ = unsafe { sender.NavigateToString(&html) };
                    }
                }
                RecoveryAction::LogOnly => {}
            }
            Ok(())
        }));
        let mut token = 0i64;
        unsafe {
            if let Ok(core) = controller.CoreWebView2() {
                let _ = core.add_ProcessFailed(&handler, &mut token);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renderer_failure(exit_code: Option<i32>) -> FailureInfo {
        FailureInfo {
            failure_kind: "renderProcessExited".to_string(),
            exit_code,
            reason: "crashed".to_string(),
            process_description: "renderer".to_string(),
            failure_module: "KERNELBASE.dll".to_string(),
        }
    }

    #[test]
    fn first_renderer_failure_requests_reload_and_keeps_pending_info() {
        let mut tracker = RecoveryTracker::default();
        let now = Instant::now();

        assert_eq!(
            tracker.record_failure(now, 1000, renderer_failure(Some(9))),
            RecoveryAction::Reload
        );
        assert!(tracker.is_unhealthy());
        assert_eq!(
            tracker.take_info(),
            Some(RecoveryInfo {
                recovered: true,
                failure_kind: "renderProcessExited".to_string(),
                exit_code: Some(9),
                happened_at_ms: 1000,
            })
        );
        assert_eq!(tracker.take_info(), None);
    }

    #[test]
    fn repeated_renderer_failure_within_five_minutes_uses_fallback_page() {
        let mut tracker = RecoveryTracker::default();
        let now = Instant::now();

        tracker.record_failure(now, 1000, renderer_failure(Some(1)));
        assert_eq!(
            tracker.record_failure(
                now + Duration::from_secs(4 * 60),
                2000,
                renderer_failure(Some(2)),
            ),
            RecoveryAction::Fallback
        );
    }

    #[test]
    fn renderer_failure_after_window_may_reload_again() {
        let mut tracker = RecoveryTracker::default();
        let now = Instant::now();

        tracker.record_failure(now, 1000, renderer_failure(None));
        assert_eq!(
            tracker.record_failure(
                now + Duration::from_secs(5 * 60 + 1),
                2000,
                renderer_failure(None),
            ),
            RecoveryAction::Reload
        );
    }

    #[test]
    fn non_renderer_failure_does_not_enter_safe_recovery() {
        let mut tracker = RecoveryTracker::default();
        let now = Instant::now();
        let failure = FailureInfo {
            failure_kind: "gpuProcessExited".to_string(),
            exit_code: Some(3),
            reason: "crashed".to_string(),
            process_description: "gpu".to_string(),
            failure_module: "".to_string(),
        };

        assert_eq!(
            tracker.record_failure(now, 1000, failure),
            RecoveryAction::LogOnly
        );
        assert!(!tracker.is_unhealthy());
        assert_eq!(tracker.take_info(), None);
    }

    #[test]
    fn successful_page_load_clears_unhealthy_but_preserves_pending_info() {
        let mut tracker = RecoveryTracker::default();
        tracker.record_failure(Instant::now(), 1000, renderer_failure(None));

        tracker.mark_page_loaded();

        assert!(!tracker.is_unhealthy());
        assert!(tracker.take_info().is_some());
    }

    #[test]
    fn fallback_page_stays_unhealthy_for_fast_exit() {
        let mut tracker = RecoveryTracker::default();
        let now = Instant::now();
        tracker.record_failure(now, 1000, renderer_failure(None));
        tracker.record_failure(now, 2000, renderer_failure(None));

        tracker.mark_page_loaded();

        assert!(tracker.is_unhealthy());
    }
}
