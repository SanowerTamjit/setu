//! Edit a remote file in a local editor: Setu copies it to a private temp
//! folder, opens it with the chosen editor (or the system's default app),
//! and writes it back to the server each time the editor saves.
//!
//! Before each write-back Setu checks that the server's copy has not changed
//! since it was opened or last saved; if it has, the save waits for the
//! person to choose. Writes replace the file's contents in place, so it
//! keeps its owner, group and permissions.

use crate::files::Files;
use serde::Serialize;
use setu_ssh::sftp;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Emitter, State};

/// Files larger than this are not opened for editing.
const MAX_BYTES: u64 = 50 * 1024 * 1024;
const POLL: Duration = Duration::from_millis(700);
pub const EVENT: &str = "remote-edit";

struct Edit {
    id: u32,
    files_id: u32,
    remote: String,
    local: PathBuf,
    dir: PathBuf,
    stop: AtomicBool,
    /// Write even though the server's copy changed (after the person chose to).
    force: AtomicBool,
    /// Size and modification time of the server's copy as last seen.
    seen: Mutex<(u64, Option<u64>)>,
}

#[derive(Default)]
pub struct RemoteEdits {
    next_id: AtomicU32,
    edits: Mutex<HashMap<u32, Arc<Edit>>>,
}

impl std::fmt::Debug for RemoteEdits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteEdits").finish_non_exhaustive()
    }
}

impl RemoteEdits {
    fn get(&self, id: u32) -> Result<Arc<Edit>, String> {
        self.edits
            .lock()
            .map_err(|_| "edits lock poisoned".to_string())?
            .get(&id)
            .cloned()
            .ok_or_else(|| "that file is no longer being edited".into())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "state"
)]
pub enum EditEvent {
    /// The editor saved and the server has the new contents.
    Saved {
        edit_id: u32,
        at: u64,
        bytes: u64,
    },
    /// The server's copy changed since Setu last read or wrote it.
    Conflict {
        edit_id: u32,
    },
    Error {
        edit_id: u32,
        message: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Opened {
    edit_id: u32,
    local: String,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Deletes local copies left by an earlier run (a crash or a forced quit)
/// once they are a week old, so a copy with unsaved work survives a while.
fn sweep_old(root: &Path) {
    let week = Duration::from_secs(7 * 24 * 3600);
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > week);
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// A new private folder for one edit: `<temp>/setu-edit/<unique>`, mode 0700.
fn private_dir() -> Result<PathBuf, String> {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir().join("setu-edit");
    sweep_old(&root);
    let dir = root.join(format!("{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not make a temp folder: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for d in [dir.parent().unwrap_or(&dir), dir.as_path()] {
            std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(dir)
}

/// Splits an editor command like `code --wait` or `"C:\Program Files\x.exe" -n`
/// into words, honouring double quotes. `{file}` marks where the path goes;
/// without it the path is added at the end.
pub fn editor_argv(command: &str, file: &Path) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in command.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    words.push(std::mem::take(&mut word));
                    any = false;
                }
            }
            c => {
                word.push(c);
                any = true;
            }
        }
    }
    if any {
        words.push(word);
    }
    let path = file.display().to_string();
    if words.iter().any(|w| w.contains("{file}")) {
        words.iter().map(|w| w.replace("{file}", &path)).collect()
    } else {
        words.push(path);
        words
    }
}

/// Opens `file` with `editor`, or the system's default app when it is empty.
fn launch(editor: &str, file: &Path) -> Result<(), String> {
    let argv: Vec<String> = if editor.trim().is_empty() {
        let path = file.display().to_string();
        if cfg!(target_os = "macos") {
            vec!["open".into(), path]
        } else if cfg!(windows) {
            vec!["explorer".into(), path]
        } else {
            vec!["xdg-open".into(), path]
        }
    } else {
        editor_argv(editor, file)
    };
    let (program, args) = argv.split_first().ok_or("the editor command is empty")?;
    std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| {
            if editor.trim().is_empty() {
                format!("could not open the file with the system's default app ({program}: {e}). Set an editor in Settings → Files.")
            } else {
                format!("could not start the editor `{program}`: {e}")
            }
        })
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Writes the local copy back if the server's copy is still the one Setu last saw.
async fn push(files: &Files, edit: &Edit) -> Result<EditEvent, String> {
    let edit_id = edit.id;
    let session = files.get(edit.files_id)?;
    let now = session
        .sftp
        .stat(&edit.remote)
        .await
        .map_err(|e| e.to_string())?;
    let seen = *edit.seen.lock().map_err(|_| "lock poisoned")?;
    if (now.size, now.modified) != seen && !edit.force.swap(false, Ordering::Relaxed) {
        return Ok(EditEvent::Conflict { edit_id });
    }
    let data = tokio::fs::read(&edit.local)
        .await
        .map_err(|e| format!("could not read {}: {e}", edit.local.display()))?;
    session
        .sftp
        .write_file(&edit.remote, &data)
        .await
        .map_err(|e| e.to_string())?;
    let after = session
        .sftp
        .stat(&edit.remote)
        .await
        .map_err(|e| e.to_string())?;
    *edit.seen.lock().map_err(|_| "lock poisoned")? = (after.size, after.modified);
    Ok(EditEvent::Saved {
        edit_id,
        at: now_secs(),
        bytes: data.len() as u64,
    })
}

/// Copies a remote file to a private temp folder, opens it in `editor`, and
/// writes it back whenever it is saved. Events go out as [`EVENT`].
#[tauri::command]
pub async fn remote_edit_open(
    app: AppHandle,
    files: State<'_, Files>,
    edits: State<'_, RemoteEdits>,
    id: u32,
    remote: String,
    editor: String,
) -> Result<Opened, String> {
    let session = files.get(id)?;
    let name = sftp::name_remote(&remote).to_string();
    sftp::check_name(&name).map_err(|e| e.to_string())?;
    let stat = session
        .sftp
        .stat(&remote)
        .await
        .map_err(|e| e.to_string())?;
    if stat.kind == sftp::EntryKind::Dir {
        return Err("folders cannot be opened in an editor".into());
    }
    let data = session
        .sftp
        .read_file(&remote, MAX_BYTES)
        .await
        .map_err(|e| e.to_string())?;
    let edit_id = edits.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    let dir = private_dir()?;
    let local = dir.join(&name);
    std::fs::write(&local, &data)
        .map_err(|e| format!("could not write {}: {e}", local.display()))?;
    let edit = Arc::new(Edit {
        id: edit_id,
        files_id: id,
        remote,
        local: local.clone(),
        dir,
        stop: AtomicBool::new(false),
        force: AtomicBool::new(false),
        seen: Mutex::new((stat.size, stat.modified)),
    });
    if let Err(e) = launch(&editor, &local) {
        let _ = std::fs::remove_dir_all(&edit.dir);
        return Err(e);
    }
    edits
        .edits
        .lock()
        .map_err(|_| "edits lock poisoned")?
        .insert(edit_id, Arc::clone(&edit));

    // Watch the local copy and write it back after each save.
    tauri::async_runtime::spawn(async move {
        use tauri::Manager;
        let mut last = modified(&edit.local);
        loop {
            tokio::time::sleep(POLL).await;
            if edit.stop.load(Ordering::Relaxed) {
                break;
            }
            let now = modified(&edit.local);
            if now.is_none() || now == last {
                continue;
            }
            // Editors often write in several steps; wait for the file to settle.
            tokio::time::sleep(Duration::from_millis(250)).await;
            last = modified(&edit.local);
            let files = app.state::<Files>();
            let event = push(&files, &edit)
                .await
                .unwrap_or_else(|message| EditEvent::Error { edit_id, message });
            let _ = app.emit(EVENT, event);
        }
    });
    Ok(Opened {
        edit_id,
        local: local.display().to_string(),
    })
}

/// Writes the local copy now, even if the server's copy changed meanwhile.
#[tauri::command]
pub async fn remote_edit_push(
    files: State<'_, Files>,
    edits: State<'_, RemoteEdits>,
    edit_id: u32,
) -> Result<EditEvent, String> {
    let edit = edits.get(edit_id)?;
    edit.force.store(true, Ordering::Relaxed);
    push(&files, &edit).await
}

/// Stops watching and deletes the local copy.
#[tauri::command]
pub fn remote_edit_stop(edits: State<'_, RemoteEdits>, edit_id: u32) -> Result<(), String> {
    let edit = edits
        .edits
        .lock()
        .map_err(|_| "edits lock poisoned")?
        .remove(&edit_id);
    if let Some(edit) = edit {
        edit.stop.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_dir_all(&edit.dir);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_commands() {
        let f = Path::new("/tmp/x/app.conf");
        assert_eq!(
            editor_argv("code --wait", f),
            ["code", "--wait", "/tmp/x/app.conf"]
        );
        assert_eq!(
            editor_argv(
                r#""C:\Program Files\Notepad++\notepad++.exe" -multiInst"#,
                f
            ),
            [
                r"C:\Program Files\Notepad++\notepad++.exe",
                "-multiInst",
                "/tmp/x/app.conf"
            ]
        );
        assert_eq!(
            editor_argv("xterm -e vim {file}", f),
            ["xterm", "-e", "vim", "/tmp/x/app.conf"]
        );
        assert_eq!(editor_argv("  subl  ", f), ["subl", "/tmp/x/app.conf"]);
    }

    #[test]
    fn events_use_the_names_the_app_reads() {
        let json = serde_json::to_string(&EditEvent::Saved {
            edit_id: 3,
            at: 9,
            bytes: 12,
        })
        .unwrap_or_default();
        assert_eq!(json, r#"{"state":"saved","editId":3,"at":9,"bytes":12}"#);
    }
}
