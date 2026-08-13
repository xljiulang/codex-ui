use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::Manager;

/// 文本预览参数（由主窗口传入新窗口）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextPreviewParams {
    pub root: String,
    pub path: String,
}

/// 新窗口取走参数的共享状态
pub struct TextPreviewParamsState(pub Mutex<Option<TextPreviewParams>>);

/// 打开独立文本预览窗口；参数存入共享状态，由新窗口一次性取走
#[tauri::command]
pub async fn open_text_preview(
    app: tauri::AppHandle,
    root: String,
    path: String,
) -> Result<(), String> {
    if let Some(state) = app.try_state::<TextPreviewParamsState>() {
        *state.0.lock().unwrap() = Some(TextPreviewParams { root, path });
    }
    // 窗口创建必须发生在主线程消息泵上：同步创建 WebView2 窗口会阻塞主线程导致死锁
    // （新窗口停在 about:blank、invoke 永不返回），因此用 run_on_main_thread 异步创建。
    let app2 = app.clone();
    app.run_on_main_thread(move || {
        if let Some(win) = app2.get_webview_window("text-preview") {
            // 复用已有窗口：更新参数后重新加载，避免 close+新建同 label 窗口的竞态
            let _ = win.eval("location.reload()");
            return;
        }
        if let Err(e) = tauri::WebviewWindowBuilder::new(
            &app2,
            "text-preview",
            tauri::WebviewUrl::App("index.html".into()),
        )
        .title("文本预览")
        .inner_size(1280.0, 720.0)
        .min_inner_size(400.0, 560.0)
        .center()
        .resizable(true)
        .build()
        {
            eprintln!("open text preview window failed: {e}");
        }
    })
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn take_text_preview_params(
    state: tauri::State<'_, TextPreviewParamsState>,
) -> Option<TextPreviewParams> {
    state.0.lock().unwrap().take()
}
