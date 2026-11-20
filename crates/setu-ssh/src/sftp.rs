//! SFTP: browse remote files and copy files or folders in either direction.
//!
//! Built on [`russh_sftp`] over a channel of an existing [`SshClient`], so it
//! reuses the connection (and its host-key check and authentication).
//!
//! Copies write straight to the destination name; if a copy fails or is
//! cancelled, the partly written file is deleted. Symlinked folders are not
//! followed when copying a folder, so a link loop cannot make a copy endless.

use crate::{SshClient, SshError};
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::FileType;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const CHUNK: usize = 256 * 1024;
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Other,
}

/// One row of a directory listing, local or remote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileEntry {
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
    /// Seconds since the Unix epoch.
    pub modified: Option<u64>,
    /// Unix mode bits, when known.
    pub permissions: Option<u32>,
}

/// Folders first, then by name ignoring case.
pub fn sort_entries(entries: &mut [FileEntry]) {
    entries.sort_by(|a, b| {
        (a.kind != EntryKind::Dir)
            .cmp(&(b.kind != EntryKind::Dir))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// Joins a remote directory and a name with `/`.
pub fn join_remote(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// The parent of a remote path (`/` stays `/`).
pub fn parent_remote(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() && path.starts_with('/') {
        return "/".into();
    }
    match trimmed.rfind('/') {
        Some(0) => "/".into(),
        Some(i) => trimmed[..i].into(),
        None => ".".into(),
    }
}

/// The last component of a remote path.
pub fn name_remote(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
}

/// Rejects names that would escape the folder they are created in.
pub fn check_name(name: &str) -> Result<(), SshError> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
        return Err(SshError::Files {
            path: name.into(),
            reason: "not a valid file name".into(),
        });
    }
    Ok(())
}

fn files_err(path: impl std::fmt::Display, e: impl std::fmt::Display) -> SshError {
    SshError::Files {
        path: path.to_string(),
        reason: e.to_string(),
    }
}

fn kind_of(t: FileType) -> EntryKind {
    match t {
        FileType::Dir => EntryKind::Dir,
        FileType::File => EntryKind::File,
        FileType::Symlink => EntryKind::Symlink,
        FileType::Other => EntryKind::Other,
    }
}

/// Lists a local folder.
pub fn list_local(dir: &Path) -> Result<Vec<FileEntry>, SshError> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| files_err(dir.display(), e))? {
        let Ok(entry) = entry else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        let kind = if meta.is_symlink() {
            EntryKind::Symlink
        } else if meta.is_dir() {
            EntryKind::Dir
        } else if meta.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        };
        #[cfg(unix)]
        let permissions = {
            use std::os::unix::fs::PermissionsExt;
            Some(meta.permissions().mode())
        };
        #[cfg(not(unix))]
        let permissions = None;
        out.push(FileEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            kind,
            size: if meta.is_file() { meta.len() } else { 0 },
            modified: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
            permissions,
        });
    }
    sort_entries(&mut out);
    Ok(out)
}

/// How far a copy has got.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub files_done: usize,
    pub files_total: usize,
    /// The file being copied, relative to what was dropped.
    pub current: String,
}

/// What a copy will do, worked out before any byte moves.
#[derive(Debug, Default)]
struct Plan<S, D> {
    /// Folders to create at the destination, parents first.
    dirs: Vec<D>,
    /// (source, destination, size, display name)
    files: Vec<(S, D, u64, String)>,
}

impl<S, D> Plan<S, D> {
    fn total(&self) -> u64 {
        self.files.iter().map(|f| f.2).sum()
    }
}

/// Reports progress at most every [`PROGRESS_EVERY`], plus at each file's end.
struct Reporter<F: FnMut(&Progress)> {
    progress: Progress,
    last: Instant,
    on_progress: F,
}

impl<F: FnMut(&Progress)> Reporter<F> {
    fn new(total_bytes: u64, files_total: usize, on_progress: F) -> Self {
        let mut r = Reporter {
            progress: Progress {
                total_bytes,
                files_total,
                ..Progress::default()
            },
            last: Instant::now(),
            on_progress,
        };
        (r.on_progress)(&r.progress);
        r
    }

    fn start(&mut self, name: &str) {
        self.progress.current = name.to_string();
        self.emit();
    }

    fn add(&mut self, bytes: u64) {
        self.progress.done_bytes += bytes;
        if self.last.elapsed() >= PROGRESS_EVERY {
            self.emit();
        }
    }

    fn file_done(&mut self) {
        self.progress.files_done += 1;
        self.emit();
    }

    fn emit(&mut self) {
        self.last = Instant::now();
        (self.on_progress)(&self.progress);
    }
}

/// An SFTP session on an SSH connection.
pub struct Sftp {
    session: SftpSession,
}

impl std::fmt::Debug for Sftp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sftp").finish_non_exhaustive()
    }
}

impl SshClient {
    /// Opens an SFTP session on this connection.
    pub async fn sftp(&self) -> Result<Sftp, SshError> {
        let stream = self.subsystem("sftp").await?;
        let session = SftpSession::new(stream)
            .await
            .map_err(|e| SshError::Protocol(format!("the server refused SFTP: {e}")))?;
        Ok(Sftp { session })
    }
}

impl Sftp {
    /// The remote home folder (where the session starts).
    pub async fn home(&self) -> Result<String, SshError> {
        self.session
            .canonicalize(".")
            .await
            .map_err(|e| files_err(".", e))
    }

    /// Resolves `.`/`..` and symlinks in a remote path.
    pub async fn canonicalize(&self, path: &str) -> Result<String, SshError> {
        self.session
            .canonicalize(path)
            .await
            .map_err(|e| files_err(path, e))
    }

    /// Lists a remote folder, without `.` and `..`.
    pub async fn list(&self, path: &str) -> Result<Vec<FileEntry>, SshError> {
        let dir = self
            .session
            .read_dir(path)
            .await
            .map_err(|e| files_err(path, e))?;
        let mut out: Vec<FileEntry> = dir
            .filter(|e| e.file_name() != "." && e.file_name() != "..")
            .map(|e| {
                let meta = e.metadata();
                FileEntry {
                    name: e.file_name(),
                    kind: kind_of(e.file_type()),
                    size: meta.size.unwrap_or(0),
                    modified: meta.mtime.map(u64::from),
                    permissions: meta.permissions,
                }
            })
            .collect();
        sort_entries(&mut out);
        Ok(out)
    }

    pub async fn mkdir(&self, path: &str) -> Result<(), SshError> {
        self.session
            .create_dir(path)
            .await
            .map_err(|e| files_err(path, e))
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<(), SshError> {
        self.session
            .rename(from, to)
            .await
            .map_err(|e| files_err(from, e))
    }

    /// Deletes a file, a symlink, or a folder with everything in it.
    pub async fn remove(&self, path: &str) -> Result<(), SshError> {
        let meta = self
            .session
            .symlink_metadata(path)
            .await
            .map_err(|e| files_err(path, e))?;
        if !meta.is_dir() {
            return self
                .session
                .remove_file(path)
                .await
                .map_err(|e| files_err(path, e));
        }
        // Walk depth-first, collecting folders to remove after their contents.
        let mut stack = vec![path.to_string()];
        let mut dirs = Vec::new();
        while let Some(dir) = stack.pop() {
            for entry in self.list(&dir).await? {
                let child = join_remote(&dir, &entry.name);
                if entry.kind == EntryKind::Dir {
                    stack.push(child);
                } else {
                    self.session
                        .remove_file(&child)
                        .await
                        .map_err(|e| files_err(&child, e))?;
                }
            }
            dirs.push(dir);
        }
        for dir in dirs.iter().rev() {
            self.session
                .remove_dir(dir)
                .await
                .map_err(|e| files_err(dir, e))?;
        }
        Ok(())
    }

    async fn plan_download(
        &self,
        remote: &str,
        local_dir: &Path,
    ) -> Result<Plan<String, PathBuf>, SshError> {
        let name = name_remote(remote).to_string();
        check_name(&name)?;
        let meta = self
            .session
            .metadata(remote)
            .await
            .map_err(|e| files_err(remote, e))?;
        let mut plan = Plan::default();
        if !meta.is_dir() {
            plan.files.push((
                remote.to_string(),
                local_dir.join(&name),
                meta.size.unwrap_or(0),
                name,
            ));
            return Ok(plan);
        }
        let mut stack = vec![(remote.to_string(), local_dir.join(&name), name)];
        while let Some((src, dst, label)) = stack.pop() {
            plan.dirs.push(dst.clone());
            for entry in self.list(&src).await? {
                check_name(&entry.name)?;
                let child_src = join_remote(&src, &entry.name);
                let child_dst = dst.join(&entry.name);
                let child_label = format!("{label}/{}", entry.name);
                match entry.kind {
                    EntryKind::Dir => stack.push((child_src, child_dst, child_label)),
                    EntryKind::File => {
                        plan.files
                            .push((child_src, child_dst, entry.size, child_label))
                    }
                    // Copy what a symlink points to if it is a file; skip linked folders.
                    EntryKind::Symlink => {
                        if let Ok(m) = self.session.metadata(&child_src).await {
                            if m.is_regular() {
                                plan.files.push((
                                    child_src,
                                    child_dst,
                                    m.size.unwrap_or(0),
                                    child_label,
                                ));
                            }
                        }
                    }
                    EntryKind::Other => {}
                }
            }
        }
        Ok(plan)
    }

    /// Copies a remote file or folder into `local_dir`.
    pub async fn download(
        &self,
        remote: &str,
        local_dir: &Path,
        cancel: &AtomicBool,
        on_progress: impl FnMut(&Progress),
    ) -> Result<Progress, SshError> {
        let plan = self.plan_download(remote, local_dir).await?;
        let mut report = Reporter::new(plan.total(), plan.files.len(), on_progress);
        for dir in &plan.dirs {
            tokio::fs::create_dir_all(dir)
                .await
                .map_err(|e| files_err(dir.display(), e))?;
        }
        let mut buf = vec![0u8; CHUNK];
        for (src, dst, _, label) in &plan.files {
            report.start(label);
            let result = async {
                let mut from = self
                    .session
                    .open(src)
                    .await
                    .map_err(|e| files_err(src, e))?;
                let mut to = tokio::fs::File::create(dst)
                    .await
                    .map_err(|e| files_err(dst.display(), e))?;
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(SshError::Cancelled);
                    }
                    let n = from.read(&mut buf).await.map_err(|e| files_err(src, e))?;
                    if n == 0 {
                        break;
                    }
                    to.write_all(&buf[..n])
                        .await
                        .map_err(|e| files_err(dst.display(), e))?;
                    report.add(n as u64);
                }
                to.flush().await.map_err(|e| files_err(dst.display(), e))?;
                Ok(())
            }
            .await;
            if let Err(e) = result {
                let _ = tokio::fs::remove_file(dst).await;
                return Err(e);
            }
            report.file_done();
        }
        Ok(report.progress)
    }

    fn plan_upload(local: &Path, remote_dir: &str) -> Result<Plan<PathBuf, String>, SshError> {
        let name = local
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| files_err(local.display(), "has no file name"))?;
        check_name(&name)?;
        let meta = std::fs::metadata(local).map_err(|e| files_err(local.display(), e))?;
        let mut plan = Plan::default();
        if !meta.is_dir() {
            plan.files.push((
                local.to_path_buf(),
                join_remote(remote_dir, &name),
                meta.len(),
                name,
            ));
            return Ok(plan);
        }
        let mut stack = vec![(local.to_path_buf(), join_remote(remote_dir, &name), name)];
        while let Some((src, dst, label)) = stack.pop() {
            plan.dirs.push(dst.clone());
            for entry in list_local(&src)? {
                check_name(&entry.name)?;
                let child_src = src.join(&entry.name);
                let child_dst = join_remote(&dst, &entry.name);
                let child_label = format!("{label}/{}", entry.name);
                match entry.kind {
                    EntryKind::Dir => stack.push((child_src, child_dst, child_label)),
                    EntryKind::File => {
                        plan.files
                            .push((child_src, child_dst, entry.size, child_label))
                    }
                    EntryKind::Symlink => {
                        if let Ok(m) = std::fs::metadata(&child_src) {
                            if m.is_file() {
                                plan.files
                                    .push((child_src, child_dst, m.len(), child_label));
                            }
                        }
                    }
                    EntryKind::Other => {}
                }
            }
        }
        Ok(plan)
    }

    /// Copies a local file or folder into `remote_dir`.
    pub async fn upload(
        &self,
        local: &Path,
        remote_dir: &str,
        cancel: &AtomicBool,
        on_progress: impl FnMut(&Progress),
    ) -> Result<Progress, SshError> {
        let plan = Self::plan_upload(local, remote_dir)?;
        let mut report = Reporter::new(plan.total(), plan.files.len(), on_progress);
        for dir in &plan.dirs {
            // An existing folder is fine: copying into it merges.
            if !self.session.try_exists(dir.as_str()).await.unwrap_or(false) {
                self.mkdir(dir).await?;
            }
        }
        let mut buf = vec![0u8; CHUNK];
        for (src, dst, _, label) in &plan.files {
            report.start(label);
            let result = async {
                let mut from = tokio::fs::File::open(src)
                    .await
                    .map_err(|e| files_err(src.display(), e))?;
                let mut to = self
                    .session
                    .create(dst)
                    .await
                    .map_err(|e| files_err(dst, e))?;
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(SshError::Cancelled);
                    }
                    let n = from
                        .read(&mut buf)
                        .await
                        .map_err(|e| files_err(src.display(), e))?;
                    if n == 0 {
                        break;
                    }
                    to.write_all(&buf[..n])
                        .await
                        .map_err(|e| files_err(dst, e))?;
                    report.add(n as u64);
                }
                to.shutdown().await.map_err(|e| files_err(dst, e))?;
                Ok(())
            }
            .await;
            if let Err(e) = result {
                let _ = self.session.remove_file(dst.as_str()).await;
                return Err(e);
            }
            report.file_done();
        }
        Ok(report.progress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_paths() {
        assert_eq!(join_remote("/home/a", "x"), "/home/a/x");
        assert_eq!(join_remote("/", "x"), "/x");
        assert_eq!(parent_remote("/home/a/"), "/home");
        assert_eq!(parent_remote("/home"), "/");
        assert_eq!(parent_remote("/"), "/");
        assert_eq!(name_remote("/var/log/syslog"), "syslog");
        assert_eq!(name_remote("/var/log/"), "log");
    }

    #[test]
    fn names_cannot_escape() {
        for bad in ["", ".", "..", "a/b", "a\\b", "nul\0"] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
        assert!(check_name(".bashrc").is_ok());
    }

    #[test]
    fn sorting_puts_folders_first() {
        let e = |name: &str, kind| FileEntry {
            name: name.into(),
            kind,
            size: 0,
            modified: None,
            permissions: None,
        };
        let mut list = vec![
            e("b.txt", EntryKind::File),
            e("Zeta", EntryKind::Dir),
            e("A.txt", EntryKind::File),
            e("alpha", EntryKind::Dir),
        ];
        sort_entries(&mut list);
        let names: Vec<_> = list.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["alpha", "Zeta", "A.txt", "b.txt"]);
    }

    #[test]
    fn local_listing_and_upload_plan() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("site");
        std::fs::create_dir_all(root.join("css")).expect("mkdir");
        std::fs::write(root.join("index.html"), b"<h1>hi</h1>").expect("write");
        std::fs::write(root.join("css/app.css"), b"body{}").expect("write");
        let listing = list_local(&root).expect("list");
        assert_eq!(listing[0].name, "css");
        assert_eq!(listing[1].size, 11);

        let plan = Sftp::plan_upload(&root, "/srv").expect("plan");
        assert_eq!(plan.dirs, ["/srv/site", "/srv/site/css"]);
        let mut files: Vec<_> = plan.files.iter().map(|f| f.1.as_str()).collect();
        files.sort();
        assert_eq!(files, ["/srv/site/css/app.css", "/srv/site/index.html"]);
        assert_eq!(plan.total(), 17);
    }
}
