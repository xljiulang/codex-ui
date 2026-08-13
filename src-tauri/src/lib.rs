pub mod codex;

use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager;

use codex::app_server::CodexServer;

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            codex::commands::server_status,
            codex::commands::server_connect,
            codex::commands::server_logs,
            codex::commands::codex_rpc,
            codex::commands::codex_rpc_long,
            codex::commands::codex_pin_capability,
            codex::commands::codex_title_helper_capability,
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
            codex::commands::workspace_dir,
            codex::commands::open_url,
            codex::commands::reveal_path,
            codex::diff::build_diff_preview,
            codex::diff::open_diff_window,
            codex::diff::take_diff_params,
            codex::commands::pick_files,
            codex::commands::pick_directory,
            codex::commands::pick_codex_file,
            codex::commands::save_pasted_image,
            codex::commands::clipboard_file_paths,
            codex::commands::settings_get,
            codex::commands::settings_set,
            codex::session_fs::session_fs_list,
            codex::session_fs::session_fs_search,
            codex::session_fs::session_fs_metadata,
            codex::session_fs::session_fs_rename,
            codex::session_fs::session_fs_delete,
            codex::session_fs::session_fs_copy,
            codex::session_fs::session_fs_paste,
            codex::session_fs::session_fs_watch_start,
            codex::session_fs::session_fs_watch_stop,
            codex::session_fs::session_fs_read,
            codex::session_fs::session_fs_probe_text,
            codex::text_preview::open_text_preview,
            codex::text_preview::take_text_preview_params,
            codex::git::git_changes_status,
            codex::git::git_changes_init,
            codex::git::git_changes_commit,
            codex::git::git_changes_pull,
            codex::git::git_changes_branches,
            codex::git::git_changes_branch_create,
            codex::git::git_changes_branch_delete,
            codex::git::git_changes_branch_switch,
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
            let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let server = Arc::new(CodexServer::new(app.handle().clone(), workspace));
            let server_handle = server.clone();
            app.manage(server);
            app.manage(codex::diff::DiffParamsState(std::sync::Mutex::new(None)));
            app.manage(codex::session_fs::FsWatcherState(std::sync::Mutex::new(None)));
            app.manage(codex::text_preview::TextPreviewParamsState(std::sync::Mutex::new(None)));
            app.manage(codex::git::GitWatcherState(std::sync::Mutex::new(None)));
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
            }
        });
}
