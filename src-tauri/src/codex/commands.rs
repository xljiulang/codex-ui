use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{Value, json};
use tauri::{AppHandle, Manager, State};

use crate::codex::app_server::{CodexServer, apply_codex_env, find_codex_sync};
use crate::codex::custom_instructions;
use crate::codex::model_config;
use crate::codex::path_util::clean_path;
use crate::codex::settings::{self, AppSettings};
use crate::codex::skills;

type Server = Arc<CodexServer>;

/// 全局串行化系统“选择文件夹”对话框：任何入口（头部新建会话、输入框目录附件等）
/// 同一时刻只允许弹出一个，避免多个原生对话框叠加。
static PICK_DIRECTORY_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn pick_directory_lock() -> &'static Mutex<()> {
    PICK_DIRECTORY_LOCK.get_or_init(|| Mutex::new(()))
}

/// E2E 测试钩子开关：仅当以 CODEX_UI_TEST=1 启动时暴露 window.__CODEX_UI_TEST__，
/// 正常启动不暴露——避免 openLink/openPathInApp 被测试钩子短路导致生产环境
/// 文件链接/引用点击无反应。
#[tauri::command]
pub fn test_hook_enabled() -> bool {
    std::env::var_os("CODEX_UI_TEST").is_some()
}

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

/// 前端用户动作日志（仅允许安全字段；调用方负责不传敏感内容，失败静默）。
#[tauri::command]
pub async fn session_log(
    server: State<'_, Server>,
    level: String,
    thread_id: Option<String>,
    event: String,
    detail: Option<String>,
) -> Result<(), String> {
    server.session_log(level, thread_id, event, detail);
    Ok(())
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

/// 只读获取当前 codex 的内置 Pinned 分区 id（不修改任何线程状态）。
#[tauri::command]
pub async fn codex_pinned_section_id(server: State<'_, Server>) -> Result<String, String> {
    server.pinned_section_id().await
}

/// Respond to a server-initiated request (approval / user input / elicitation).
#[tauri::command]
pub async fn interaction_respond(
    server: State<'_, Server>,
    request_id: Value,
    result: Value,
) -> Result<(), String> {
    server.send_response(&request_id, result).await
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
    // 协议核对（0.149.0）：`getAuthStatus` 为兼容旧接口保留，正式接口为
    // `account/read`，这里使用正式接口。前端当前未调用本命令。
    server.request("account/read", json!({}), None).await
}

#[tauri::command]
pub fn auth_login(app: AppHandle) -> Result<(), String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let settings = settings::load(&dir);
    let codex = find_codex_sync(&settings)?;
    let mut cmd = std::process::Command::new(&codex);
    cmd.arg("login");
    apply_codex_env(&mut cmd);
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
pub fn startup_workspace(server: State<'_, Server>) -> String {
    clean_path(server.workspace())
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
        // explorer 的 /select 参数无法处理路径含逗号的情况，退化为打开所在目录
        if p.to_string_lossy().contains(',') {
            if let Some(parent) = p.parent() {
                return open::that(parent).map_err(|e| e.to_string());
            }
        }
        // explorer /select 在资源管理器中定位文件
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", p.display()))
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    Err(format!("文件或目录不存在: {}", path))
}

/// 应用非空初始目录到原生文件对话框
fn dialog_with_initial_dir(
    mut dialog: rfd::FileDialog,
    initial_dir: Option<String>,
) -> rfd::FileDialog {
    if let Some(d) = initial_dir.as_deref() {
        if !d.is_empty() {
            dialog = dialog.set_directory(d);
        }
    }
    dialog
}

#[tauri::command]
pub async fn pick_files(
    multiple: bool,
    initial_dir: Option<String>,
) -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(move || {
        let dialog = dialog_with_initial_dir(
            rfd::FileDialog::new().add_filter("所有文件", &["*"]),
            initial_dir,
        );
        let picked = if multiple {
            dialog.pick_files()
        } else {
            dialog.pick_file().map(|f| vec![f])
        };
        Ok(picked
            .unwrap_or_default()
            .into_iter()
            .map(|p| clean_path(&p))
            .collect::<Vec<_>>())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn pick_directory(initial_dir: Option<String>) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        // 锁在阻塞任务内获取并持有到对话框关闭：与 git_op_lock 同模式，
        // 保证排队等待的后续调用在对话框真正关闭前不会并发弹窗。
        let _guard = pick_directory_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dialog = dialog_with_initial_dir(rfd::FileDialog::new(), initial_dir);
        Ok(dialog.pick_folder().map(|p| clean_path(&p)))
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
        let dialog = dialog_with_initial_dir(
            rfd::FileDialog::new().add_filter("Codex 可执行文件", &["exe"]),
            initial_dir,
        );
        let Some(picked) = dialog.pick_file() else {
            return Ok(None);
        };
        if !is_codex_exe(&picked) {
            return Err(format!(
                "请选择名为 codex.exe 的文件（当前选择：{}）",
                picked.display()
            ));
        }
        Ok(Some(clean_path(&picked)))
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
    Ok(clean_path(&path))
}

/// 清理目录中超过 `max_age` 的旧文件（粘贴图片临时目录防无限累积；尽力而为）
fn cleanup_old_files(dir: &std::path::Path, max_age: std::time::Duration) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for e in rd.flatten() {
        let Ok(meta) = e.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let too_old = meta
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .map(|d| d > max_age)
            .unwrap_or(false);
        if too_old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// 粘贴的图片落盘：优先临时目录 `%TEMP%\codex-ui-paste`，失败回退应用数据目录
#[tauri::command]
pub async fn save_pasted_image(
    app: AppHandle,
    bytes: Vec<u8>,
    name: String,
) -> Result<String, String> {
    const MAX_PASTE_IMAGE_BYTES: usize = 50 * 1024 * 1024;
    if bytes.len() > MAX_PASTE_IMAGE_BYTES {
        return Err("图片过大（超过 50 MB），无法粘贴".into());
    }
    tokio::task::spawn_blocking(move || {
        let ext = image_extension(&name)?;
        let file_name = pasted_file_name(&ext);
        let primary = std::env::temp_dir().join("codex-ui-paste");
        // 顺手清理 7 天前的残留粘贴图片，避免临时目录无限增长
        cleanup_old_files(&primary, std::time::Duration::from_secs(7 * 24 * 3600));
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

/// 读取模型配置（CODEX_HOME/config.toml 与 model_catalog_json 目标文件），
/// 供设置页「模型配置」Tab 使用。
#[tauri::command]
pub fn model_config_read() -> Result<model_config::ModelConfigState, String> {
    model_config::read_state()
}

/// 保存 config.toml 整文件：内容必须为合法 TOML，原文写入，不注入受管键。
#[tauri::command]
pub fn model_config_save(content: String) -> Result<(), String> {
    model_config::save_config(&content)
}

/// 保存可视化模型配置（model / model_reasoning_effort / model_provider / 提供方列表）：
/// 只写可视化覆盖的键与提供方表，其余 TOML 内容保留。
#[tauri::command]
pub fn model_config_ui_save(input: model_config::ModelConfigUiEdit) -> Result<(), String> {
    model_config::save_config_ui(&input)
}

/// 保存 model_catalog_json 目标文件原文：目标路径从 config.toml 解析，
/// 内容必须非空且为合法 JSON。
#[tauri::command]
pub fn model_catalog_save(content: String) -> Result<(), String> {
    model_config::save_model_catalog(&content)
}

/// 检查 model_catalog_json 配置值对应的目标文件当前是否存在（不创建文件），
/// 供设置页保存前给出黄色警告。
#[tauri::command]
pub fn model_catalog_target_exists(value: String) -> Result<bool, String> {
    model_config::catalog_target_exists(&value)
}

/// 读取本地技能列表（从 skills/list 聚合列表过滤 CODEX_HOME/skills 下的技能），
/// 供设置页「技能管理」使用；force_reload 为 true 时绕过技能缓存强制重扫。
#[tauri::command]
pub async fn skills_read(
    server: State<'_, Server>,
    force_reload: Option<bool>,
) -> Result<skills::SkillsState, String> {
    skills::read_state(&server, force_reload.unwrap_or(false)).await
}

/// 读取自定义指令（CODEX_HOME/AGENTS.md），供设置页「模型配置」Tab 使用。
#[tauri::command]
pub fn custom_instructions_read() -> Result<custom_instructions::CustomInstructionsState, String> {
    custom_instructions::read_state()
}

/// 保存自定义指令（CODEX_HOME/AGENTS.md）：原文写入，允许空内容。
#[tauri::command]
pub fn custom_instructions_save(content: String) -> Result<(), String> {
    custom_instructions::save(&content)
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

    #[test]
    fn pick_directory_lock_serializes_entries() {
        // 锁可重入获取（同一线程）且互斥：同时只有一个调用持有
        let lock = pick_directory_lock();
        let g1 = lock.lock().unwrap();
        let g2 = lock.try_lock();
        assert!(g2.is_err(), "已持有时再获取必须失败，防止并发弹窗");
        drop(g1);
        assert!(lock.try_lock().is_ok(), "释放后应可再次获取");
    }
}
