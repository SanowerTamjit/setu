//! Terminal tabs: Tauri commands for local shells (`setu-pty`) and SSH
//! sessions (`setu-ssh`).
//!
//! Each open tab owns one [`Session`]. Output is pushed to the UI over a Tauri
//! channel as raw bytes (an `ArrayBuffer` in JavaScript), so multi-byte
//! characters split across reads are reassembled by xterm.js, not mangled here.

use serde::Serialize;
use setu_pty::{detect_shells, PtySession, PtySize, ShellProfile, SpawnOptions};
use setu_ssh::{ShellSize, SshClient, SshShell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::State;

/// What runs behind a tab.
pub enum Session {
    Local(Arc<PtySession>),
    Ssh {
        shell: SshShell,
        /// Keeps the connection (and any jump hosts) open while the tab lives.
        client: Arc<SshClient>,
    },
}

impl Session {
    fn write(&self, data: &[u8]) -> Result<(), String> {
        match self {
            Session::Local(pty) => pty.write(data).map_err(|e| e.to_string()),
            Session::Ssh { shell, .. } => shell.write(data).map_err(|e| e.to_string()),
        }
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        match self {
            Session::Local(pty) => pty
                .resize(PtySize { rows, cols })
                .map_err(|e| e.to_string()),
            Session::Ssh { shell, .. } => shell
                .resize(ShellSize { cols, rows })
                .map_err(|e| e.to_string()),
        }
    }

    fn close(&self) -> Result<(), String> {
        match self {
            Session::Local(pty) => pty.kill().map_err(|e| e.to_string()),
            Session::Ssh { shell, .. } => {
                shell.close();
                Ok(())
            }
        }
    }
}

pub(crate) type Sessions = Arc<Mutex<HashMap<u32, Arc<Session>>>>;
type PipeMap = Arc<Mutex<HashMap<u32, Arc<Pipes>>>>;

/// Output held while a tab moves between windows is capped at this size.
const HOLD_LIMIT: usize = 4 * 1024 * 1024;

enum Out {
    Live(Channel<InvokeResponseBody>),
    /// The tab is moving to another window: keep output until it attaches.
    Held(Vec<u8>),
}

struct PipeState {
    out: Out,
    exit: Channel<u32>,
    /// An exit that happened while held, delivered on attach.
    pending_exit: Option<u32>,
    /// The window showing this session.
    window: String,
}

/// Where a session's output and exit code go: a window's channels, or a
/// buffer while its tab is being moved to another window.
pub(crate) struct Pipes(Mutex<PipeState>);

impl Pipes {
    fn send(&self, chunk: &[u8]) {
        let Ok(mut st) = self.0.lock() else { return };
        match &mut st.out {
            // A send only fails when the window is gone; it closes its sessions then.
            Out::Live(c) => {
                let _ = c.send(InvokeResponseBody::Raw(chunk.to_vec()));
            }
            Out::Held(buf) => {
                if buf.len() + chunk.len() <= HOLD_LIMIT {
                    buf.extend_from_slice(chunk);
                }
            }
        }
    }

    fn exit(&self, code: u32) {
        let Ok(mut st) = self.0.lock() else { return };
        match st.out {
            Out::Live(_) => {
                let _ = st.exit.send(code);
            }
            Out::Held(_) => st.pending_exit = Some(code),
        }
    }

    fn hold(&self) {
        if let Ok(mut st) = self.0.lock() {
            if let Out::Live(_) = st.out {
                st.out = Out::Held(Vec::new());
            }
        }
    }

    fn attach(&self, output: Channel<InvokeResponseBody>, exit: Channel<u32>, window: &str) {
        let Ok(mut st) = self.0.lock() else { return };
        if let Out::Held(buf) = &mut st.out {
            if !buf.is_empty() {
                let _ = output.send(InvokeResponseBody::Raw(std::mem::take(buf)));
            }
        }
        st.out = Out::Live(output);
        st.exit = exit;
        st.window = window.to_string();
        if let Some(code) = st.pending_exit.take() {
            let _ = st.exit.send(code);
        }
    }

    fn shown_in(&self, window: &str) -> bool {
        self.0
            .lock()
            .map(|st| matches!(st.out, Out::Live(_)) && st.window == window)
            .unwrap_or(false)
    }
}

/// All open terminal sessions, keyed by tab id.
#[derive(Default)]
pub struct Terminals {
    next_id: AtomicU32,
    pub(crate) sessions: Sessions,
    pipes: PipeMap,
}

impl std::fmt::Debug for Terminals {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Terminals").finish_non_exhaustive()
    }
}

impl Terminals {
    /// The SSH connection behind a tab, for opening SFTP on it.
    pub(crate) fn ssh_client(&self, id: u32) -> Option<Arc<SshClient>> {
        let sessions = self.sessions.lock().ok()?;
        match sessions.get(&id)?.as_ref() {
            Session::Ssh { client, .. } => Some(Arc::clone(client)),
            Session::Local(_) => None,
        }
    }

    /// Creates the pipes for session `id`, shown in `window`.
    pub(crate) fn pipes(
        &self,
        id: u32,
        output: Channel<InvokeResponseBody>,
        exit: Channel<u32>,
        window: &str,
    ) -> Arc<Pipes> {
        let pipes = Arc::new(Pipes(Mutex::new(PipeState {
            out: Out::Live(output),
            exit,
            pending_exit: None,
            window: window.to_string(),
        })));
        if let Ok(mut map) = self.pipes.lock() {
            map.insert(id, Arc::clone(&pipes));
        }
        pipes
    }

    /// What a session calls when its process exits: report it and forget the session.
    pub(crate) fn exit_handler(
        &self,
        id: u32,
        pipes: Arc<Pipes>,
    ) -> impl FnOnce(u32) + Send + 'static {
        let sessions = Arc::clone(&self.sessions);
        let all_pipes = Arc::clone(&self.pipes);
        move |code| {
            pipes.exit(code);
            if let Ok(mut map) = sessions.lock() {
                map.remove(&id);
            }
            if let Ok(mut map) = all_pipes.lock() {
                map.remove(&id);
            }
        }
    }

    fn pipe(&self, id: u32) -> Result<Arc<Pipes>, String> {
        self.pipes
            .lock()
            .map_err(|_| "terminal state is unavailable".to_string())?
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("terminal {id} is not open"))
    }

    /// Closes the sessions a closed window was showing (not those moving out of it).
    pub fn close_shown_in(&self, window: &str) {
        let ids: Vec<u32> = match self.pipes.lock() {
            Ok(map) => map
                .iter()
                .filter(|(_, p)| p.shown_in(window))
                .map(|(id, _)| *id)
                .collect(),
            Err(_) => return,
        };
        for id in ids {
            let session = self.sessions.lock().ok().and_then(|mut m| m.remove(&id));
            if let Some(session) = session {
                let _ = session.close();
            }
            if let Ok(mut map) = self.pipes.lock() {
                map.remove(&id);
            }
        }
    }

    pub(crate) fn next_id(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub(crate) fn insert(&self, id: u32, session: Session) -> Result<(), String> {
        self.sessions
            .lock()
            .map_err(|_| "terminal state is unavailable".to_string())?
            .insert(id, Arc::new(session));
        Ok(())
    }

    fn get(&self, id: u32) -> Result<Arc<Session>, String> {
        self.sessions
            .lock()
            .map_err(|_| "terminal state is unavailable".to_string())?
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("terminal {id} is not open"))
    }
}

/// Sends raw output bytes to whichever window shows the session.
pub(crate) fn output_sink(pipes: Arc<Pipes>) -> impl FnMut(&[u8]) + Send {
    move |chunk: &[u8]| pipes.send(chunk)
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
    window: tauri::Window,
    state: State<'_, Terminals>,
    shell_id: Option<String>,
    cols: u16,
    rows: u16,
    on_output: Channel<InvokeResponseBody>,
    on_exit: Channel<u32>,
) -> Result<OpenedTerminal, String> {
    let shell = choose_shell(detect_shells(), shell_id.as_deref())?;
    let opts = SpawnOptions::from(&shell).size(PtySize { rows, cols });

    let id = state.next_id();
    let pipes = state.pipes(id, on_output, on_exit, window.label());
    let session = Arc::new(
        PtySession::spawn(opts, output_sink(Arc::clone(&pipes))).map_err(|e| e.to_string())?,
    );
    state.insert(id, Session::Local(Arc::clone(&session)))?;

    // Watch for the shell exiting so the tab can show it and the session is freed.
    let on_exit = state.exit_handler(id, pipes);
    std::thread::Builder::new()
        .name(format!("setu-term-{id}-exit"))
        .spawn(move || loop {
            match session.wait_timeout(Duration::from_millis(250)) {
                Ok(Some(status)) => {
                    on_exit(status.code);
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
    state.get(id)?.write(data.as_bytes())
}

#[tauri::command]
pub fn resize_terminal(
    state: State<'_, Terminals>,
    id: u32,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    state.get(id)?.resize(cols, rows)
}

#[tauri::command]
pub fn close_terminal(state: State<'_, Terminals>, id: u32) -> Result<(), String> {
    let session = state
        .sessions
        .lock()
        .map_err(|_| "terminal state is unavailable".to_string())?
        .remove(&id);
    match session {
        Some(session) => session.close(),
        None => Ok(()),
    }
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

/// Starts holding the output of these sessions, before their tab moves to
/// another window.
#[tauri::command]
pub fn terminal_hold(state: State<'_, Terminals>, ids: Vec<u32>) -> Result<(), String> {
    for id in ids {
        state.pipe(id)?.hold();
    }
    Ok(())
}

/// Shows session `id` in the calling window from now on, with what it printed meanwhile.
#[tauri::command]
pub fn terminal_attach(
    window: tauri::Window,
    state: State<'_, Terminals>,
    id: u32,
    on_output: Channel<InvokeResponseBody>,
    on_exit: Channel<u32>,
) -> Result<(), String> {
    state.pipe(id)?.attach(on_output, on_exit, window.label());
    Ok(())
}
