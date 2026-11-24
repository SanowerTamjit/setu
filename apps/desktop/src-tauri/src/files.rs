//! Files tabs: SFTP on an open SSH tab's connection, the local file system,
//! and copies between them with progress and cancel.

use crate::terminals::Terminals;
use serde::{Deserialize, Serialize};
use setu_core::extend::{AuditAction, AuditEvent, AuditOutcome, Extensions};
use setu_ssh::sftp::{self, Change, FileEntry, Progress, Sftp, Stat, TransferControl};
use setu_ssh::{SshClient, SshError};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tauri::ipc::Channel;
use tauri::State;

pub(crate) struct FileSession {
    pub(crate) sftp: Sftp,
    /// Keeps the connection open even if its terminal tab is closed.
    pub(crate) client: Arc<SshClient>,
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
    pub(crate) fn get(&self, id: u32) -> Result<Arc<FileSession>, String> {
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
        .insert(id, Arc::new(FileSession { sftp, client }));
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
    extensions: State<'_, Extensions>,
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
    extensions.audit(transfer_event(
        session.client.host(),
        direction,
        &source,
        &target_dir,
        &result,
        control.paused(),
    ));
    match result {
        Err(SshError::Cancelled) if control.paused() => Err("paused".into()),
        other => other.map_err(err),
    }
}

/// The audit record of a copy: which file went where, and how it ended.
fn transfer_event<T>(
    host: &str,
    direction: Direction,
    source: &str,
    target_dir: &str,
    result: &Result<T, SshError>,
    paused: bool,
) -> AuditEvent {
    let outcome = match result {
        Ok(_) => AuditOutcome::Succeeded,
        Err(SshError::Cancelled) => AuditOutcome::Cancelled,
        Err(_) => AuditOutcome::Failed,
    };
    let verb = match direction {
        Direction::Download => "download",
        Direction::Upload => "upload",
    };
    let paused = if paused { " (paused)" } else { "" };
    AuditEvent {
        action: AuditAction::FileTransfer,
        outcome,
        target: host.to_string(),
        detail: format!("{verb} {source} to {target_dir}{paused}"),
    }
}

/// A user or group name that is safe to pass to `getent` on the server.
fn valid_account_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '$'))
}

/// Looks up `getent <db> <key>` on the server: (name, id) when found.
async fn getent(client: &SshClient, db: &str, key: &str) -> Option<(String, u32)> {
    let (out, code) = client.exec(&format!("getent {db} '{key}'")).await.ok()?;
    if code != Some(0) {
        return None;
    }
    let line = String::from_utf8_lossy(&out);
    let mut fields = line.trim().split(':');
    let name = fields.next()?.to_string();
    let id = fields.nth(1)?.parse().ok()?;
    Some((name, id))
}

/// A user or group typed in the permissions editor: a number, or a name the
/// server knows.
async fn account_id(client: &SshClient, db: &str, typed: &str) -> Result<u32, String> {
    let typed = typed.trim();
    if let Ok(id) = typed.parse::<u32>() {
        return Ok(id);
    }
    let what = if db == "passwd" { "user" } else { "group" };
    if !valid_account_name(typed) {
        return Err(format!("`{typed}` is not a valid {what} name"));
    }
    getent(client, db, typed)
        .await
        .map(|(_, id)| id)
        .ok_or_else(|| format!("the server has no {what} called `{typed}`"))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatView {
    #[serde(flatten)]
    stat: Stat,
    /// Names for the ids, when the server can tell (`getent`).
    user: Option<String>,
    group: Option<String>,
}

/// Mode, owner and group of one remote item, with names where known.
#[tauri::command]
pub async fn files_stat(
    files: State<'_, Files>,
    id: u32,
    path: String,
) -> Result<StatView, String> {
    let session = files.get(id)?;
    let stat = session.sftp.stat(&path).await.map_err(err)?;
    let name = |db: &'static str, id: Option<u32>| {
        let client = Arc::clone(&session.client);
        async move {
            match id {
                Some(id) => getent(&client, db, &id.to_string()).await.map(|(n, _)| n),
                None => None,
            }
        }
    };
    let (user, group) = tokio::join!(name("passwd", stat.uid), name("group", stat.gid));
    Ok(StatView { stat, user, group })
}

/// Changes mode, user and/or group of `names` in `dir`; `None` leaves that
/// part alone. With `recursive`, folders' contents change too. Returns how
/// many items changed.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn files_set_attributes(
    files: State<'_, Files>,
    id: u32,
    dir: String,
    names: Vec<String>,
    mode: Option<u32>,
    user: Option<String>,
    group: Option<String>,
    recursive: bool,
) -> Result<usize, String> {
    let session = files.get(id)?;
    if mode.is_some_and(|m| m > 0o7777) {
        return Err("that is not a valid mode".into());
    }
    let uid = match user.as_deref().filter(|u| !u.trim().is_empty()) {
        Some(u) => Some(account_id(&session.client, "passwd", u).await?),
        None => None,
    };
    let gid = match group.as_deref().filter(|g| !g.trim().is_empty()) {
        Some(g) => Some(account_id(&session.client, "group", g).await?),
        None => None,
    };
    let change = Change { mode, uid, gid };
    let mut items = 0;
    for name in &names {
        sftp::check_name(name).map_err(err)?;
        let path = sftp::join_remote(&dir, name);
        items += session
            .sftp
            .set_attributes(&path, change, recursive)
            .await
            .map_err(err)?
            .items;
    }
    Ok(items)
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
    fn transfer_audit_names_the_file_and_outcome() {
        let ok: Result<(), SshError> = Ok(());
        let e = transfer_event("web1", Direction::Upload, "/tmp/a.tar", "/srv", &ok, false);
        assert_eq!(e.action, AuditAction::FileTransfer);
        assert_eq!(e.outcome, AuditOutcome::Succeeded);
        assert_eq!(e.target, "web1");
        assert_eq!(e.detail, "upload /tmp/a.tar to /srv");
        let stopped: Result<(), SshError> = Err(SshError::Cancelled);
        let e = transfer_event("web1", Direction::Download, "/a", "/b", &stopped, true);
        assert_eq!(e.outcome, AuditOutcome::Cancelled);
        assert_eq!(e.detail, "download /a to /b (paused)");
    }

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
    fn account_names_are_checked() {
        for ok in ["www-data", "deploy", "svc_app", "web.ops", "machine$"] {
            assert!(valid_account_name(ok), "{ok}");
        }
        for bad in ["", "-x", "a b", "x'; rm -rf /", "a/b", "$(id)"] {
            assert!(!valid_account_name(bad), "{bad}");
        }
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
