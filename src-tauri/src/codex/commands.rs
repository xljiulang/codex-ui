use std::sync::{Arc, Mutex, OnceLock};
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::{Value, json};
use tauri::{AppHandle, Manager, State};

use crate::codex::app_server::{CodexServer, apply_codex_env, find_codex_sync};
use crate::codex::custom_instructions;
use crate::codex::model_config;
use crate::codex::path_util::clean_path;
use crate::codex::model_catalog;
use crate::codex::scheduled_tasks::{self, ScheduledTask, ScheduledTaskStore, TaskScheduler, TaskRunRecord};
use crate::codex::session_state::{SessionState, SessionStateStore};
use crate::codex::settings::{self, AppSettings};
use crate::codex::skills;
use crate::codex::wechat_bridge::WeChatBridge;
use crate::codex::zen_proxy;

type Server = Arc<CodexServer>;

/// 微信桥托管句柄（与 Server 一致的 Arc 形态）。
type WeChat = Arc<WeChatBridge>;

/// 定时任务存储/调度器托管句柄。
type TaskStore = Arc<ScheduledTaskStore>;
type Scheduler = Arc<TaskScheduler>;

/// 生成「纯参数透传」RPC 命令：命令签名统一为 `(State<Server>, params: Value)`，
/// 仅转发到指定协议方法并携带超时。用于收敛 thread_start / thread_resume /
/// turn_start / turn_steer 这四个同形 Tauri 命令，集中超时管理、避免方法名手写错。
/// 其它需要拼装参数或不同签名的 RPC 命令（thread_read / goal_* / auth_* 等）不适用本宏，
/// 仍保留手写。
macro_rules! rpc_passthrough {
    ($name:ident, $method:literal, $timeout:expr) => {
        #[tauri::command]
        pub async fn $name(server: State<'_, Server>, params: Value) -> Result<Value, String> {
            server.request($method, params, $timeout).await
        }
    };
}

/// 应用退出（前端安全收尾完成后调用）：放行并触发 RunEvent::Exit，
/// 统一关停 codex server、微信与运行中终端。
#[tauri::command]
pub fn app_exit(app: AppHandle) {
    crate::ALLOW_EXIT.store(true, Ordering::SeqCst);
    app.exit(0);
}

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

/// 开启/关闭 Zen 本地代理并返回状态；端口或上游地址变化时自动重启。
#[tauri::command]
pub async fn zen_proxy_apply(
    server: State<'_, Server>,
    enabled: bool,
    port: Option<u16>,
    base_url: Option<String>,
) -> Result<zen_proxy::ZenProxyStatus, String> {
    let current = server.zen_proxy_status().await;
    let port = port.unwrap_or(current.port);
    let base_url = base_url.unwrap_or_else(|| zen_proxy::DEFAULT_ZEN_BASE_URL.to_string());
    Ok(server.apply_zen_proxy(enabled, port, base_url).await)
}

/// 查询 Zen 本地代理当前状态。
#[tauri::command]
pub async fn zen_proxy_status(
    server: State<'_, Server>,
) -> Result<zen_proxy::ZenProxyStatus, String> {
    Ok(server.zen_proxy_status().await)
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

rpc_passthrough!(thread_start, "thread/start", Some(Duration::from_secs(60)));

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

rpc_passthrough!(thread_resume, "thread/resume", Some(Duration::from_secs(60)));

rpc_passthrough!(thread_fork, "thread/fork", Some(Duration::from_secs(60)));

#[tauri::command]
pub async fn thread_delete(
    server: State<'_, Server>,
    tasks: State<'_, TaskStore>,
    scheduler: State<'_, Scheduler>,
    thread_id: String,
) -> Result<Value, String> {
    let result = server
        .request("thread/delete", json!({ "threadId": thread_id }), None)
        .await;
    // 会话删除成功 → 级联删除其绑定的定时任务与执行记录（幂等：无任务时为空）
    if result.is_ok() {
        if let Ok(ids) = tasks.remove_by_thread(&thread_id) {
            for id in &ids {
                scheduler.purge_task(id);
            }
            if !ids.is_empty() {
                scheduler.notify_changed(None);
            }
        }
    }
    result
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

rpc_passthrough!(turn_start, "turn/start", Some(Duration::from_secs(60)));

rpc_passthrough!(turn_steer, "turn/steer", Some(Duration::from_secs(60)));

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

// ---------------------------------------------------------------------------
// 定时任务：管理与执行记录查询；创建入口为动态工具 codexui.add_scheduled_task
// 与本组命令（设置页管理操作），到点执行由 TaskScheduler 驱动。
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn scheduled_tasks_list(store: State<'_, TaskStore>) -> Result<Vec<ScheduledTask>, String> {
    Ok(store.list())
}

#[tauri::command]
pub async fn scheduled_task_add(
    scheduler: State<'_, Scheduler>,
    store: State<'_, TaskStore>,
    name: String,
    prompt: String,
    cron: String,
    thread_id: String,
    busy_policy: Option<String>,
) -> Result<ScheduledTask, String> {
    let task = store.add(
        &name,
        &prompt,
        &cron,
        &thread_id,
        busy_policy
            .as_deref()
            .unwrap_or(scheduled_tasks::BUSY_POLICY_SKIP),
    )?;
    scheduler.notify_changed(Some(&task.id));
    Ok(task)
}

#[tauri::command]
pub async fn scheduled_task_remove(
    scheduler: State<'_, Scheduler>,
    store: State<'_, TaskStore>,
    id: String,
) -> Result<bool, String> {
    let existed = store.remove(&id)?;
    if existed {
        // 级联删除已带走执行记录，清理调度器内存残留并推送快照
        scheduler.purge_task(&id);
        scheduler.notify_changed(Some(&id));
    }
    Ok(existed)
}

#[tauri::command]
pub async fn scheduled_task_set_enabled(
    scheduler: State<'_, Scheduler>,
    store: State<'_, TaskStore>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    store.set_enabled(&id, enabled)?;
    scheduler.notify_changed(Some(&id));
    Ok(())
}

#[tauri::command]
pub async fn scheduled_task_set_busy_policy(
    scheduler: State<'_, Scheduler>,
    store: State<'_, TaskStore>,
    id: String,
    policy: String,
) -> Result<(), String> {
    store.set_busy_policy(&id, &policy)?;
    scheduler.notify_changed(Some(&id));
    Ok(())
}

#[tauri::command]
pub async fn scheduled_task_update(
    scheduler: State<'_, Scheduler>,
    store: State<'_, TaskStore>,
    id: String,
    name: String,
    prompt: String,
    busy_policy: String,
) -> Result<ScheduledTask, String> {
    let task = store.update(&id, &name, &prompt, &busy_policy)?;
    scheduler.notify_changed(Some(&id));
    Ok(task)
}

#[tauri::command]
pub async fn scheduled_task_run_now(scheduler: State<'_, Scheduler>, id: String) -> Result<(), String> {
    scheduler.run_now(&id)
}

#[tauri::command]
pub async fn scheduled_task_runs(
    store: State<'_, TaskStore>,
    task_id: String,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<TaskRunRecord>, String> {
    store.list_runs(&task_id, limit.unwrap_or(20).clamp(1, 100), offset.unwrap_or(0).max(0))
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

// ---------------- 微信接入（ClawBot sidecar） ----------------

/// 当前微信桥状态快照（供会话面板与绑定弹窗轮询、事件兜底刷新）。
#[tauri::command]
pub async fn wechat_state(wechat: State<'_, WeChat>) -> Result<Value, String> {
    Ok(wechat.state().await)
}

/// 当前会话↔微信绑定列表（会话面板徽标用）。
#[tauri::command]
pub async fn wechat_bindings(wechat: State<'_, WeChat>) -> Result<Value, String> {
    Ok(wechat.bindings().await)
}

/// 对指定会话发起扫码绑定：二维码内容随后经 `wechat/event` 事件推送。
#[tauri::command]
pub async fn wechat_bind_login_start(
    wechat: State<'_, WeChat>,
    thread_id: String,
) -> Result<(), String> {
    wechat.bind_login_start(&thread_id).await
}

/// 解除指定会话的微信绑定并停止对应账号接收。
#[tauri::command]
pub async fn wechat_unbind(wechat: State<'_, WeChat>, thread_id: String) -> Result<(), String> {
    wechat.unbind(&thread_id).await
}

/// 取消当前扫码绑定（弹窗关闭时调用）。
#[tauri::command]
pub async fn wechat_cancel_bind(wechat: State<'_, WeChat>) -> Result<(), String> {
    wechat.cancel_bind().await;
    Ok(())
}

/// 读取某线程的统一会话状态（微信绑定/权限/模型/推理强度）。
#[tauri::command]
pub fn sessions_get(
    store: State<'_, Arc<SessionStateStore>>,
    thread_id: String,
) -> Option<SessionState> {
    store.get(&thread_id)
}

/// 写入某线程的权限/模型/推理强度（全量覆盖这三个字段，None 表示恢复默认，保留微信绑定）。
#[tauri::command]
pub fn sessions_update(
    store: State<'_, Arc<SessionStateStore>>,
    thread_id: String,
    permission_mode: Option<String>,
    model: Option<String>,
    effort: Option<String>,
) -> Result<(), String> {
    store.set_settings(&thread_id, permission_mode, model, effort)
}

/// 清空某线程的会话状态记录（删除会话时调用）。
#[tauri::command]
pub fn sessions_remove(
    store: State<'_, Arc<SessionStateStore>>,
    thread_id: String,
) -> Result<(), String> {
    store.remove(&thread_id)
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
pub async fn pick_directory(
    initial_dir: Option<String>,
    title: Option<String>,
) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        // 锁在阻塞任务内获取并持有到对话框关闭：与 git_op_lock 同模式，
        // 保证排队等待的后续调用在对话框真正关闭前不会并发弹窗。
        let _guard = pick_directory_lock().lock().unwrap_or_else(|e| e.into_inner());
        let mut dialog = rfd::FileDialog::new();
        if let Some(t) = title.as_deref().filter(|t| !t.is_empty()) {
            dialog = dialog.set_title(t);
        }
        let dialog = dialog_with_initial_dir(dialog, initial_dir);
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

/// 粘贴图片临时目录：`%TEMP%\codex-ui-paste`。
fn paste_image_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("codex-ui-paste")
}

/// 把图片字节写入指定目录，返回绝对路径
pub(crate) fn save_image_bytes(
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
        let primary = paste_image_dir();
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

/// 把文件路径列表以 CF_HDROP 写入系统剪贴板（供资源管理器/微信/输入框等粘贴）
#[tauri::command]
pub async fn clipboard_write_files(paths: Vec<String>) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        use clipboard_win::Setter;
        let _clip = clipboard_win::Clipboard::new_attempts(10)
            .map_err(|e| format!("打开剪贴板失败: {e}"))?;
        clipboard_win::formats::FileList
            .write_clipboard(&paths)
            .map_err(|e| format!("写入剪贴板文件失败: {e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 读取系统剪贴板文本（CF_UNICODETEXT）。经 Rust 读剪贴板，避免调用
/// navigator.clipboard.readText() 触发 WebView2「查看复制到剪贴板的文本和图像」
/// 权限确认框；剪贴板无文本时返回空串。
#[tauri::command]
pub async fn clipboard_read_text() -> Result<String, String> {
    tokio::task::spawn_blocking(|| {
        clipboard_win::get_clipboard_string().map_err(|e| format!("读取剪贴板文本失败: {e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 从图片来源取原始字节：支持 `data:` URL（base64）与本地文件路径；
/// 远程 http(s) 图片不做下载，直接给出可读错误。
fn image_bytes_from_source(source: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return Err("图片来源为空".into());
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Err("暂不支持复制网络图片，请先另存到本地".into());
    }
    if lower.starts_with("data:") {
        let Some((_, payload)) = trimmed.split_once(";base64,") else {
            return Err("暂不支持该 data URL 形式（仅支持 base64）".into());
        };
        return base64::engine::general_purpose::STANDARD
            .decode(payload.trim())
            .map_err(|e| format!("data URL 解析失败: {e}"));
    }
    std::fs::read(trimmed).map_err(|e| format!("读取图片失败 {trimmed}: {e}"))
}

/// RGBA8 像素 → 32 位 BI_RGB 的 DIB（BITMAPINFOHEADER + 自顶向下像素行）。
/// 透明像素按白底合成：多数应用忽略 32bpp 的 alpha，不合成会变成黑底。
fn rgba_to_dib(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    const HEADER_LEN: usize = 40;
    let mut out = Vec::with_capacity(HEADER_LEN + width as usize * height as usize * 4);
    out.extend_from_slice(&40u32.to_le_bytes()); // biSize
    out.extend_from_slice(&(width as i32).to_le_bytes()); // biWidth
    out.extend_from_slice(&(-(height as i32)).to_le_bytes()); // biHeight：负值 = 自顶向下
    out.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    out.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    out.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
    out.extend_from_slice(&(width * height * 4).to_le_bytes()); // biSizeImage
    out.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerMeter
    out.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerMeter
    out.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    out.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant
    for px in rgba.chunks_exact(4) {
        let alpha = px[3] as u32;
        let blend = |c: u8| -> u8 { ((c as u32 * alpha + 255 * (255 - alpha)) / 255) as u8 };
        out.push(blend(px[2])); // B
        out.push(blend(px[1])); // G
        out.push(blend(px[0])); // R
        out.push(255); // A：写不透明
    }
    out
}

/// DIB → BMP 文件字节（14 字节 BITMAPFILEHEADER + DIB），供 CF_BITMAP 兜底格式使用。
fn dib_to_bmp(dib: &[u8]) -> Vec<u8> {
    const FILE_HEADER_LEN: usize = 14;
    let mut out = Vec::with_capacity(FILE_HEADER_LEN + dib.len());
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((FILE_HEADER_LEN + dib.len()) as u32).to_le_bytes()); // bfSize
    out.extend_from_slice(&0u16.to_le_bytes()); // bfReserved1
    out.extend_from_slice(&0u16.to_le_bytes()); // bfReserved2
    out.extend_from_slice(&(FILE_HEADER_LEN as u32).to_le_bytes()); // bfOffBits
    out.extend_from_slice(dib);
    out
}

/// 把图片写入系统剪贴板（CF_DIB 主格式 + CF_BITMAP 兜底），供图片灯箱右键「复制图像」使用。
#[tauri::command]
pub async fn clipboard_write_image(source: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let bytes = image_bytes_from_source(&source)?;
        let img = image::load_from_memory(&bytes)
            .map_err(|e| format!("图片解码失败（支持 png/jpg/jpeg/webp/gif/bmp）：{e}"))?;
        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();
        if width == 0 || height == 0 {
            return Err("图片尺寸无效".into());
        }
        let dib = rgba_to_dib(width, height, rgba.as_raw());
        // 打开剪贴板（被占用时重试 10 次；Drop 时自动 close）
        let _clip = clipboard_win::Clipboard::new_attempts(10)
            .map_err(|e| format!("打开剪贴板失败: {e}"))?;
        // CF_DIB：多数应用（画图/Word/微信等）首选；set() 会先清空剪贴板
        clipboard_win::raw::set(clipboard_win::formats::CF_DIB, &dib)
            .map_err(|e| format!("写入剪贴板图像失败: {e}"))?;
        // 追加 CF_BITMAP，兼容只认 HBITMAP 的老程序（失败不阻断，DIB 已在剪贴板里）
        let _ = clipboard_win::raw::set_bitmap_with(
            &dib_to_bmp(&dib),
            clipboard_win::options::NoClear,
        );
        Ok(())
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
    settings::save(&dir, &settings)?;
    // 毛玻璃特效/主题变化即时反映到主窗口（仅 Windows 生效，失败不阻断保存）
    crate::window_glass::apply(&app, &settings);
    Ok(())
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

/// 保存 model_catalog_json 目标文件原文：目标路径从 config.toml 解析，
/// 内容必须非空且为合法 JSON。
#[tauri::command]
pub fn model_catalog_save(content: String) -> Result<(), String> {
    model_config::save_model_catalog(&content)
}

/// 从指定模型提供者的 `/models` 生成完整模型目录，供前端回填模型目录编辑框。
#[tauri::command]
pub async fn model_catalog_generate_from_provider(
    app: AppHandle,
    base_url: String,
    api_key: String,
) -> Result<model_catalog::ModelCatalogGenerateResult, String> {
    let app_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    model_catalog::generate_from_provider(&app_dir, &base_url, &api_key).await
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

/// 添加技能：打开单文件对话框（Markdown 过滤）选择 SKILL.md，校验通过后把
/// 所在文件夹复制/覆盖到 CODEX_HOME/skills；返回技能名，用户取消返回 null。
#[tauri::command]
pub async fn skills_add() -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        let dialog = rfd::FileDialog::new().add_filter("SKILL.md", &["md"]);
        let Some(picked) = dialog.pick_file() else {
            return Ok(None);
        };
        let home = model_config::codex_home()?;
        skills::install_in(&picked, &home).map(Some)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 删除技能：删除该 SKILL.md 所在目录并返回目录名。
#[tauri::command]
pub fn skills_remove(skill_path: String) -> Result<String, String> {
    skills::remove_in(std::path::Path::new(&skill_path))
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
    fn image_bytes_from_source_reads_data_url_and_path_rejects_remote() {
        use base64::Engine as _;
        // data URL（base64）
        let payload = base64::engine::general_purpose::STANDARD.encode(b"png-bytes");
        let bytes =
            image_bytes_from_source(&format!("data:image/png;base64,{payload}")).unwrap();
        assert_eq!(bytes, b"png-bytes");
        // 本地路径
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("a.png");
        std::fs::write(&file, b"file-bytes").unwrap();
        assert_eq!(
            image_bytes_from_source(&file.to_string_lossy()).unwrap(),
            b"file-bytes"
        );
        // 远程图片与非法输入
        assert!(image_bytes_from_source("https://x/a.png")
            .unwrap_err()
            .contains("暂不支持复制网络图片"));
        assert!(image_bytes_from_source("data:image/svg+xml,<svg/>")
            .unwrap_err()
            .contains("仅支持 base64"));
        assert!(image_bytes_from_source("  ").unwrap_err().contains("为空"));
        assert!(image_bytes_from_source(&dir.path().join("missing.png").to_string_lossy())
            .unwrap_err()
            .contains("读取图片失败"));
    }

    #[test]
    fn rgba_to_dib_writes_header_and_composites_transparency() {
        // 2x1：不透明红 + 全透明（应合成成白）
        let rgba = [255u8, 0, 0, 255, 0, 0, 0, 0];
        let dib = rgba_to_dib(2, 1, &rgba);
        assert_eq!(dib.len(), 40 + 2 * 4);
        let u32_at = |off: usize| u32::from_le_bytes(dib[off..off + 4].try_into().unwrap());
        let i32_at = |off: usize| i32::from_le_bytes(dib[off..off + 4].try_into().unwrap());
        assert_eq!(u32_at(0), 40, "biSize");
        assert_eq!(i32_at(4), 2, "biWidth");
        assert_eq!(i32_at(8), -1, "biHeight 负值 = 自顶向下");
        assert_eq!(u16::from_le_bytes(dib[12..14].try_into().unwrap()), 1, "biPlanes");
        assert_eq!(u16::from_le_bytes(dib[14..16].try_into().unwrap()), 32, "biBitCount");
        assert_eq!(u32_at(16), 0, "biCompression = BI_RGB");
        assert_eq!(u32_at(20), 2 * 1 * 4, "biSizeImage");
        // 像素：BGR 顺序 + alpha 置 255
        assert_eq!(&dib[40..44], &[0, 0, 255, 255], "不透明红 → BGR");
        assert_eq!(&dib[44..48], &[255, 255, 255, 255], "全透明 → 白底");
    }

    #[test]
    fn dib_to_bmp_prepends_file_header() {
        let bmp = dib_to_bmp(&[1, 2, 3, 4]);
        assert_eq!(&bmp[0..2], b"BM");
        assert_eq!(
            u32::from_le_bytes(bmp[2..6].try_into().unwrap()),
            14 + 4,
            "bfSize"
        );
        assert_eq!(
            u32::from_le_bytes(bmp[10..14].try_into().unwrap()),
            14,
            "bfOffBits"
        );
        assert_eq!(&bmp[14..], &[1, 2, 3, 4]);
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
