//! Markdown → PDF 导出：编辑器右键菜单「导出 PDF」。
//!
//! 链路：前端用 marked 渲染出打印样式 HTML → 本命令弹系统保存对话框 →
//! 隐藏 WebView 窗口经 `print-export://` 自定义协议直接加载该 HTML →
//! 以 WebView2 `NavigationCompleted` 事件确认加载完成 →
//! `PrintToPdfAsync` 写出 PDF。
//! COM 互操作部分为 Windows/WebView2 专属，以 `#[cfg(windows)]` 隔离。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::codex::path_util::clean_path;

const PRINT_WINDOW_LABEL: &str = "pdf-export";

/// 打印 HTML 的临时存储：id → 完整打印文档（`print-export://` 协议处理器读取）。
pub struct PdfExportState(pub Mutex<HashMap<String, String>>);

/// 生成唯一导出 id（时间戳 + 自增序号，避免同纳秒碰撞）。
fn new_export_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("e{nanos}-{seq}")
}

/// 打印页导航 URL：wry 在 Windows 上把自定义协议注册为 `http://<scheme>.localhost/` 的
/// workaround 形式（WebView2 不支持非标准 scheme 的顶层导航），导航必须用该形式；
/// id 位于 path 段，与协议处理器取 id 的逻辑一致。
pub fn print_url(id: &str) -> String {
    format!("http://print-export.localhost/{id}")
}

/// 判断导航是否属于打印页：兼容 workaround 形式与原始 scheme 形式。
fn is_print_export_navigation(uri: &str) -> bool {
    uri.starts_with("http://print-export.localhost/")
        || uri.starts_with("https://print-export.localhost/")
        || uri.starts_with("print-export://")
}

#[tauri::command]
pub async fn export_markdown_pdf(
    app: AppHandle,
    html: String,
    suggested_name: Option<String>,
    initial_dir: Option<String>,
) -> Result<Option<String>, String> {
    if html.trim().is_empty() {
        return Err("导出内容为空".into());
    }

    // 1. 系统保存对话框：取消返回 None（前端不提示）
    let save_path = tauri::async_runtime::spawn_blocking({
        let suggested_name = suggested_name.clone();
        let initial_dir = initial_dir.clone();
        move || {
            let mut dialog = rfd::FileDialog::new().add_filter("PDF 文件", &["pdf"]);
            if let Some(name) = suggested_name.as_deref() {
                if !name.is_empty() {
                    dialog = dialog.set_file_name(name);
                }
            }
            if let Some(dir) = initial_dir.as_deref() {
                if !dir.is_empty() {
                    dialog = dialog.set_directory(dir);
                }
            }
            dialog.save_file().map(|p| clean_path(&p))
        }
    })
    .await
    .map_err(|e| format!("保存对话框异常: {e}"))?;

    let Some(save_path) = save_path else {
        return Ok(None);
    };
    let save_path = PathBuf::from(save_path);

    // 2. 暂存打印 HTML，隐藏窗口经自定义协议加载（不依赖静态文件/document.write）
    let id = new_export_id();
    let url = print_url(&id);
    Url::parse(&url).map_err(|e| format!("打印页 URL 非法: {e}"))?;
    app.state::<PdfExportState>()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.clone(), html);

    // 同名标签残留时先关闭
    if let Some(old) = app.get_webview_window(PRINT_WINDOW_LABEL) {
        let _ = old.close();
    }
    // 初始页用 about:blank：导航只由 navigate_and_wait 显式发起一次，
    // 避免窗口创建时已开始的 print-export 导航被二次导航取消（CONNECTION_ABORTED 误报）
    let window = WebviewWindowBuilder::new(
        &app,
        PRINT_WINDOW_LABEL,
        WebviewUrl::External(
            Url::parse("about:blank").map_err(|e| format!("初始页面 URL 非法: {e}"))?,
        ),
    )
    .title("")
    .visible(false)
    .decorations(false)
    .skip_taskbar(true)
    .resizable(false)
    .focused(false)
    .inner_size(800.0, 1000.0)
    .build()
    .map_err(|e| format!("创建导出窗口失败: {e}"))?;

    let result = print_html_to_pdf(&window, &url, &save_path).await;

    // 无论成败：关闭隐藏窗口并清理暂存内容
    if let Some(win) = app.get_webview_window(PRINT_WINDOW_LABEL) {
        let _ = win.close();
    }
    app.state::<PdfExportState>()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&id);

    result?;
    Ok(Some(clean_path(&save_path)))
}

/// 导航确认 + 图片等待 + WebView2 打印。
async fn print_html_to_pdf(
    window: &WebviewWindow,
    url: &str,
    save_path: &Path,
) -> Result<(), String> {
    // NavigationCompleted 事件权威确认页面加载完成/失败，失败即时报错
    navigate_and_wait(window, url).await?;

    // 图片等待尽力而为：超时照常打印
    wait_until(
        window,
        "document.readyState === 'complete' && Array.from(document.images).every(i => i.complete)",
        15_000,
    )
    .await?;

    print_webview_to_pdf(window, save_path).await
}

/// 轮询 JS 表达式直到返回 true 或超时（超时按就绪继续，尽力而为）；
/// eval 异常视为未就绪继续轮询。仅用于打印前的图片等待。
async fn wait_until(window: &WebviewWindow, js: &str, timeout_ms: u64) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    loop {
        if std::time::Instant::now() >= deadline {
            return Ok(());
        }
        let ready = Arc::new(AtomicBool::new(false));
        let flag = ready.clone();
        if window
            .eval_with_callback(js.to_string(), move |v| {
                if v.trim() == "true" {
                    flag.store(true, Ordering::SeqCst);
                }
            })
            .is_err()
        {
            // 页面尚未就绪：继续轮询
        }
        if ready.load(Ordering::SeqCst) {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// 重新导航到打印页并以 NavigationCompleted 事件等待加载完成。
/// 失败时把 WebErrorStatus 编入错误信息，避免静默超时。
#[cfg(windows)]
async fn navigate_and_wait(window: &WebviewWindow, url: &str) -> Result<(), String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_WEB_ERROR_STATUS;
    use webview2_com::{NavigationCompletedEventHandler, NavigationStartingEventHandler};
    use windows_core::{HSTRING, PWSTR};

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    let url = url.to_string();
    window
        .with_webview(move |webview| {
            let result = (|| -> Result<(), String> {
                let core = unsafe { webview.controller().CoreWebView2() }
                    .map_err(|e| format!("获取 WebView2 控制器失败: {e}"))?;

                // 目标导航 id：只记录我们自己发起的 print-export:// 导航；
                // 被取消的旧导航（about:blank）触发的事件据此过滤，避免 CONNECTION_ABORTED 误报
                let target_id = Arc::new(Mutex::new(None::<u64>));
                let target_ref = target_id.clone();
                let mut start_token = 0i64;
                let starting = NavigationStartingEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else {
                        return Ok(());
                    };
                    unsafe {
                        let mut uri = PWSTR::null();
                        let _ = args.Uri(&mut uri);
                        let uri_text = uri.to_string().unwrap_or_default();
                        if is_print_export_navigation(&uri_text) {
                            let mut nav_id = 0u64;
                            let _ = args.NavigationId(&mut nav_id);
                            *target_ref.lock().unwrap() = Some(nav_id);
                        }
                    }
                    Ok(())
                }));
                unsafe {
                    core.add_NavigationStarting(&starting, &mut start_token)
                        .map_err(|e| format!("注册导航开始事件失败: {e}"))?;
                }

                let (done_tx, done_rx) = std::sync::mpsc::channel::<Result<(), String>>();
                let mut done_token = 0i64;
                let target_done = target_id.clone();
                let handler = NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else {
                        return Ok(());
                    };
                    unsafe {
                        let mut nav_id = 0u64;
                        let _ = args.NavigationId(&mut nav_id);
                        // 非目标导航（被取消的旧导航等）一律忽略
                        if target_done.lock().unwrap().as_ref() != Some(&nav_id) {
                            return Ok(());
                        }
                        let mut ok = windows_core::BOOL::default();
                        let mut status = COREWEBVIEW2_WEB_ERROR_STATUS(0);
                        let _ = args.IsSuccess(&mut ok);
                        let _ = args.WebErrorStatus(&mut status);
                        let res = if ok.as_bool() {
                            Ok(())
                        } else {
                            Err(format!("打印页面加载失败（WebErrorStatus={status:?}）"))
                        };
                        let _ = done_tx.send(res);
                    }
                    Ok(())
                }));
                // SAFETY: WebView2 COM 调用由 webview2-com 绑定；先注册事件再重新导航，
                // wait_with_pump 泵消息直至 NavigationCompleted 回调返回。
                unsafe {
                    core.add_NavigationCompleted(&handler, &mut done_token)
                        .map_err(|e| format!("注册导航事件失败: {e}"))?;
                    let url_h = HSTRING::from(url);
                    core.Navigate(&url_h)
                        .map_err(|e| format!("发起打印页导航失败: {e}"))?;
                }
                webview2_com::wait_with_pump(done_rx)
                    .map_err(|e| format!("等待打印页加载失败: {e}"))?
            })();
            let _ = tx.send(result);
        })
        .map_err(|e| format!("访问导出窗口失败: {e}"))?;

    match rx.await {
        Ok(res) => res,
        Err(_) => Err("导出窗口未执行导航".into()),
    }
}

#[cfg(not(windows))]
async fn navigate_and_wait(_window: &WebviewWindow, _url: &str) -> Result<(), String> {
    Err("导出 PDF 仅支持 Windows/WebView2".into())
}

/// WebView2 `PrintToPdfAsync`：把当前 WebView 页面内容直接写出到目标路径。
#[cfg(windows)]
async fn print_webview_to_pdf(window: &WebviewWindow, save_path: &Path) -> Result<(), String> {
    use windows_core::PCWSTR;

    let path_wide: Vec<u16> = save_path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    window
        .with_webview(move |webview| {
            let controller = webview.controller();
            let environment = webview.environment();
            let path_pwstr = PCWSTR(path_wide.as_ptr());
            let _ = tx.send(print_with_webview2(&controller, &environment, path_pwstr));
        })
        .map_err(|e| format!("访问导出窗口失败: {e}"))?;

    match rx.await {
        Ok(res) => res,
        Err(_) => Err("导出窗口未执行打印".into()),
    }
}

#[cfg(windows)]
fn print_with_webview2(
    controller: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller,
    environment: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Environment,
    path: windows_core::PCWSTR,
) -> Result<(), String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Environment6, ICoreWebView2_7,
    };
    use webview2_com::PrintToPdfCompletedHandler;
    use windows_core::Interface;

    // SAFETY: WebView2 COM 接口由 webview2-com 绑定；控制器/环境取自当前窗口，
    // PrintToPdf 为异步操作，wait_for_async_operation 泵消息直至回调完成。
    unsafe {
        let core = controller
            .CoreWebView2()
            .map_err(|e| format!("获取 WebView2 控制器失败: {e}"))?;
        let webview7 = core
            .cast::<ICoreWebView2_7>()
            .map_err(|e| format!("当前 WebView2 运行时过旧，不支持 PrintToPdf: {e}"))?;
        let env6 = environment
            .cast::<ICoreWebView2Environment6>()
            .map_err(|e| format!("获取 WebView2 打印设置接口失败: {e}"))?;
        let settings = env6
            .CreatePrintSettings()
            .map_err(|e| format!("创建打印设置失败: {e}"))?;
        settings
            .SetShouldPrintBackgrounds(true)
            .map_err(|e| format!("设置打印背景失败: {e}"))?;

        PrintToPdfCompletedHandler::wait_for_async_operation(
            Box::new(move |handler| {
                webview7
                    .PrintToPdf(path, &settings, &handler)
                    .map_err(webview2_com::Error::from)
            }),
            Box::new(|hr_result: windows_core::Result<()>, success: bool| {
                if success {
                    hr_result
                } else {
                    match hr_result {
                        Err(e) => Err(e),
                        Ok(()) => Err(windows_core::Error::from(windows_core::HRESULT(
                            0x8000_4005u32 as i32,
                        ))),
                    }
                }
            }),
        )
        .map_err(|e| format!("导出 PDF 失败: {e}"))?;
        Ok(())
    }
}

#[cfg(not(windows))]
async fn print_webview_to_pdf(_window: &WebviewWindow, _save_path: &Path) -> Result<(), String> {
    Err("导出 PDF 仅支持 Windows/WebView2".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_url_builds_custom_scheme_url() {
        let url = print_url("e123-0");
        assert_eq!(url, "http://print-export.localhost/e123-0");
        let parsed = Url::parse(&url).unwrap();
        // 协议处理器从 path 段取 id
        assert_eq!(parsed.path().trim_start_matches('/'), "e123-0");
    }

    #[test]
    fn print_navigation_uri_is_recognized() {
        assert!(is_print_export_navigation("http://print-export.localhost/e1"));
        assert!(is_print_export_navigation("https://print-export.localhost/e1"));
        assert!(is_print_export_navigation("print-export://localhost/e1"));
        assert!(!is_print_export_navigation("http://example.com/e1"));
        assert!(!is_print_export_navigation(""));
    }

    #[test]
    fn export_ids_are_unique() {
        assert_ne!(new_export_id(), new_export_id());
    }
}
