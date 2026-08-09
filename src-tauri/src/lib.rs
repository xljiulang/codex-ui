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
            codex::commands::read_file,
            codex::commands::pick_files,
            codex::commands::pick_directory,
            codex::commands::settings_get,
            codex::commands::settings_set,
        ])
        .setup(|app| {
            let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let server = Arc::new(CodexServer::new(app.handle().clone(), workspace));
            let server_handle = server.clone();
            app.manage(server);
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
