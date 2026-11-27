//! The Setu desktop app: a Tauri 2 shell around the Rust core.
//!
//! The web view only renders. Shells, SSH sessions, saved hosts and the
//! vault run in Rust and talk to the UI through the commands in [`terminals`],
//! [`ssh`], [`files`], [`hosts`], [`vault`], [`workflows`], [`tunnels`], [`monitor`], [`server`], [`problems`], [`memory`] and [`session`].

mod color_schemes;
mod extensions;
mod files;
mod hosts;
mod keys;
mod memory;
mod monitor;
mod nettools;
mod packs;
mod problems;
mod prompts;
mod remote_edit;
mod server;
mod session;
mod ssh;
mod terminals;
mod tunnels;
mod vault;
mod windows;
mod workflows;

use serde::Serialize;
pub use setu_core::extend::Extensions;

#[derive(Debug, Serialize)]
struct AppInfo {
    version: &'static str,
    os: &'static str,
    /// Local login name, the default SSH user (as in OpenSSH).
    user: String,
    /// The edition a build with extensions runs as ("Enterprise"), if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    edition: Option<String>,
}

#[tauri::command]
fn app_info(extensions: tauri::State<'_, Extensions>) -> AppInfo {
    AppInfo {
        version: setu_core::VERSION,
        os: std::env::consts::OS,
        user: ssh::local_user(),
        edition: extensions.edition(),
    }
}

/// Starts the desktop app as the open-source edition: no extensions.
pub fn run() {
    run_with(Extensions::default());
}

/// Starts the desktop app with `extensions` (an audit trail, a command
/// policy). Builds that add their own hooks call this from `main`.
#[allow(clippy::expect_used)]
pub fn run_with(extensions: Extensions) {
    tauri::Builder::default()
        // Rust-only: no capability grants the web view its commands.
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .manage(extensions)
        .manage(terminals::Terminals::default())
        .manage(hosts::Hosts::from_env())
        .manage(vault::VaultState::from_env())
        .manage(files::Files::default())
        .manage(remote_edit::RemoteEdits::default())
        .manage(session::SessionFile::from_env())
        .manage(session::SettingsFile::from_env())
        .manage(windows::Detached::default())
        .manage(ssh::OfferedKeys::default())
        .manage(workflows::Workflows::from_env())
        .manage(memory::Memory::from_env())
        .manage(tunnels::Tunnels::from_env())
        .manage(monitor::Monitors::default())
        .manage(server::ServerSessions::default())
        .manage(problems::Problems::from_env())
        .setup(|app| {
            use tauri::{Emitter, Manager};
            if let Some(edition) = app.state::<Extensions>().edition() {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.set_title(&format!("Setu {edition}"));
                }
            }
            let vault = app.state::<vault::VaultState>().inner().clone();
            vault.spawn_auto_lock(app.handle().clone());
            // Unlock from the OS keychain off the main thread (the keychain
            // may be slow or show its own prompt), then tell the UI.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                vault.startup_unlock();
                let _ = handle.emit(vault::STATUS_EVENT, vault.status());
                // Tunnels that start with Setu may need the vault's secrets.
                handle.state::<tunnels::Tunnels>().start_auto(
                    &handle.state::<hosts::Hosts>(),
                    &vault,
                    &handle.state::<Extensions>(),
                );
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // A closed window ends the shells it was showing.
            if let tauri::WindowEvent::Destroyed = event {
                use tauri::Manager;
                window
                    .state::<terminals::Terminals>()
                    .close_shown_in(window.label());
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            extensions::extension_panel,
            extensions::extension_action,
            terminals::list_shells,
            terminals::open_terminal,
            terminals::write_terminal,
            terminals::resize_terminal,
            terminals::close_terminal,
            terminals::terminal_hold,
            terminals::terminal_attach,
            windows::window_detach,
            windows::window_take,
            ssh::ssh_connect,
            ssh::ssh_open_shell,
            ssh::ssh_trust_host,
            keys::ssh_keys_list,
            keys::ssh_key_generate,
            keys::ssh_key_add_to_agent,
            keys::ssh_key_load_resident,
            keys::known_hosts_list,
            keys::known_hosts_remove,
            hosts::hosts_list,
            hosts::host_save,
            hosts::host_delete,
            hosts::ssh_config_scan,
            hosts::ssh_config_import,
            hosts::hosts_parse_file,
            color_schemes::color_scheme_parse,
            prompts::prompt_detect,
            packs::packs_list,
            packs::pack_inspect,
            packs::pack_install,
            packs::pack_remove,
            vault::vault_status,
            vault::vault_create,
            vault::vault_unlock,
            vault::vault_lock,
            vault::vault_remember,
            vault::vault_remove,
            vault::vault_change_password,
            session::session_load,
            session::session_save,
            session::session_clear,
            session::settings_load,
            session::settings_save,
            files::files_open,
            files::files_close,
            files::files_list,
            files::files_mkdir,
            files::files_rename,
            files::files_remove,
            files::local_list,
            files::transfer,
            files::transfer_cancel,
            files::transfer_pause,
            files::files_stat,
            files::files_set_attributes,
            remote_edit::remote_edit_open,
            remote_edit::remote_edit_push,
            remote_edit::remote_edit_stop,
            workflows::workflows_list,
            workflows::workflow_save,
            workflows::workflow_delete,
            workflows::workflow_run,
            workflows::workflow_answer,
            workflows::workflow_cancel,
            workflows::workflow_history,
            workflows::workflow_history_clear,
            workflows::board_load,
            workflows::board_save,
            memory::memory_view,
            memory::memory_record,
            memory::memory_settings,
            memory::memory_delete,
            memory::memory_clear,
            memory::memory_dismiss,
            memory::memory_snippet_save,
            memory::memory_snippet_delete,
            memory::memory_workflow_draft,
            memory::memory_summary,
            tunnels::tunnels_list,
            tunnels::tunnel_save,
            tunnels::tunnel_delete,
            tunnels::tunnel_start,
            tunnels::tunnel_stop,
            nettools::net_check_port,
            nettools::net_lookup,
            nettools::net_ping,
            nettools::net_wake,
            nettools::net_tls,
            monitor::monitor_start,
            monitor::monitor_stop,
            problems::problems_list,
            problems::problems_check,
            problems::problems_snooze,
            problems::problems_unsnooze,
            server::server_services,
            server::server_service_details,
            server::server_service_logs,
            server::server_service_command,
            server::server_service_action,
            server::server_keys,
            server::server_key_check,
            server::server_key_add,
            server::server_key_remove,
            server::server_disconnect,
            server::server_processes,
            server::server_process_command,
            server::server_process_signal,
            server::server_disks,
            server::server_folder_sizes,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Setu desktop app");
}
