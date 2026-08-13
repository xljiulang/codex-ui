use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

/// 文本编辑参数（由主窗口传入新窗口）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextEditorParams {
    pub root: String,
    pub path: String,
}

/// 新窗口取走参数的共享状态
pub struct TextEditorParamsState(pub Mutex<Option<TextEditorParams>>);

/// 打开独立文本编辑窗口；参数存入共享状态，由新窗口一次性取走
#[tauri::command]
pub async fn open_text_editor(
    app: tauri::AppHandle,
    root: String,
    path: String,
) -> Result<(), String> {
    let params = TextEditorParams { root, path };
    // 标题取文件名（不含路径），提取失败回退默认值，避免首开时闪现旧标题
    let title = Path::new(&params.path)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("文本编辑")
        .to_string();
    if let Some(state) = app.try_state::<TextEditorParamsState>() {
        *state.0.lock().unwrap() = Some(params.clone());
    }
    // 窗口创建必须发生在主线程消息泵上：同步创建 WebView2 窗口会阻塞主线程导致死锁
    // （新窗口停在 about:blank、invoke 永不返回），因此用 run_on_main_thread 异步创建。
    let app2 = app.clone();
    app.run_on_main_thread(move || {
        if app2.get_webview_window("text-editor").is_some() {
            // 复用已有窗口：发事件通知前端切换文件（不 reload，避免丢失未保存内容）
            let _ = app2.emit_to("text-editor", "text-editor/open", &params);
            return;
        }
        if let Err(e) = tauri::WebviewWindowBuilder::new(
            &app2,
            "text-editor",
            tauri::WebviewUrl::App("index.html".into()),
        )
        .title(title)
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
pub fn take_text_editor_params(
    state: tauri::State<'_, TextEditorParamsState>,
) -> Option<TextEditorParams> {
    state.0.lock().unwrap().take()
}
