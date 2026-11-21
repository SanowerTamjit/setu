//! The Setu desktop app: a Tauri 2 shell around the Rust core.
//!
//! The web view only renders. Shells, and later SSH sessions and the vault,
//! run in Rust and talk to the UI through the commands in [`terminals`] and [`ssh`].

mod ssh;
mod terminals;

use serde::Serialize;

#[derive(Debug, Serialize)]
struct AppInfo {
    version: &'static str,
    os: &'static str,
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        version: setu_core::VERSION,
        os: std::env::consts::OS,
    }
}

/// Starts the desktop app.
#[allow(clippy::expect_used)]
pub fn run() {
    tauri::Builder::default()
        .manage(terminals::Terminals::default())
        .invoke_handler(tauri::generate_handler![
            app_info,
            terminals::list_shells,
            terminals::open_terminal,
            terminals::write_terminal,
            terminals::resize_terminal,
            terminals::close_terminal,
            ssh::ssh_connect,
            ssh::ssh_trust_host,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Setu desktop app");
}
