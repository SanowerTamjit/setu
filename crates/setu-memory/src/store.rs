//! The work memory file: what is kept, the person's choices, and saved
//! snippets. One JSON file, `work-memory.json` in Setu's config folder,
//! written atomically and readable only by the owner.

use crate::redact::redact;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const FORMAT_VERSION: u32 = 1;
/// Commands kept at most; the oldest go first.
pub const MAX_ENTRIES: usize = 5000;
/// Longer lines are not kept (a pasted script, most likely).
pub const MAX_COMMAND: usize = 2000;
pub const MAX_IGNORE: usize = 50;
pub const MAX_SNIPPETS: usize = 200;
pub const MAX_DISMISSED: usize = 500;
pub const DEFAULT_RETENTION_DAYS: u32 = 30;
/// A bigger file was not written by Setu (5,000 capped lines fit easily).
pub const MAX_FILE: u64 = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("{0}")]
    Invalid(String),
    #[error("{path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("{path} was written by a newer Setu (format {found}); update Setu to open it")]
    NewerFormat { path: PathBuf, found: u32 },
    #[error("could not write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
}

fn invalid(m: impl Into<String>) -> MemoryError {
    MemoryError::Invalid(m.into())
}

/// One command that was run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    /// Seconds since 1970.
    pub at: u64,
    /// The command line, with secrets masked.
    pub command: String,
    /// The saved host it ran on; `None` for a local shell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Which terminal pane, so commands typed one after another in the
    /// same place can be read as a sequence.
    pub session: String,
    /// Whether a secret was masked. Masked commands are never suggested.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub masked: bool,
}

/// A command saved as a snippet: it shows in the command palette and
/// types the command without pressing Enter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    pub id: String,
    pub title: String,
    pub command: String,
    pub created: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// Off until the person turns it on.
    #[serde(default)]
    pub recording: bool,
    #[serde(default = "default_retention")]
    pub retention_days: u32,
    /// Commands starting with any of these are never kept.
    #[serde(default)]
    pub ignore: Vec<String>,
}

fn default_retention() -> u32 {
    DEFAULT_RETENTION_DAYS
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            recording: false,
            retention_days: DEFAULT_RETENTION_DAYS,
            ignore: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryFile {
    #[serde(default = "format_version")]
    pub version: u32,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub entries: Vec<Entry>,
    #[serde(default)]
    pub next_id: u64,
    /// Suggestions the person said no to.
    #[serde(default)]
    pub dismissed: Vec<String>,
    #[serde(default)]
    pub snippets: Vec<Snippet>,
}

fn format_version() -> u32 {
    FORMAT_VERSION
}

impl Default for MemoryFile {
    fn default() -> Self {
        MemoryFile {
            version: FORMAT_VERSION,
            settings: Settings::default(),
            entries: Vec::new(),
            next_id: 1,
            dismissed: Vec::new(),
            snippets: Vec::new(),
        }
    }
}

/// Why a command was or was not kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Recorded {
    Kept,
    /// Kept, with a secret masked.
    KeptMasked,
    /// Remembering is off.
    Off,
    /// Empty, too long, a leading space (like shell history), on the
    /// ignore list, the same as the line before, or holding a private key.
    Skipped,
}

fn clean_text(s: &str) -> Option<String> {
    if s.chars().any(|c| c.is_control() && c != '\t') {
        return None;
    }
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    (!collapsed.is_empty()).then_some(collapsed)
}

/// Whether `command` is `prefix` or starts with it as whole words, so
/// ignoring `vim` skips `vim notes.txt` but not `vimdiff`.
fn ignored(command: &str, prefix: &str) -> bool {
    !prefix.is_empty()
        && command.starts_with(prefix)
        && command[prefix.len()..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace)
}

fn valid_label(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}

impl MemoryFile {
    /// Keeps a command typed in a terminal. `host` is the saved host's
    /// name for SSH panes; `session` identifies the pane.
    pub fn record(
        &mut self,
        raw: &str,
        host: Option<&str>,
        session: &str,
        now: u64,
    ) -> Result<Recorded, MemoryError> {
        if !valid_label(session, 64)
            || !session
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(invalid("bad session id"));
        }
        if let Some(h) = host {
            if !valid_label(h, 200) {
                return Err(invalid("bad host name"));
            }
        }
        if !self.settings.recording {
            return Ok(Recorded::Off);
        }
        // Shells skip history for lines that start with a space; so do we.
        if raw.starts_with(' ') || raw.len() > MAX_COMMAND {
            return Ok(Recorded::Skipped);
        }
        let Some(command) = clean_text(raw) else {
            return Ok(Recorded::Skipped);
        };
        if self.settings.ignore.iter().any(|p| ignored(&command, p)) {
            return Ok(Recorded::Skipped);
        }
        let Some(red) = redact(&command) else {
            return Ok(Recorded::Skipped);
        };
        if let Some(last) = self.entries.last() {
            if last.session == session
                && last.command == red.text
                && now.saturating_sub(last.at) < 2
            {
                return Ok(Recorded::Skipped);
            }
        }
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        self.entries.push(Entry {
            id,
            at: now,
            command: red.text,
            host: host.map(str::to_string),
            session: session.to_string(),
            masked: red.masked,
        });
        self.prune(now);
        Ok(if red.masked {
            Recorded::KeptMasked
        } else {
            Recorded::Kept
        })
    }

    /// Drops entries past the retention period and beyond the cap.
    pub fn prune(&mut self, now: u64) {
        let keep_from = now.saturating_sub(u64::from(self.settings.retention_days) * 86_400);
        self.entries.retain(|e| e.at >= keep_from);
        if self.entries.len() > MAX_ENTRIES {
            let extra = self.entries.len() - MAX_ENTRIES;
            self.entries.drain(..extra);
        }
    }

    pub fn delete(&mut self, id: u64) {
        self.entries.retain(|e| e.id != id);
    }

    /// Forgets every command (settings and snippets stay).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.dismissed.clear();
    }

    pub fn dismiss(&mut self, id: &str) -> Result<(), MemoryError> {
        if !valid_label(id, 64) {
            return Err(invalid("bad suggestion id"));
        }
        if !self.dismissed.iter().any(|d| d == id) {
            self.dismissed.push(id.to_string());
            if self.dismissed.len() > MAX_DISMISSED {
                self.dismissed.remove(0);
            }
        }
        Ok(())
    }

    pub fn set_settings(&mut self, settings: Settings, now: u64) -> Result<(), MemoryError> {
        if !(1..=365).contains(&settings.retention_days) {
            return Err(invalid("keep commands for 1 to 365 days"));
        }
        if settings.ignore.len() > MAX_IGNORE {
            return Err(invalid(format!("at most {MAX_IGNORE} ignored commands")));
        }
        let mut ignore = Vec::new();
        for p in &settings.ignore {
            let p = p.trim();
            if p.is_empty() {
                continue;
            }
            if !valid_label(p, 100) {
                return Err(invalid(
                    "an ignored command is too long or has control characters",
                ));
            }
            ignore.push(p.to_string());
        }
        self.settings = Settings { ignore, ..settings };
        self.prune(now);
        Ok(())
    }

    /// Saves a snippet. Refuses commands holding a secret: snippets are
    /// plain text in this file and show in the palette.
    pub fn add_snippet(
        &mut self,
        title: &str,
        command: &str,
        now: u64,
    ) -> Result<Snippet, MemoryError> {
        let title = title.trim();
        if !valid_label(title, 100) {
            return Err(invalid("give the snippet a name (up to 100 characters)"));
        }
        if command.len() > MAX_COMMAND {
            return Err(invalid("the command is too long for a snippet"));
        }
        let command = clean_text(command)
            .ok_or_else(|| invalid("the command is empty or has control characters"))?;
        match redact(&command) {
            Some(r) if !r.masked => {}
            _ => {
                return Err(invalid(
                    "that command holds a secret, so it cannot be a snippet",
                ))
            }
        }
        if let Some(same) = self.snippets.iter().find(|s| s.command == command) {
            return Ok(same.clone());
        }
        if self.snippets.len() >= MAX_SNIPPETS {
            return Err(invalid(format!("at most {MAX_SNIPPETS} snippets")));
        }
        let snippet = Snippet {
            id: format!("s{}-{}", now, self.snippets.len() + 1),
            title: title.to_string(),
            command,
            created: now,
        };
        self.snippets.push(snippet.clone());
        Ok(snippet)
    }

    pub fn delete_snippet(&mut self, id: &str) {
        self.snippets.retain(|s| s.id != id);
    }
}

/// Where the memory file lives.
#[derive(Debug, Clone)]
pub struct MemoryStore {
    path: PathBuf,
}

impl MemoryStore {
    pub fn in_dir(config_dir: &Path) -> Self {
        MemoryStore {
            path: config_dir.join("work-memory.json"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The file, or an empty memory when there is none. A file that does
    /// not parse is an error, so a save never throws away what is there.
    pub fn load(&self) -> Result<MemoryFile, MemoryError> {
        if let Ok(meta) = std::fs::metadata(&self.path) {
            if meta.len() > MAX_FILE {
                return Err(MemoryError::Parse {
                    path: self.path.clone(),
                    message: format!(
                        "larger than {} MiB; not a Setu work memory file",
                        MAX_FILE / 1024 / 1024
                    ),
                });
            }
        }
        let text = match std::fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(MemoryFile::default()),
            Err(source) => {
                return Err(MemoryError::Read {
                    path: self.path.clone(),
                    source,
                })
            }
        };
        let file: MemoryFile = serde_json::from_str(&text).map_err(|e| MemoryError::Parse {
            path: self.path.clone(),
            message: e.to_string(),
        })?;
        if file.version > FORMAT_VERSION {
            return Err(MemoryError::NewerFormat {
                path: self.path.clone(),
                found: file.version,
            });
        }
        Ok(file)
    }

    pub fn save(&self, file: &MemoryFile) -> Result<(), MemoryError> {
        let json = serde_json::to_vec(file).map_err(|e| invalid(e.to_string()))?;
        write_atomic(&self.path, &json)
    }

    /// Removes the file altogether.
    pub fn wipe(&self) -> Result<(), MemoryError> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(MemoryError::Write {
                path: self.path.clone(),
                source,
            }),
        }
    }
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<(), MemoryError> {
    let err = |source| MemoryError::Write {
        path: path.to_path_buf(),
        source,
    };
    if let Some(dir) = path.parent() {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(dir).map_err(err)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> MemoryFile {
        let mut m = MemoryFile::default();
        m.settings.recording = true;
        m
    }

    #[test]
    fn nothing_is_kept_until_turned_on() {
        let mut m = MemoryFile::default();
        assert_eq!(
            m.record("uptime", None, "p1", 100).expect("ok"),
            Recorded::Off
        );
        assert!(m.entries.is_empty());
    }

    #[test]
    fn keeps_masks_and_skips() {
        let mut m = on();
        assert_eq!(
            m.record("  df   -h ", None, "p1", 100).expect("ok"),
            Recorded::Skipped
        );
        assert_eq!(
            m.record("df   -h", Some("web-1"), "p1", 100).expect("ok"),
            Recorded::Kept
        );
        assert_eq!(m.entries[0].command, "df -h");
        assert_eq!(m.entries[0].host.as_deref(), Some("web-1"));
        // The same line again at once (a double Enter) is kept once.
        assert_eq!(
            m.record("df -h", Some("web-1"), "p1", 101).expect("ok"),
            Recorded::Skipped
        );
        assert_eq!(
            m.record("df -h", Some("web-1"), "p1", 200).expect("ok"),
            Recorded::Kept
        );
        assert_eq!(
            m.record("PGPASSWORD=x psql", None, "p1", 300).expect("ok"),
            Recorded::KeptMasked
        );
        assert!(m.entries.last().expect("entry").masked);
        assert_eq!(
            m.record("echo a\x1b[A", None, "p1", 300).expect("ok"),
            Recorded::Skipped
        );
        assert_eq!(
            m.record(&"x".repeat(MAX_COMMAND + 1), None, "p1", 300)
                .expect("ok"),
            Recorded::Skipped
        );
        assert!(m.record("ls", None, "../etc", 300).is_err());
        assert_eq!(m.entries.len(), 3);
    }

    #[test]
    fn ignore_list_and_retention() {
        let mut m = on();
        let s = Settings {
            recording: true,
            retention_days: 1,
            ignore: vec![" vim ".into(), "  ".into()],
        };
        m.set_settings(s, 0).expect("settings");
        assert_eq!(m.settings.ignore, vec!["vim".to_string()]);
        assert_eq!(
            m.record("vim notes.txt", None, "p", 10).expect("ok"),
            Recorded::Skipped
        );
        assert_eq!(
            m.record("vim", None, "p", 10).expect("ok"),
            Recorded::Skipped
        );
        assert_eq!(
            m.record("vimdiff a b", None, "p", 10).expect("ok"),
            Recorded::Kept
        );
        m.record("uptime", None, "p", 10).expect("ok");
        m.record("free -m", None, "p", 10 + 2 * 86_400).expect("ok");
        assert_eq!(m.entries.len(), 1);
        assert_eq!(m.entries[0].command, "free -m");
        let bad = Settings {
            retention_days: 0,
            ..Settings::default()
        };
        assert!(m.set_settings(bad, 0).is_err());
    }

    #[test]
    fn cap_keeps_the_newest() {
        let mut m = on();
        for i in 0..(MAX_ENTRIES + 10) {
            m.record(&format!("echo {i}"), None, "p", 1000).expect("ok");
        }
        assert_eq!(m.entries.len(), MAX_ENTRIES);
        assert_eq!(
            m.entries.last().expect("last").command,
            format!("echo {}", MAX_ENTRIES + 9)
        );
    }

    #[test]
    fn snippets_refuse_secrets() {
        let mut m = on();
        assert!(m
            .add_snippet("Deploy", "DEPLOY_TOKEN=abc ./deploy", 1)
            .is_err());
        let s = m.add_snippet("Disk", "df -h /", 1).expect("snippet");
        assert_eq!(m.add_snippet("Disk again", "df -h /", 2).expect("same"), s);
        m.delete_snippet(&s.id);
        assert!(m.snippets.is_empty());
    }

    #[test]
    fn file_is_private_and_round_trips() {
        let dir = tempfile::tempdir().expect("dir");
        let store = MemoryStore::in_dir(&dir.path().join("setu"));
        assert_eq!(store.load().expect("empty"), MemoryFile::default());
        let mut m = on();
        m.record("uptime", Some("db-1"), "p", 5).expect("ok");
        store.save(&m).expect("save");
        assert_eq!(store.load().expect("load"), m);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(store.path())
                .expect("meta")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::write(store.path(), "{not json").expect("write");
        assert!(store.load().is_err());
        store.wipe().expect("wipe");
        assert!(!store.path().exists());
    }
}
