//! Terminal tabs: Tauri commands that run local shells through `setu-pty`.
//!
//! Each open tab owns one [`PtySession`]. Output is pushed to the UI over a
//! Tauri channel as raw bytes (an `ArrayBuffer` in JavaScript), so multi-byte
//! characters split across reads are reassembled by xterm.js, not mangled here.

use serde::Serialize;
use setu_pty::{detect_shells, PtySession, PtySize, ShellProfile, SpawnOptions};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::State;

type Sessions = Arc<Mutex<HashMap<u32, Arc<PtySession>>>>;

/// All open terminal sessions, keyed by tab id.
#[derive(Debug, Default)]
pub struct Terminals {
    next_id: AtomicU32,
    sessions: Sessions,
}

impl Terminals {
    fn get(&self, id: u32) -> Result<Arc<PtySession>, String> {
        self.sessions
            .lock()
            .map_err(|_| "terminal state is unavailable".to_string())?
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("terminal {id} is not open"))
    }
}

#[derive(Debug, Serialize)]
pub struct OpenedTerminal {
    id: u32,
    shell: ShellProfile,
}

#[tauri::command]
pub fn list_shells() -> Vec<ShellProfile> {
    detect_shells()
}

/// Picks the requested shell, or the default (first detected) one.
fn choose_shell(shells: Vec<ShellProfile>, id: Option<&str>) -> Result<ShellProfile, String> {
    match id {
        Some(id) => shells
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| format!("shell `{id}` was not found on this machine")),
        None => shells
            .into_iter()
            .next()
            .ok_or_else(|| "no shell was found on this machine".to_string()),
    }
}

#[tauri::command]
pub fn open_terminal(
    state: State<'_, Terminals>,
    shell_id: Option<String>,
    cols: u16,
    rows: u16,
    on_output: Channel<InvokeResponseBody>,
    on_exit: Channel<u32>,
) -> Result<OpenedTerminal, String> {
    let shell = choose_shell(detect_shells(), shell_id.as_deref())?;
    let opts = SpawnOptions::from(&shell).size(PtySize { rows, cols });

    let session = PtySession::spawn(opts, move |chunk| {
        // A send only fails when the window is gone; the session is closed then anyway.
        let _ = on_output.send(InvokeResponseBody::Raw(chunk.to_vec()));
    })
    .map_err(|e| e.to_string())?;
    let session = Arc::new(session);

    let id = state.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    state
        .sessions
        .lock()
        .map_err(|_| "terminal state is unavailable".to_string())?
        .insert(id, Arc::clone(&session));

    // Watch for the shell exiting so the tab can show it and the session is freed.
    let sessions = Arc::clone(&state.sessions);
    std::thread::Builder::new()
        .name(format!("setu-term-{id}-exit"))
        .spawn(move || loop {
            match session.wait_timeout(Duration::from_millis(250)) {
                Ok(Some(status)) => {
                    let _ = on_exit.send(status.code);
                    if let Ok(mut map) = sessions.lock() {
                        map.remove(&id);
                    }
                    break;
                }
                Ok(None) => continue,
                Err(_) => break,
            }
        })
        .map_err(|e| e.to_string())?;

    Ok(OpenedTerminal { id, shell })
}

#[tauri::command]
pub fn write_terminal(state: State<'_, Terminals>, id: u32, data: String) -> Result<(), String> {
    state
        .get(id)?
        .write(data.as_bytes())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn resize_terminal(
    state: State<'_, Terminals>,
    id: u32,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    state
        .get(id)?
        .resize(PtySize { rows, cols })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn close_terminal(state: State<'_, Terminals>, id: u32) -> Result<(), String> {
    let session = state
        .sessions
        .lock()
        .map_err(|_| "terminal state is unavailable".to_string())?
        .remove(&id);
    if let Some(session) = session {
        session.kill().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use setu_pty::ShellKind;

    fn shell(id: &str) -> ShellProfile {
        ShellProfile {
            id: id.into(),
            name: id.into(),
            kind: ShellKind::Other,
            program: id.into(),
            args: vec![],
        }
    }

    #[test]
    fn default_shell_is_the_first_detected() {
        let chosen = choose_shell(vec![shell("zsh"), shell("bash")], None);
        assert_eq!(chosen.map(|s| s.id), Ok("zsh".to_string()));
    }

    #[test]
    fn a_named_shell_is_found() {
        let chosen = choose_shell(vec![shell("zsh"), shell("bash")], Some("bash"));
        assert_eq!(chosen.map(|s| s.id), Ok("bash".to_string()));
    }

    #[test]
    fn unknown_or_missing_shells_are_errors() {
        assert!(choose_shell(vec![shell("zsh")], Some("fish")).is_err());
        assert!(choose_shell(vec![], None).is_err());
    }
}
