//! Files tabs: SFTP on an open SSH tab's connection, the local file system,
//! and copies between them with progress and cancel.

use crate::terminals::Terminals;
use serde::{Deserialize, Serialize};
use setu_ssh::sftp::{self, FileEntry, Progress, Sftp, TransferControl};
use setu_ssh::{SshClient, SshError};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tauri::ipc::Channel;
use tauri::State;

struct FileSession {
    sftp: Sftp,
    /// Keeps the connection open even if its terminal tab is closed.
    _client: Arc<SshClient>,
}

#[derive(Default)]
pub struct Files {
    next_id: AtomicU32,
    sessions: Mutex<HashMap<u32, Arc<FileSession>>>,
    /// Pause and cancel controls of running copies, keyed by the UI's transfer id.
    transfers: Mutex<HashMap<u32, Arc<TransferControl>>>,
}

impl std::fmt::Debug for Files {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Files").finish_non_exhaustive()
    }
}

impl Files {
    fn get(&self, id: u32) -> Result<Arc<FileSession>, String> {
        self.sessions
            .lock()
            .map_err(|_| "files lock poisoned".to_string())?
            .get(&id)
            .cloned()
            .ok_or_else(|| "this files tab is closed".into())
    }
}

fn err(e: SshError) -> String {
    e.to_string()
}

#[derive(Debug, Serialize)]
pub struct OpenedFiles {
    id: u32,
    home: String,
}

/// Opens SFTP on the connection of SSH tab `terminal_id`.
#[tauri::command]
pub async fn files_open(
    terminals: State<'_, Terminals>,
    files: State<'_, Files>,
    terminal_id: u32,
) -> Result<OpenedFiles, String> {
    let client = terminals
        .ssh_client(terminal_id)
        .ok_or("that tab is not an open SSH connection")?;
    let sftp = client.sftp().await.map_err(err)?;
    let home = sftp.home().await.map_err(err)?;
    let id = files.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    files
        .sessions
        .lock()
        .map_err(|_| "files lock poisoned")?
        .insert(
            id,
            Arc::new(FileSession {
                sftp,
                _client: client,
            }),
        );
    Ok(OpenedFiles { id, home })
}

#[tauri::command]
pub fn files_close(files: State<'_, Files>, id: u32) -> Result<(), String> {
    files
        .sessions
        .lock()
        .map_err(|_| "files lock poisoned")?
        .remove(&id);
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct Listing {
    /// The folder as resolved (absolute, no `..`).
    path: String,
    parent: Option<String>,
    entries: Vec<FileEntry>,
}

#[tauri::command]
pub async fn files_list(files: State<'_, Files>, id: u32, path: String) -> Result<Listing, String> {
    let session = files.get(id)?;
    let path = session.sftp.canonicalize(&path).await.map_err(err)?;
    let entries = session.sftp.list(&path).await.map_err(err)?;
    let parent = (path != "/").then(|| sftp::parent_remote(&path));
    Ok(Listing {
        path,
        parent,
        entries,
    })
}

#[tauri::command]
pub async fn files_mkdir(
    files: State<'_, Files>,
    id: u32,
    dir: String,
    name: String,
) -> Result<(), String> {
    sftp::check_name(&name).map_err(err)?;
    let session = files.get(id)?;
    session
        .sftp
        .mkdir(&sftp::join_remote(&dir, &name))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn files_rename(
    files: State<'_, Files>,
    id: u32,
    dir: String,
    from: String,
    to: String,
) -> Result<(), String> {
    sftp::check_name(&to).map_err(err)?;
    let session = files.get(id)?;
    session
        .sftp
        .rename(
            &sftp::join_remote(&dir, &from),
            &sftp::join_remote(&dir, &to),
        )
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn files_remove(
    files: State<'_, Files>,
    id: u32,
    dir: String,
    name: String,
) -> Result<(), String> {
    sftp::check_name(&name).map_err(err)?;
    let session = files.get(id)?;
    session
        .sftp
        .remove(&sftp::join_remote(&dir, &name))
        .await
        .map_err(err)
}

pub(crate) fn home_dir() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Lists a local folder; an empty path means the home folder.
#[tauri::command]
pub fn local_list(path: String) -> Result<Listing, String> {
    let dir = if path.trim().is_empty() {
        home_dir()
    } else {
        PathBuf::from(path.trim())
    };
    // Resolve `..` without Windows' `\\?\` prefix: rebuild from components.
    let dir: PathBuf = std::fs::canonicalize(&dir)
        .map(|p| strip_verbatim(&p))
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    let entries = sftp::list_local(&dir).map_err(err)?;
    Ok(Listing {
        parent: dir.parent().map(|p| p.display().to_string()),
        path: dir.display().to_string(),
        entries,
    })
}

fn strip_verbatim(p: &Path) -> PathBuf {
    let s = p.display().to_string();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => p.to_path_buf(),
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Upload,
    Download,
}

/// Copies one file or folder; progress streams to `on_progress`. Resolves
/// with the final progress, or rejects: with "the transfer was cancelled"
/// after [`transfer_cancel`], or "paused" after [`transfer_pause`]. With
/// `resume`, finished files are skipped and partial ones continued.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn transfer(
    files: State<'_, Files>,
    id: u32,
    transfer_id: u32,
    direction: Direction,
    source: String,
    target_dir: String,
    resume: bool,
    on_progress: Channel<Progress>,
) -> Result<Progress, String> {
    let session = files.get(id)?;
    let control = Arc::new(TransferControl::default());
    files
        .transfers
        .lock()
        .map_err(|_| "files lock poisoned")?
        .insert(transfer_id, Arc::clone(&control));
    let report = |p: &Progress| {
        let _ = on_progress.send(p.clone());
    };
    let result = match direction {
        Direction::Download => {
            session
                .sftp
                .download(&source, Path::new(&target_dir), resume, &control, report)
                .await
        }
        Direction::Upload => {
            session
                .sftp
                .upload(Path::new(&source), &target_dir, resume, &control, report)
                .await
        }
    };
    if let Ok(mut map) = files.transfers.lock() {
        map.remove(&transfer_id);
    }
    match result {
        Err(SshError::Cancelled) if control.paused() => Err("paused".into()),
        other => other.map_err(err),
    }
}

fn with_transfer(files: &Files, transfer_id: u32, f: impl FnOnce(&TransferControl)) {
    if let Ok(map) = files.transfers.lock() {
        if let Some(control) = map.get(&transfer_id) {
            f(control);
        }
    }
}

/// Stops a copy and deletes its partial file.
#[tauri::command]
pub fn transfer_cancel(files: State<'_, Files>, transfer_id: u32) {
    with_transfer(&files, transfer_id, TransferControl::cancel);
}

/// Stops a copy but keeps its partial file, so it can resume.
#[tauri::command]
pub fn transfer_pause(files: State<'_, Files>, transfer_id: u32) {
    with_transfer(&files, transfer_id, TransferControl::pause);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_listing_resolves_dot_dot() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("a/b")).expect("mkdir");
        let listing = local_list(dir.path().join("a/b/..").display().to_string()).expect("list");
        assert!(listing.path.ends_with('a'));
        assert_eq!(listing.entries[0].name, "b");
        assert!(listing.parent.is_some());
    }

    #[test]
    fn verbatim_prefix_is_removed() {
        assert_eq!(
            strip_verbatim(Path::new(r"\\?\C:\Users")),
            PathBuf::from(r"C:\Users")
        );
        assert_eq!(
            strip_verbatim(Path::new("/home/a")),
            PathBuf::from("/home/a")
        );
    }
}
