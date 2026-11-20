//! The Setu desktop app: a Tauri 2 shell around the Rust core.
//!
//! The web view only renders. Shells, SSH sessions, saved hosts and the
//! vault run in Rust and talk to the UI through the commands in [`terminals`],
//! [`ssh`], [`hosts`] and [`vault`].

mod hosts;
mod ssh;
mod terminals;
mod vault;

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
        .setup(|app| {
            use tauri::Manager;
            let vault = app.state::<vault::VaultState>().inner().clone();
            vault.spawn_auto_lock(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            terminals::list_shells,
            terminals::open_terminal,
            terminals::write_terminal,
            terminals::resize_terminal,
            terminals::close_terminal,
            ssh::ssh_connect,
            ssh::ssh_trust_host,
            hosts::hosts_list,
            hosts::host_save,
            hosts::host_delete,
            hosts::ssh_config_scan,
            hosts::ssh_config_import,
            vault::vault_status,
            vault::vault_create,
            vault::vault_unlock,
            vault::vault_lock,
            vault::vault_remove,
            vault::vault_change_password,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Setu desktop app");
}
