use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tauri::{AppHandle, Manager, State};

use crate::codex::app_server::{CodexServer, find_codex_sync};
use crate::codex::settings::{self, AppSettings};

type Server = Arc<CodexServer>;

#[tauri::command]
pub async fn server_status(server: State<'_, Server>) -> Result<Value, String> {
    Ok(server.status().await)
}

#[tauri::command]
pub async fn server_connect(server: State<'_, Server>) -> Result<(), String> {
    server.ensure_running();
    Ok(())
}

#[tauri::command]
pub async fn server_logs(server: State<'_, Server>) -> Result<Vec<String>, String> {
    let status = server.status().await;
    Ok(status
        .get("logs")
        .and_then(|l| l.as_array())
        .and_then(|a| {
            a.iter()
                .map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Option<Vec<_>>>()
        })
        .unwrap_or_default())
}

/// Generic passthrough for protocol methods not explicitly wrapped.
#[tauri::command]
pub async fn codex_rpc(
    server: State<'_, Server>,
    method: String,
    params: Value,
) -> Result<Value, String> {
    server.request(&method, params, None).await
}

#[tauri::command]
pub async fn codex_rpc_long(
    server: State<'_, Server>,
    method: String,
    params: Value,
    timeout_ms: u64,
) -> Result<Value, String> {
    server
        .request(&method, params, Some(Duration::from_millis(timeout_ms)))
        .await
}

/// 探测当前 codex 的置顶协议能力（只读，不修改任何线程状态）。
#[tauri::command]
pub async fn codex_pin_capability(server: State<'_, Server>) -> Result<Value, String> {
    server.pin_capability().await
}

/// Respond to a server-initiated request (approval / user input / elicitation).
#[tauri::command]
pub async fn interaction_respond(
    server: State<'_, Server>,
    request_id: u64,
    result: Value,
) -> Result<(), String> {
    server.send_response(request_id, result).await
}

#[tauri::command]
pub async fn thread_list(
    server: State<'_, Server>,
    limit: Option<u64>,
    cursor: Option<String>,
    cwd: Option<String>,
) -> Result<Value, String> {
    let mut params = serde_json::Map::new();
    if let Some(l) = limit {
        params.insert("limit".into(), json!(l));
    }
    if let Some(c) = cursor {
        params.insert("cursor".into(), json!(c));
    }
    if let Some(w) = cwd {
        params.insert("cwd".into(), json!(w));
    }
    server.request("thread/list", Value::Object(params), None).await
}

#[tauri::command]
pub async fn thread_start(
    server: State<'_, Server>,
    params: Value,
) -> Result<Value, String> {
    server
        .request("thread/start", params, Some(Duration::from_secs(60)))
        .await
}

#[tauri::command]
pub async fn thread_read(
    server: State<'_, Server>,
    thread_id: String,
    include_turns: Option<bool>,
) -> Result<Value, String> {
    server
        .request(
            "thread/read",
            json!({ "threadId": thread_id, "includeTurns": include_turns.unwrap_or(true) }),
            Some(Duration::from_secs(60)),
        )
        .await
}

#[tauri::command]
pub async fn thread_resume(
    server: State<'_, Server>,
    params: Value,
) -> Result<Value, String> {
    server
        .request("thread/resume", params, Some(Duration::from_secs(60)))
        .await
}

#[tauri::command]
pub async fn thread_delete(
    server: State<'_, Server>,
    thread_id: String,
) -> Result<Value, String> {
    server
        .request("thread/delete", json!({ "threadId": thread_id }), None)
        .await
}

#[tauri::command]
pub async fn thread_set_name(
    server: State<'_, Server>,
    thread_id: String,
    name: String,
) -> Result<Value, String> {
    server
        .request(
            "thread/name/set",
            json!({ "threadId": thread_id, "name": name }),
            None,
        )
        .await
}

#[tauri::command]
pub async fn turn_start(
    server: State<'_, Server>,
    params: Value,
) -> Result<Value, String> {
    server
        .request("turn/start", params, Some(Duration::from_secs(60)))
        .await
}

#[tauri::command]
pub async fn turn_steer(
    server: State<'_, Server>,
    params: Value,
) -> Result<Value, String> {
    server
        .request("turn/steer", params, Some(Duration::from_secs(60)))
        .await
}

#[tauri::command]
pub async fn turn_interrupt(
    server: State<'_, Server>,
    thread_id: String,
    turn_id: String,
) -> Result<Value, String> {
    server
        .request(
            "turn/interrupt",
            json!({ "threadId": thread_id, "turnId": turn_id }),
            None,
        )
        .await
}

#[tauri::command]
pub async fn goal_set(
    server: State<'_, Server>,
    thread_id: String,
    objective: String,
) -> Result<Value, String> {
    server
        .request(
            "thread/goal/set",
            json!({ "threadId": thread_id, "objective": objective }),
            None,
        )
        .await
}

#[tauri::command]
pub async fn goal_get(
    server: State<'_, Server>,
    thread_id: String,
) -> Result<Value, String> {
    server
        .request("thread/goal/get", json!({ "threadId": thread_id }), None)
        .await
}

#[tauri::command]
pub async fn goal_clear(
    server: State<'_, Server>,
    thread_id: String,
) -> Result<Value, String> {
    server
        .request("thread/goal/clear", json!({ "threadId": thread_id }), None)
        .await
}

#[tauri::command]
pub async fn auth_status(server: State<'_, Server>) -> Result<Value, String> {
    server
        .request(
            "getAuthStatus",
            json!({ "includeToken": false, "refreshToken": false }),
            None,
        )
        .await
}

#[tauri::command]
pub fn auth_login(app: AppHandle) -> Result<(), String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let settings = settings::load(&dir);
    let codex = find_codex_sync(&settings)?;
    let mut cmd = std::process::Command::new(&codex);
    cmd.arg("login");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // 新建独立控制台窗口运行交互式 codex login
        cmd.creation_flags(0x0000_0010); // CREATE_NEW_CONSOLE
    }
    cmd.spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn auth_api_key_configured() -> bool {
    for key in ["OPENAI_API_KEY", "CODEX_API_KEY", "OPENAI_BASE_URL"] {
        if let Ok(v) = std::env::var(key) {
            if !v.trim().is_empty() {
                return true;
            }
        }
    }
    false
}

#[tauri::command]
pub async fn auth_logout(server: State<'_, Server>) -> Result<Value, String> {
    server.request("account/logout", Value::Null, None).await
}

#[tauri::command]
pub fn workspace_dir(server: State<'_, Server>) -> String {
    server.workspace().to_string_lossy().into_owned()
}

#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    // ShellExecute 默认打开：URL 用浏览器、目录用资源管理器；无控制台窗口、不改写路径
    open::that(&url).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn reveal_path(path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    if p.is_dir() {
        return open::that(p).map_err(|e| e.to_string());
    }
    if p.is_file() {
        // explorer /select 在资源管理器中定位文件
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", p.display()))
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    Err(format!("文件或目录不存在: {}", path))
}

#[tauri::command]
pub async fn pick_files(
    multiple: bool,
    initial_dir: Option<String>,
) -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new().add_filter("所有文件", &["*"]);
        if let Some(d) = initial_dir.as_deref() {
            if !d.is_empty() {
                dialog = dialog.set_directory(d);
            }
        }
        let picked = if multiple {
            dialog.pick_files()
        } else {
            dialog.pick_file().map(|f| vec![f])
        };
        Ok(picked
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn pick_directory(initial_dir: Option<String>) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new();
        if let Some(d) = initial_dir.as_deref() {
            if !d.is_empty() {
                dialog = dialog.set_directory(d);
            }
        }
        Ok(dialog.pick_folder().map(|p| p.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 文件名（大小写不敏感）必须为 codex.exe
fn is_codex_exe(p: &std::path::Path) -> bool {
    p.file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase() == "codex.exe")
        .unwrap_or(false)
}

/// 选择 codex 可执行文件：对话框仅显示 .exe，且只能选择名为 codex.exe 的文件
#[tauri::command]
pub async fn pick_codex_file(initial_dir: Option<String>) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new().add_filter("Codex 可执行文件", &["exe"]);
        if let Some(d) = initial_dir.as_deref() {
            if !d.is_empty() {
                dialog = dialog.set_directory(d);
            }
        }
        let Some(picked) = dialog.pick_file() else {
            return Ok(None);
        };
        if !is_codex_exe(&picked) {
            return Err(format!(
                "请选择名为 codex.exe 的文件（当前选择：{}）",
                picked.display()
            ));
        }
        Ok(Some(picked.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 图片扩展名白名单（来自粘贴文件名/剪贴板类型）
fn image_extension(name: &str) -> Result<String, String> {
    let ext = name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp") {
        Ok(ext)
    } else {
        Err(format!("不支持的图片格式: {name}"))
    }
}

fn pasted_file_name(ext: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static PASTE_SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = PASTE_SEQ.fetch_add(1, Ordering::Relaxed);
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("pasted-{}-{ms}-{seq}.{ext}", std::process::id())
}

/// 把图片字节写入指定目录，返回绝对路径
fn save_image_bytes(
    dir: &std::path::Path,
    name: &str,
    bytes: &[u8],
) -> Result<String, String> {
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("创建目录失败 {}: {e}", dir.display()))?;
    let path = dir.join(name);
    std::fs::write(&path, bytes)
        .map_err(|e| format!("写入图片失败 {}: {e}", path.display()))?;
    Ok(path.to_string_lossy().into_owned())
}

/// 粘贴的图片落盘：优先临时目录 `%TEMP%\codex-ui-paste`，失败回退应用数据目录
#[tauri::command]
pub async fn save_pasted_image(
    app: AppHandle,
    bytes: Vec<u8>,
    name: String,
) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let ext = image_extension(&name)?;
        let file_name = pasted_file_name(&ext);
        let primary = std::env::temp_dir().join("codex-ui-paste");
        if let Ok(p) = save_image_bytes(&primary, &file_name, &bytes) {
            return Ok(p);
        }
        let fallback = app
            .path()
            .app_data_dir()
            .map_err(|e| format!("获取应用数据目录失败: {e}"))?
            .join("attachments");
        save_image_bytes(&fallback, &file_name, &bytes)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 读取剪贴板中资源管理器复制的文件原始路径（CF_HDROP）
#[tauri::command]
pub async fn clipboard_file_paths() -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(|| {
        let mut files: Vec<String> = Vec::new();
        clipboard_win::raw::open().map_err(|e| format!("打开剪贴板失败: {e}"))?;
        let r = clipboard_win::raw::get_file_list(&mut files);
        let _ = clipboard_win::raw::close();
        r.map_err(|e| format!("读取剪贴板文件失败: {e}"))?;
        Ok(files)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn settings_get(app: AppHandle) -> Result<AppSettings, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(settings::load(&dir))
}

#[tauri::command]
pub fn settings_set(app: AppHandle, settings: AppSettings) -> Result<(), String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    settings::save(&dir, &settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_extension_whitelist() {
        assert_eq!(image_extension("shot.png").unwrap(), "png");
        assert_eq!(image_extension("a.JPEG").unwrap(), "jpeg");
        assert_eq!(image_extension("x.webp").unwrap(), "webp");
        assert!(image_extension("a.txt").is_err());
        assert!(image_extension("noext").is_err());
    }

    #[test]
    fn save_image_bytes_writes_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = save_image_bytes(dir.path(), "pasted-1-123.png", b"img").unwrap();
        assert!(std::path::Path::new(&p).is_file());
        assert_eq!(std::fs::read(&p).unwrap(), b"img");
        assert!(p.contains("pasted-1-123.png"));
    }

    #[test]
    fn pasted_file_name_unique_per_call() {
        let a = pasted_file_name("png");
        let b = pasted_file_name("png");
        assert_ne!(a, b);
        assert!(a.ends_with(".png"));
    }

    #[test]
    fn is_codex_exe_name_check() {
        use std::path::Path;
        assert!(is_codex_exe(Path::new("C:/tools/codex.exe")));
        assert!(is_codex_exe(Path::new("C:/tools/CODEX.EXE")));
        assert!(is_codex_exe(Path::new("codex.exe")));
        assert!(!is_codex_exe(Path::new("C:/tools/other.exe")));
        assert!(!is_codex_exe(Path::new("C:/tools/codex.cmd")));
        assert!(!is_codex_exe(Path::new("C:/tools/")));
    }
}
