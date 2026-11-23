//! Host-key verification against an OpenSSH `known_hosts` file.
//!
//! Setu shares `~/.ssh/known_hosts` with OpenSSH, so a host trusted in one is
//! trusted in the other and nobody has to re-verify their fleet.

use crate::SshError;
use russh::keys::{HashAlg, PublicKey};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A server's public key, in a form that is safe to show and to send to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostKeyInfo {
    pub host: String,
    pub port: u16,
    /// For example `ssh-ed25519`.
    pub algorithm: String,
    /// `SHA256:...`, as printed by `ssh-keygen -lf`.
    pub fingerprint: String,
    /// The key in OpenSSH format (`ssh-ed25519 AAAA...`), used by [`trust`].
    pub openssh: String,
}

impl HostKeyInfo {
    pub(crate) fn new(host: &str, port: u16, key: &PublicKey) -> Self {
        HostKeyInfo {
            host: host.to_string(),
            port,
            algorithm: key.algorithm().as_str().to_string(),
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
            openssh: key.to_openssh().unwrap_or_default(),
        }
    }

    /// `host` for port 22, `[host]:port` otherwise, as OpenSSH writes it.
    pub fn host_label(&self) -> String {
        if self.port == 22 {
            self.host.clone()
        } else {
            format!("[{}]:{}", self.host, self.port)
        }
    }
}

/// The result of checking a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Trusted,
    Unknown,
    Changed { line: usize },
}

/// The default `known_hosts` path (`~/.ssh/known_hosts`).
pub fn default_path() -> Option<PathBuf> {
    setu_core::paths::ssh_dir(
        &setu_core::paths::ProcessEnv,
        setu_core::paths::Os::current(),
    )
    .ok()
    .map(|d| d.join("known_hosts"))
}

/// Checks `key` for `host:port` against the file at `path`. A missing file
/// means every host is unknown.
pub fn check(path: &Path, host: &str, port: u16, key: &PublicKey) -> Result<Verdict, SshError> {
    if !path.exists() {
        return Ok(Verdict::Unknown);
    }
    match russh::keys::check_known_hosts_path(host, port, key, path) {
        Ok(true) => Ok(Verdict::Trusted),
        Ok(false) => Ok(Verdict::Unknown),
        Err(russh::keys::Error::KeyChanged { line }) => Ok(Verdict::Changed { line }),
        Err(e) => Err(SshError::KnownHosts(e.to_string())),
    }
}

/// Rejects host names that could not be a real address and would mean
/// something else in a `known_hosts` line: whitespace or a newline (a second
/// entry), commas (more hosts), and OpenSSH's pattern and marker characters
/// (`*`, `?`, `!`, `|`, `@`, `[`, `]`, `#`). Without this, a host name taken
/// from an imported file could turn "trust this host" into "trust this key for
/// every host".
pub fn check_host(host: &str) -> Result<(), SshError> {
    let bad = host.is_empty()
        || host.len() > 255
        || host.chars().any(|c| {
            c.is_whitespace()
                || c.is_control()
                || matches!(
                    c,
                    ',' | '*' | '?' | '!' | '|' | '@' | '[' | ']' | '#' | '/' | '\\'
                )
        });
    if bad {
        return Err(SshError::KnownHosts(format!(
            "`{}` is not a valid host name or address",
            host.escape_debug()
        )));
    }
    Ok(())
}

/// Records a key the user has explicitly accepted. Refuses to overwrite a
/// different key already recorded for the host: that must be fixed by hand.
pub fn trust(path: &Path, info: &HostKeyInfo) -> Result<(), SshError> {
    check_host(&info.host)?;
    let key = PublicKey::from_openssh(info.openssh.trim())
        .map_err(|e| SshError::KnownHosts(format!("invalid host key: {e}")))?;
    // Write the key as parsed, never the text we were given, so nothing
    // after the key (a newline and a second entry, say) reaches the file.
    let info = &HostKeyInfo::new(&info.host, info.port, &key);
    match check(path, &info.host, info.port, &key)? {
        Verdict::Trusted => Ok(()),
        Verdict::Changed { line } => Err(SshError::HostKeyChanged {
            key: Box::new(info.clone()),
            known_hosts: path.to_path_buf(),
            line,
        }),
        Verdict::Unknown => append(path, info).map_err(|e| SshError::KnownHosts(e.to_string())),
    }
}

/// Appends `[host]:port key` in OpenSSH's format, creating the file (0600 on
/// Unix) and its directory if needed.
fn append(path: &Path, info: &HostKeyInfo) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let existing = std::fs::read(path).unwrap_or_default();
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path)?;
    if !existing.is_empty() && !existing.ends_with(b"\n") {
        file.write_all(b"\n")?;
    }
    writeln!(file, "{} {}", info.host_label(), info.openssh)
}

/// One line of a `known_hosts` file, for showing and removing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// 1-based line number.
    pub line: usize,
    /// Host patterns as written (`web-01`, `[10.0.0.5]:2222`, …). Empty
    /// when the names are hashed.
    pub hosts: Vec<String>,
    /// The names are hashed (`|1|…`), so they cannot be shown.
    pub hashed: bool,
    /// `@cert-authority` or `@revoked`, when present.
    pub marker: Option<String>,
    pub algorithm: String,
    /// `SHA256:…`, or empty when the key could not be read.
    pub fingerprint: String,
    pub comment: String,
    /// The whole line, used to check nothing changed before removing it.
    pub raw: String,
}

fn parse_line(line: usize, raw: &str) -> Option<Entry> {
    let text = raw.trim();
    if text.is_empty() || text.starts_with('#') {
        return None;
    }
    let mut fields = text.split_whitespace();
    let mut first = fields.next()?;
    let mut marker = None;
    if first.starts_with('@') {
        marker = Some(first.to_string());
        first = fields.next()?;
    }
    let algorithm = fields.next()?.to_string();
    let blob = fields.next().unwrap_or_default();
    let comment = fields.collect::<Vec<_>>().join(" ");
    let fingerprint = PublicKey::from_openssh(&format!("{algorithm} {blob}"))
        .map(|k| k.fingerprint(HashAlg::Sha256).to_string())
        .unwrap_or_default();
    let hashed = first.starts_with("|1|");
    Some(Entry {
        line,
        hosts: if hashed {
            Vec::new()
        } else {
            first.split(',').map(str::to_string).collect()
        },
        hashed,
        marker,
        algorithm,
        fingerprint,
        comment,
        raw: raw.to_string(),
    })
}

/// Every entry in the file at `path`. A missing file has none.
pub fn entries(path: &Path) -> Result<Vec<Entry>, SshError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(SshError::KnownHosts(e.to_string())),
    };
    Ok(text
        .lines()
        .enumerate()
        .filter_map(|(i, raw)| parse_line(i + 1, raw))
        .collect())
}

/// Removes line `line` if it still reads `raw`, keeping the old file as
/// `known_hosts.old` the way `ssh-keygen -R` does.
pub fn remove(path: &Path, line: usize, raw: &str) -> Result<(), SshError> {
    let err = |e: std::io::Error| SshError::KnownHosts(e.to_string());
    let text = std::fs::read_to_string(path).map_err(err)?;
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let current = line
        .checked_sub(1)
        .and_then(|i| lines.get(i))
        .map(|l| l.trim_end_matches(['\n', '\r']));
    if current != Some(raw) {
        return Err(SshError::KnownHosts(
            "the file changed since it was read; look again and retry".into(),
        ));
    }
    let kept: String = lines
        .iter()
        .enumerate()
        .filter(|(i, _)| i + 1 != line)
        .map(|(_, l)| *l)
        .collect();
    let mut backup = path.as_os_str().to_owned();
    backup.push(".old");
    std::fs::copy(path, &backup).map_err(err)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".setu-tmp");
    let tmp = PathBuf::from(tmp);
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let mode = std::fs::metadata(path)
                .map(|m| m.permissions().mode() & 0o777)
                .unwrap_or(0o600);
            opts.mode(mode);
        }
        let mut file = opts.open(&tmp).map_err(err)?;
        file.write_all(kept.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(err)?;
    }
    std::fs::rename(&tmp, path).map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIEnkv1WOdhQAC3pJI7A9/J86Oa07kSFN1vnFpLy8TsQt";
    const KEY_B: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINA/cLkKiSDVS2J0xog1wqk5C2Sck9dRPbPK21+RUqnZ";

    fn key(s: &str) -> PublicKey {
        PublicKey::from_openssh(s).expect("valid key")
    }

    #[test]
    fn missing_file_means_unknown() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("known_hosts");
        assert_eq!(
            check(&path, "h", 22, &key(KEY_A)).ok(),
            Some(Verdict::Unknown)
        );
    }

    #[test]
    fn trust_then_check_and_detect_change() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ssh").join("known_hosts");
        let info = HostKeyInfo::new("server.example", 2222, &key(KEY_A));
        assert_eq!(info.host_label(), "[server.example]:2222");
        assert!(info.fingerprint.starts_with("SHA256:"));

        trust(&path, &info).expect("trust");
        assert_eq!(
            check(&path, "server.example", 2222, &key(KEY_A)).ok(),
            Some(Verdict::Trusted)
        );
        // Same host on another port is a different entry.
        assert_eq!(
            check(&path, "server.example", 22, &key(KEY_A)).ok(),
            Some(Verdict::Unknown)
        );
        // A different key of the same type is a change, and trust refuses it.
        assert!(matches!(
            check(&path, "server.example", 2222, &key(KEY_B)),
            Ok(Verdict::Changed { .. })
        ));
        let other = HostKeyInfo::new("server.example", 2222, &key(KEY_B));
        assert!(matches!(
            trust(&path, &other),
            Err(SshError::HostKeyChanged { .. })
        ));
    }

    #[test]
    fn host_names_cannot_inject_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("known_hosts");
        for host in [
            "evil\n* ssh-ed25519 AAAA",
            "a b",
            "a,b",
            "*",
            "*.corp",
            "!x",
            "|1|abc",
            "@cert-authority",
            "[h]:22",
            "",
        ] {
            let info = HostKeyInfo::new(host, 22, &key(KEY_A));
            assert!(trust(&path, &info).is_err(), "{host:?}");
        }
        assert!(!path.exists());
        // Extra text after the key is dropped, not written.
        let mut info = HostKeyInfo::new("ok.example", 22, &key(KEY_A));
        info.openssh = format!("{KEY_A}\n* {KEY_B}");
        assert!(
            trust(&path, &info).is_err() || {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                text.lines().count() == 1 && !text.contains('*')
            }
        );
        // Real names and addresses still work.
        for host in ["web-01", "10.0.0.5", "2001:db8::1", "db.internal.example"] {
            trust(&path, &HostKeyInfo::new(host, 22, &key(KEY_A))).expect(host);
        }
    }

    #[test]
    fn trusting_twice_is_harmless() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("known_hosts");
        let info = HostKeyInfo::new("h", 22, &key(KEY_A));
        trust(&path, &info).expect("first");
        trust(&path, &info).expect("second");
        let text = std::fs::read_to_string(&path).expect("read");
        assert_eq!(text.lines().count(), 1);
    }

    #[test]
    fn lists_and_removes_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("known_hosts");
        let text = format!(
            "# comment\nweb-01,10.0.0.5 {KEY_A}\n|1|abc=|def= {KEY_B}\n@cert-authority *.corp {KEY_A} ca\n[db]:2222 {KEY_B}\n"
        );
        std::fs::write(&path, &text).expect("write");
        let list = entries(&path).expect("entries");
        assert_eq!(list.len(), 4);
        assert_eq!(list[0].line, 2);
        assert_eq!(list[0].hosts, vec!["web-01", "10.0.0.5"]);
        assert!(list[0].fingerprint.starts_with("SHA256:"));
        assert!(list[1].hashed && list[1].hosts.is_empty());
        assert_eq!(list[2].marker.as_deref(), Some("@cert-authority"));
        assert_eq!(list[2].comment, "ca");

        // A stale view is refused.
        assert!(remove(&path, 2, "something else").is_err());
        remove(&path, 2, &list[0].raw).expect("remove");
        let after = entries(&path).expect("entries");
        assert_eq!(after.len(), 3);
        assert!(after
            .iter()
            .all(|e| !e.hosts.contains(&"web-01".to_string())));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("known_hosts.old")).expect("old"),
            text
        );
        assert_eq!(
            entries(&dir.path().join("missing")).expect("missing").len(),
            0
        );
    }
}
