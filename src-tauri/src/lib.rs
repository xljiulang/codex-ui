pub mod codex;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[cfg(windows)]
use tauri::AppHandle;
use tauri::webview::PageLoadEvent;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tauri::utils::config::Color;

use codex::app_server::CodexServer;

/// 主窗口一次性显示守卫：页面加载完成后首次触发，避免重复导航反复处理
static MAIN_WINDOW_SHOWN: AtomicBool = AtomicBool::new(false);

/// 主窗口 F1-F12 默认浏览器行为拦截：
/// WebView2 会把无修饰键的功能键（F5 刷新、F1 帮助、F11 全屏、F12 开发者工具等）当
/// 成浏览器快捷键直接处理，导致界面刷新或弹出浏览器功能。这里注册
/// AcceleratorKeyPressed 事件，把 F1-F12（虚拟键码 0x70-0x7B）标记为已处理，
/// 阻止默认行为；Ctrl/Alt/Shift 组合下的功能键（如 Alt+F4）不拦截，保留系统快捷键。
#[cfg(windows)]
fn disable_main_window_function_keys(app: &AppHandle) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN, COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN,
    };
    use webview2_com::AcceleratorKeyPressedEventHandler;

    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let _ = window.with_webview(|webview| {
        let controller = webview.controller();
        let handler = AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else {
                return Ok(());
            };
            unsafe {
                let mut kind = COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN;
                let _ = args.KeyEventKind(&mut kind);
                if kind == COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
                    || kind == COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN
                {
                    let mut key = 0u32;
                    let _ = args.VirtualKey(&mut key);
                    // VK_F1(0x70) ..= VK_F12(0x7B)
                    if (0x70..=0x7B).contains(&key) {
                        let _ = args.SetHandled(true);
                    }
                }
            }
            Ok(())
        }));
        // token 仅用于后续 remove_AcceleratorKeyPressed；handler 由事件注册持有
        let mut token = 0i64;
        unsafe {
            let _ = controller.add_AcceleratorKeyPressed(&handler, &mut token);
        }
    });
}

pub fn run() {
    tauri::Builder::default()
        .on_page_load(|webview, payload| {
            // 主窗口页面加载完成（Vue 已挂载、加载动画已渲染）后再显示，
            // 避免窗体到 WebView 初始化完成之间的默认色闪屏。
            if webview.label() == "main"
                && payload.event() == PageLoadEvent::Finished
                && !MAIN_WINDOW_SHOWN.swap(true, Ordering::SeqCst)
            {
                let _ = webview.window().show();
            }
        })
        .register_asynchronous_uri_scheme_protocol("print-export", |ctx, request, responder| {
            // 隐藏打印窗口经 print-export://localhost/<id> 加载暂存的打印 HTML
            let id = request.uri().path().trim_start_matches('/').to_string();
            let state = ctx
                .app_handle()
                .state::<codex::pdf_export::PdfExportState>();
            let html = state
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&id)
                .cloned();
            match html {
                Some(body) => responder.respond(
                    tauri::http::Response::builder()
                        .header(
                            tauri::http::header::CONTENT_TYPE,
                            "text/html; charset=utf-8",
                        )
                        .body(body.into_bytes())
                        .unwrap(),
                ),
                None => responder.respond(
                    tauri::http::Response::builder()
                        .status(tauri::http::StatusCode::BAD_REQUEST)
                        .body("print content not found".to_string().into_bytes())
                        .unwrap(),
                ),
            }
        })
        .invoke_handler(tauri::generate_handler![
            codex::commands::test_hook_enabled,
            codex::commands::server_status,
            codex::commands::server_connect,
            codex::commands::server_logs,
            codex::commands::session_log,
            codex::commands::codex_rpc,
            codex::commands::codex_rpc_long,
            codex::commands::codex_pinned_section_id,
            codex::commands::interaction_respond,
            codex::commands::thread_list,
            codex::commands::thread_start,
            codex::commands::thread_read,
            codex::commands::thread_resume,
            codex::commands::thread_delete,
            codex::commands::thread_set_name,
            codex::commands::turn_start,
            codex::commands::turn_steer,
            codex::commands::turn_interrupt,
            codex::commands::goal_set,
            codex::commands::goal_get,
            codex::commands::goal_clear,
            codex::commands::auth_status,
            codex::commands::auth_login,
            codex::commands::auth_api_key_configured,
            codex::commands::auth_logout,
            codex::commands::startup_workspace,
            codex::commands::wechat_state,
            codex::commands::wechat_login_start,
            codex::commands::wechat_logout,
            codex::commands::wechat_service_sync,
            codex::commands::open_url,
            codex::commands::reveal_path,
            codex::diff::build_diff_preview,
            codex::commands::pick_files,
            codex::commands::pick_directory,
            codex::commands::pick_codex_file,
            codex::commands::save_pasted_image,
            codex::commands::clipboard_file_paths,
            codex::commands::settings_get,
            codex::commands::settings_set,
            codex::commands::model_config_read,
            codex::commands::model_config_save,
            codex::commands::model_catalog_save,
            codex::commands::model_catalog_target_exists,
            codex::commands::skills_read,
            codex::commands::custom_instructions_read,
            codex::commands::custom_instructions_save,
            codex::session_fs::session_fs_list,
            codex::session_fs::session_fs_search,
            codex::session_fs::session_fs_metadata,
            codex::session_fs::session_fs_rename,
            codex::session_fs::session_fs_delete,
            codex::session_fs::session_fs_copy,
            codex::session_fs::session_fs_paste,
            codex::session_fs::session_fs_move,
            codex::session_fs::session_fs_watch_start,
            codex::session_fs::session_fs_watch_stop,
            codex::session_fs::session_fs_read,
            codex::session_fs::session_fs_write,
            codex::session_fs::session_fs_write_bytes,
            codex::session_fs::session_fs_create_file,
            codex::session_fs::session_fs_create_dir,
            codex::session_fs::session_fs_probe_text,
            codex::session_fs::session_fs_read_bytes,
            codex::session_fs::session_fs_icons,
            codex::session_fs::session_fs_icon_for_ext,
            codex::pdf_export::export_markdown_pdf,
            codex::terminal::terminal_spawn,
            codex::terminal::terminal_write,
            codex::terminal::terminal_resize,
            codex::terminal::terminal_kill,
            codex::git::git_changes_status,
            codex::git::git_changes_init,
            codex::git::git_changes_commit,
            codex::git::git_changes_pull,
            codex::git::git_changes_push,
            codex::git::git_changes_remotes,
            codex::git::git_changes_remote_add,
            codex::git::git_changes_remote_set_url,
            codex::git::git_changes_remote_remove,
            codex::git::git_changes_remote_fetch,
            codex::git::git_changes_branch_checkout_remote,
            codex::git::git_changes_remote_branch_delete,
            codex::git::git_changes_remote_switch_upstream,
            codex::git::git_changes_git_available,
            codex::git::git_changes_branches,
            codex::git::git_changes_branch_create,
            codex::git::git_changes_branch_delete,
            codex::git::git_changes_branch_switch,
            codex::git::git_changes_branch_merge,
            codex::git::git_changes_log,
            codex::git::git_changes_commit_detail,
            codex::git::git_changes_commit_file_diff,
            codex::git::git_changes_diff,
            codex::git::git_changes_stage,
            codex::git::git_changes_unstage,
            codex::git::git_changes_stage_all,
            codex::git::git_changes_unstage_all,
            codex::git::git_changes_restore,
            codex::git::git_changes_delete,
            codex::git::git_changes_ignore,
            codex::git::git_changes_watch_start,
            codex::git::git_changes_watch_stop,
        ])
        .setup(|app| {
            // 启动时探测一次系统 git 并缓存（`git --version`），后续全部 git 功能复用该结果
            codex::git::probe_git_at_startup();

            // 先读取已保存主题，再以对应背景色创建主窗口：
            // 窗口从创建那一刻起颜色即与主题一致，避免首帧错色/闪色（缺省按 blue）。
            let app_dir = app.path().app_data_dir().ok();
            let theme = app_dir
                .as_deref()
                .map(codex::settings::load)
                .unwrap_or_default()
                .theme;
            let (r, g, b, a) = codex::settings::theme_background_rgba(&theme);
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title(format!("Codex UI v{}", env!("CARGO_PKG_VERSION")))
                .inner_size(1280.0, 720.0)
                .min_inner_size(400.0, 560.0)
                .resizable(true)
                .center()
                .visible(false)
                .background_color(Color(r, g, b, a))
                .build()
                .map_err(|e| {
                    eprintln!("创建主窗口失败: {e}");
                    format!("创建主窗口失败: {e}")
                })?;

            // 拦截主窗口 F1-F12 的 WebView2 默认行为（F5 刷新等）
            disable_main_window_function_keys(app.handle());

            // 兜底：页面加载失败/卡死时，窗口也能在 5 秒后出现（对已显示窗口是空操作）
            let fallback_handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(5));
                if let Some(win) = fallback_handle.get_webview_window("main") {
                    let _ = win.show();
                }
            });

            let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let server = Arc::new(CodexServer::new(app.handle().clone(), workspace));
            let server_handle = server.clone();
            app.manage(server);
            // 微信接入桥：数据根目录随应用数据目录；开关开启时后台自动恢复长轮询
            let wechat_app_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
            let wechat = codex::wechat_bridge::WeChatBridge::new(
                app.handle().clone(),
                server_handle.clone(),
                wechat_app_dir,
            );
            app.manage(wechat.clone());
            let wechat_boot = wechat.clone();
            tauri::async_runtime::spawn(async move {
                wechat_boot.autostart().await;
            });
            app.manage(codex::pdf_export::PdfExportState(
                std::sync::Mutex::new(std::collections::HashMap::new()),
            ));
            app.manage(codex::session_fs::FsWatcherState(std::sync::Mutex::new(None)));
            app.manage(codex::git::GitWatcherState(std::sync::Mutex::new(None)));
            app.manage(codex::terminal::TerminalState(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )));
            server_handle.ensure_running();
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(server) = app_handle.try_state::<Arc<CodexServer>>() {
                    server.shutdown();
                }
                if let Some(wechat) =
                    app_handle.try_state::<Arc<codex::wechat_bridge::WeChatBridge>>()
                {
                    tauri::async_runtime::block_on(wechat.stop_service());
                }
            }
        });
}
