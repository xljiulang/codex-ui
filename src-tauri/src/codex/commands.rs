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
    let mut cmd = std::process::Command::new("cmd");
    cmd.args(["/c", "start", "", &url]);
    cmd.spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn pick_files(multiple: bool) -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(move || {
        let dialog = rfd::FileDialog::new().add_filter("所有文件", &["*"]);
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
pub async fn pick_directory() -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        Ok(rfd::FileDialog::new()
            .pick_folder()
            .map(|p| p.to_string_lossy().into_owned()))
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
