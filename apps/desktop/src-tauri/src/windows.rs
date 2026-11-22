//! Detached windows: a tab dragged (or moved with Ctrl+Shift+N) out of a
//! window opens in a new window of its own, keeping its running shells.
//!
//! The UI holds the tab's sessions (see `terminal_hold`), hands its layout and
//! scrollback here as an opaque JSON payload, and the new window takes the
//! payload once at start-up and attaches to the sessions.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, State, WebviewUrl, WebviewWindowBuilder};

/// Payloads waiting for their new window, by token.
#[derive(Debug, Default)]
pub struct Detached {
    next: AtomicU32,
    waiting: Mutex<HashMap<String, String>>,
}

/// Larger payloads are refused (scrollback is capped well below this).
const MAX_PAYLOAD: usize = 16 * 1024 * 1024;

#[tauri::command]
pub fn window_detach(
    app: AppHandle,
    state: State<'_, Detached>,
    payload: String,
    title: String,
) -> Result<(), String> {
    if payload.len() > MAX_PAYLOAD {
        return Err("that tab is too large to move".into());
    }
    let n = state.next.fetch_add(1, Ordering::Relaxed) + 1;
    let token = format!("w{n}");
    state
        .waiting
        .lock()
        .map_err(|_| "window state is unavailable")?
        .insert(token.clone(), payload);
    let url = WebviewUrl::App(format!("index.html?detached={token}").into());
    let built = WebviewWindowBuilder::new(&app, format!("detached-{n}"), url)
        .title(format!("{title} · Setu"))
        .inner_size(960.0, 620.0)
        .min_inner_size(480.0, 320.0)
        .build();
    if let Err(e) = built {
        if let Ok(mut map) = state.waiting.lock() {
            map.remove(&token);
        }
        return Err(e.to_string());
    }
    Ok(())
}

/// The payload for a new window; each token works once.
#[tauri::command]
pub fn window_take(state: State<'_, Detached>, token: String) -> Option<String> {
    state.waiting.lock().ok()?.remove(&token)
}
