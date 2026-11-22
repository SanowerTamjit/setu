//! The Setu desktop app: a Tauri 2 shell around the Rust core.
//!
//! The web view only renders. Shells, SSH sessions, saved hosts and the
//! vault run in Rust and talk to the UI through the commands in [`terminals`],
//! [`ssh`], [`files`], [`hosts`], [`vault`] and [`session`].

mod files;
mod hosts;
mod keys;
mod session;
mod ssh;
mod terminals;
mod vault;
mod windows;

use serde::Serialize;

#[derive(Debug, Serialize)]
struct AppInfo {
    version: &'static str,
    os: &'static str,
    /// Local login name, the default SSH user (as in OpenSSH).
    user: String,
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        version: setu_core::VERSION,
        os: std::env::consts::OS,
        user: ssh::local_user(),
    }
}

/// Starts the desktop app.
#[allow(clippy::expect_used)]
pub fn run() {
    tauri::Builder::default()
        .manage(terminals::Terminals::default())
        .manage(hosts::Hosts::from_env())
        .manage(vault::VaultState::from_env())
        .manage(files::Files::default())
        .manage(session::SessionFile::from_env())
        .manage(session::SettingsFile::from_env())
        .manage(windows::Detached::default())
        .setup(|app| {
            use tauri::{Emitter, Manager};
            let vault = app.state::<vault::VaultState>().inner().clone();
            vault.spawn_auto_lock(app.handle().clone());
            // Unlock from the OS keychain off the main thread (the keychain
            // may be slow or show its own prompt), then tell the UI.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                vault.startup_unlock();
                let _ = handle.emit(vault::STATUS_EVENT, vault.status());
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
            keys::known_hosts_list,
            keys::known_hosts_remove,
            hosts::hosts_list,
            hosts::host_save,
            hosts::host_delete,
            hosts::ssh_config_scan,
            hosts::ssh_config_import,
            hosts::hosts_parse_file,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Setu desktop app");
}
