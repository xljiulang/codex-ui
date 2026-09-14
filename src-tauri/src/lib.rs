pub mod codex;
pub(crate) mod window_glass;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[cfg(windows)]
use tauri::AppHandle;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::PageLoadEvent;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri::utils::config::Color;

use codex::app_server::CodexServer;

/// 主窗口一次性显示守卫：页面加载完成后首次触发，避免重复导航反复处理
static MAIN_WINDOW_SHOWN: AtomicBool = AtomicBool::new(false);
/// 用户是否已把主窗口关闭到系统托盘：置真后，启动期的自动显示（页面加载完成 / 5 秒兜底）
/// 不再把窗口再弹出来，直到用户从托盘主动恢复（show_main 复位）。
/// 用于修复「启动后立即关到托盘，初始化完成又把窗体显示回去」的 bug。
static USER_HIDDEN_TO_TRAY: AtomicBool = AtomicBool::new(false);
/// 是否允许退出：仅托盘「退出」置真；普通关窗（隐藏）不退出应用。
pub(crate) static ALLOW_EXIT: AtomicBool = AtomicBool::new(false);
/// 托盘「退出」后前端安全收尾的兜底时长（秒）：超时仍未退出则强制退出。
const EXIT_GRACE_SECONDS: u64 = 8;

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

/// 显示并聚焦主窗口（托盘左键 / 菜单「显示主窗口」共用）。
fn show_main(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        // 用户从托盘主动恢复窗口：复位「用户隐藏到托盘」标记，
        // 避免后续任何自动显示逻辑误判为不应显示。
        USER_HIDDEN_TO_TRAY.store(false, Ordering::SeqCst);
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
    // 隐藏到托盘后再显示：Windows 任务栏按钮重建会丢失进度条动画，
    // 通知前端按当前工作态重新应用任务栏进度。
    let _ = app.emit("taskbar-progress-refresh", ());
}

pub fn run() {
    // 尽早声明进程 AUMID（安装版），确保定时任务 toast 按钮的前台激活投递到本运行进程，
    // 而非经快捷方式拉起新实例（那样会丢失点击上下文，无法聚焦并打开绑定会话）。
    codex::scheduled_tasks::ensure_process_app_user_model_id();

    let builder = tauri::Builder::default()
        .on_page_load(|webview, payload| {
            // 主窗口页面加载完成（Vue 已挂载、加载动画已渲染）后再显示，
            // 避免窗体到 WebView 初始化完成之间的默认色闪屏。
            if webview.label() == "main"
                && payload.event() == PageLoadEvent::Finished
                && !MAIN_WINDOW_SHOWN.swap(true, Ordering::SeqCst)
                && !USER_HIDDEN_TO_TRAY.load(Ordering::SeqCst)
            {
                let window = webview.window();
                // Tauri v2 Windows 透明窗口激活：创建后先把尺寸归零再还原，
                // 否则 WebView 透明可能不生效（页面看起来仍是不透明背景）。
                let size = window.inner_size().ok();
                let _ = window.set_size(tauri::Size::Physical(tauri::PhysicalSize::new(0, 0)));
                if let Some(size) = size {
                    let _ = window.set_size(tauri::Size::Physical(size));
                }
                let _ = window.show();
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
            codex::commands::app_exit,
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
            codex::commands::thread_fork,
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
            codex::commands::wechat_state,
            codex::commands::wechat_bindings,
            codex::commands::wechat_bind_login_start,
            codex::commands::wechat_unbind,
            codex::commands::wechat_cancel_bind,
            codex::commands::sessions_get,
            codex::commands::sessions_update,
            codex::commands::sessions_remove,
            codex::commands::open_url,
            codex::commands::reveal_path,
            codex::diff::build_diff_preview,
            codex::commands::pick_files,
            codex::commands::pick_directory,
            codex::commands::pick_codex_file,
            codex::commands::save_pasted_image,
            codex::commands::clipboard_file_paths,
            codex::commands::clipboard_write_files,
            codex::commands::clipboard_read_text,
            codex::commands::clipboard_write_image,
            codex::commands::settings_get,
            codex::commands::settings_set,
            codex::commands::zen_proxy_apply,
            codex::commands::zen_proxy_status,
            codex::commands::scheduled_tasks_list,
            codex::commands::scheduled_task_add,
            codex::commands::scheduled_task_remove,
            codex::commands::scheduled_task_set_enabled,
            codex::commands::scheduled_task_set_busy_policy,
            codex::commands::scheduled_task_update,
            codex::commands::scheduled_task_run_now,
            codex::commands::scheduled_task_runs,
              codex::commands::model_config_read,
              codex::commands::model_config_save,
              codex::commands::model_catalog_save,
              codex::commands::model_catalog_generate_from_provider,
              codex::model_snapshots::model_snapshots_list,
              codex::model_snapshots::model_snapshots_save,
              codex::model_snapshots::model_snapshots_apply,
              codex::model_snapshots::model_snapshots_delete,
              codex::model_snapshots::model_snapshots_open,
              codex::commands::skills_read,
            codex::commands::skills_add,
            codex::commands::skills_remove,
            codex::commands::custom_instructions_read,
            codex::commands::custom_instructions_save,
            codex::browser_bridge::browser_bridge_repair,
            codex::browser_bridge::browser_bridge_status,
            codex::browser_bridge::browser_bridge_stop,
            codex::session_fs::session_fs_list,
            codex::session_fs::session_fs_search,
            codex::session_fs::session_fs_rg_status,
            codex::session_fs::session_fs_search_rg,
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
        ]);

        // 单实例仅 release 打包生效：同一时刻只允许一个实例；重复启动时聚焦已有窗口。
        // 定时任务 toast 的前台激活对未打包 Win32 应用，可能由系统按 AUMID 经快捷方式
        // 二次启动本 exe 并透传激活串（协议串 `codexui://open-session/<threadId>`），
        // 由此回调在「运行中的首实例」里解析参数并打开绑定会话。dev（debug_assertions）
        // 不注册，允许多开，便于调试/热更。
        #[cfg(not(debug_assertions))]
        let builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // 无论是否命中，都把二次启动收到的参数记入会话日志，便于诊断 toast 点击路由。
            if let Some(server) = app.try_state::<Arc<codex::app_server::CodexServer>>() {
                server.session_log(
                    "info".into(),
                    None,
                    "sched-toast-relaunch".into(),
                    Some(format!(
                        "args={}",
                        args.iter()
                            .map(|s| s.replace(' ', "\\ "))
                            .collect::<Vec<_>>()
                            .join(" ")
                    ))
                    .into(),
                );
            }
            if let Some(thread) = codex::scheduled_tasks::parse_activation_thread(&args) {
                if let Some(server) = app.try_state::<Arc<codex::app_server::CodexServer>>() {
                    server.session_log(
                        "info".into(),
                        None,
                        "sched-toast-open-session".into(),
                        Some(format!("via=relaunch thread={thread}")).into(),
                    );
                }
                show_main(app);
                let _ = app.emit("scheduled-task-notification-open", &thread);
                return;
            }
            show_main(app);
        }));

        builder
        .setup(|app| {
            // 启动时探测一次系统 git 并缓存（`git --version`），后续全部 git 功能复用该结果
            codex::git::probe_git_at_startup();

            // 启动时后台刷新各模型元数据源缓存（OpenRouter / models.dev）；
            // 失败不影响启动，生成时回退内置资源。
            if let Ok(model_source_app_dir) = app.path().app_data_dir() {
                tauri::async_runtime::spawn(async move {
                    codex::model_catalog::refresh_source_caches(&model_source_app_dir)
                        .await;
                });
            }

            // 读取已保存设置：主题决定 Mica 深浅/回退着色，毛玻璃特效决定是否
            // 应用系统背景。窗口常驻透明，启动期由「页面加载完成后再显示」避免
            // 空窗透明/错色（缺省按 blue + 毛玻璃开启）。
            let app_dir = app.path().app_data_dir().ok();
            let loaded = app_dir
                .as_deref()
                .map(codex::settings::load)
                .unwrap_or_default();
            let main_window = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title(format!("Codex UI v{}", env!("CARGO_PKG_VERSION")))
                .inner_size(1280.0, 800.0)
                .min_inner_size(400.0, 560.0)
                .resizable(true)
                // 自绘标题栏：隐藏原生标题栏与最小化/最大化/关闭按钮，
                // 改由前端 AppHeader 标题栏提供拖动与窗口控制（关闭仍走下方
                // on_window_event 的“关闭→隐藏到系统托盘”逻辑）。
                .decorations(false)
                // 毛玻璃特效：窗口透明以便露出 Windows Mica/Acrylic；关闭开关时
                // 由前端纯色根背景兜底，视觉等同关闭（透明会失去系统阴影）。
                .transparent(true)
                // 无边框透明窗口与默认阴影冲突：禁用后透明才真正生效
                .shadow(false)
                .center()
                .visible(false)
                .background_color(Color(0, 0, 0, 0))
                .build()
                .map_err(|e| {
                    eprintln!("创建主窗口失败: {e}");
                    format!("创建主窗口失败: {e}")
                })?;

            // 关闭 → 隐藏：取消关闭并隐藏到系统托盘（codex/微信不随窗口关闭而停）。
            // 直接隐藏而不等待前端：前端关闭守卫只负责 preventDefault 阻止默认销毁，
            // 隐藏时窗口不销毁，工作会话/未保存内容仍驻留内存，无需确认或清理。
            {
                let app_handle = app.handle().clone();
                main_window.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        // 用户主动关闭 → 隐藏到系统托盘，并记录以免启动期自动显示再弹回
                        USER_HIDDEN_TO_TRAY.store(true, Ordering::SeqCst);
                        if let Some(win) = app_handle.get_webview_window("main") {
                            let _ = win.hide();
                        }
                    }
                });
            }

            // 系统托盘：常驻；左键显示主窗口，右键菜单仅「退出」。
            {
                let quit = MenuItemBuilder::with_id("quit", "退出").build(app)?;
                let menu = MenuBuilder::new(app).item(&quit).build()?;
                let icon = app
                    .default_window_icon()
                    .cloned()
                    .ok_or_else(|| "缺少应用图标，无法创建系统托盘".to_string())?;
                TrayIconBuilder::with_id("main-tray")
                    .icon(icon)
                    .tooltip(format!("Codex UI v{}", env!("CARGO_PKG_VERSION")))
                    .menu(&menu)
                    .show_menu_on_left_click(false)
                    .on_menu_event(|app, event| {
                        // 托盘「退出」：先通知前端做安全收尾（停止工作会话/终止运行中终端），
                        // 由前端调用 app_exit 真正退出；WebView 无响应时用兜底超时强制退出。
                        if event.id().0.as_str() == "quit" {
                            let _ = app.emit("app-exit-requested", ());
                            let handle = app.clone();
                            std::thread::spawn(move || {
                                std::thread::sleep(Duration::from_secs(EXIT_GRACE_SECONDS));
                                crate::ALLOW_EXIT.store(true, Ordering::SeqCst);
                                handle.exit(0);
                            });
                        }
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            show_main(tray.app_handle());
                        }
                    })
                    .build(app)?;
            }

            // 拦截主窗口 F1-F12 的 WebView2 默认行为（F5 刷新等）
            disable_main_window_function_keys(app.handle());

            // 启动即按保存设置应用毛玻璃（Mica，非 Win11 回退 Acrylic）
            window_glass::apply(app.handle(), &loaded);

            // Windows 11 原生圆角：保留无阴影透明的同时由 DWM 裁剪四角
            #[cfg(windows)]
            window_glass::apply_round_corners(&main_window);

            // 兜底：页面加载失败/卡死时，窗口也能在 5 秒后出现（对已显示窗口是空操作）
            let fallback_handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(5));
                if let Some(win) = fallback_handle.get_webview_window("main") {
                    // 用户若已在此前关闭到托盘，则不再把窗口弹回
                    if !USER_HIDDEN_TO_TRAY.load(Ordering::SeqCst) {
                        let _ = win.show();
                    }
                }
            });

            let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let server = Arc::new(CodexServer::new(app.handle().clone(), workspace));
            let server_handle = server.clone();
            app.manage(server);

            // 启动时后台导出本机 codex 自带的官方条目（GPT/codex 基线条目随 codex 版本变化，
            // 不再内置在 resources/official-models.json）；失败保留上次缓存并记 warn。
            if let Some(export_dir) = app_dir.clone() {
                let export_server = server_handle.clone();
                let export_settings = loaded.clone();
                tauri::async_runtime::spawn(async move {
                    let codex = match codex::app_server::find_codex_sync(&export_settings) {
                        Ok(path) => path,
                        Err(err) => {
                            export_server
                                .push_log("warn", format!("导出官方模型条目失败：{err}"))
                                .await;
                            return;
                        }
                    };
                    if let Err(err) =
                        codex::model_catalog::refresh_codex_models(&export_dir, &codex).await
                    {
                        export_server
                            .push_log("warn", format!("导出官方模型条目失败：{err}"))
                            .await;
                    }
                });
            }

            // 微信接入桥：数据根目录随应用数据目录；存在绑定时后台自动恢复长轮询
            let wechat_app_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
            let session_store = Arc::new(
                codex::session_state::SessionStateStore::new(&wechat_app_dir)
                    .map_err(|e| e.to_string())?,
            );
            app.manage(session_store.clone());
            // 定时任务：自有 SQLite 库 + 调度器（到点在绑定会话内自动发回合）
            let task_store = Arc::new(
                codex::scheduled_tasks::ScheduledTaskStore::open(&wechat_app_dir)
                    .map_err(|e| e.to_string())?,
            );
            app.manage(task_store.clone());
            let scheduler = Arc::new(codex::scheduled_tasks::TaskScheduler::new(
                app.handle().clone(),
                server_handle.clone(),
                task_store.clone(),
                session_store.clone(),
                wechat_app_dir.clone(),
            ));
            app.manage(scheduler.clone());
            scheduler.start();
            let wechat = codex::wechat_bridge::WeChatBridge::new(
                app.handle().clone(),
                server_handle.clone(),
                wechat_app_dir,
                session_store,
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
            // 最后一个窗口关闭会触发 ExitRequested：普通关窗仅隐藏，不允许退出；
            // 仅当托盘「退出」置位 ALLOW_EXIT 后才放行。
            if let tauri::RunEvent::ExitRequested { ref api, .. } = event {
                if !ALLOW_EXIT.load(Ordering::SeqCst) {
                    api.prevent_exit();
                }
            }
            if let tauri::RunEvent::Exit = event {
                // 收尾：终止所有运行中终端（ConPTY 子进程），避免 cmd/powershell 孤儿残留
                if let Some(terminals) = app_handle.try_state::<codex::terminal::TerminalState>() {
                    codex::terminal::kill_all(&terminals);
                }
                if let Some(server) = app_handle.try_state::<Arc<CodexServer>>() {
                    server.shutdown();
                }
                if let Some(wechat) =
                    app_handle.try_state::<Arc<codex::wechat_bridge::WeChatBridge>>()
                {
                    // 兜底：桥异常挂起时也保证应用能退出（超时后不再等待微信收尾）。
                    tauri::async_runtime::block_on(async {
                        let _ =
                            tokio::time::timeout(Duration::from_secs(5), wechat.shutdown()).await;
                    });
                }
            }
        });
}
