//! Saved SSH tunnels (port forwards), kept in `tunnels.toml` next to
//! `hosts.toml`.
//!
//! ```toml
//! version = 1
//!
//! [[tunnel]]
//! id = "db-admin"
//! name = "Postgres on db-01"
//! host = "db-01"            # a saved host's name
//! kind = "local"            # local (-L), remote (-R) or dynamic (-D, SOCKS5)
//! bind_port = 5433          # listen here on this computer
//! target_host = "127.0.0.1" # as seen from the server
//! target_port = 5432
//! auto_start = true
//! ```
//!
//! Nothing secret is stored: a tunnel logs in the way its saved host does.
//! `bind_address` defaults to loopback, so only this computer can use a
//! tunnel unless the file says otherwise.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// File name inside Setu's config folder.
pub const TUNNELS_FILE: &str = "tunnels.toml";
/// The format this version of Setu reads and writes.
pub const TUNNELS_VERSION: u32 = 1;
/// More than this many saved tunnels is not a file Setu wrote.
pub const MAX_TUNNELS: usize = 200;
/// Files bigger than this are refused before parsing.
const MAX_FILE_BYTES: u64 = 512 * 1024;

const LOOPBACK: &str = "127.0.0.1";

/// Which way a tunnel carries connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TunnelKind {
    /// `ssh -L`: a port here reaches a host and port the server can reach.
    Local,
    /// `ssh -R`: a port on the server reaches a host and port this computer can reach.
    Remote,
    /// `ssh -D`: a SOCKS5 proxy here; apps choose where to go through the server.
    Dynamic,
}

fn loopback() -> String {
    LOOPBACK.into()
}

fn is_loopback_default(address: &str) -> bool {
    address == LOOPBACK
}

/// One saved tunnel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tunnel {
    /// Stable id: lowercase letters, digits and `-`.
    pub id: String,
    pub name: String,
    /// The saved host whose login carries the tunnel.
    pub host: String,
    pub kind: TunnelKind,
    /// Where to listen: on this computer (local, dynamic) or on the server
    /// (remote). Loopback unless the person chose otherwise.
    #[serde(default = "loopback", skip_serializing_if = "is_loopback_default")]
    pub bind_address: String,
    pub bind_port: u16,
    /// Where connections go (local and remote only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_port: Option<u16>,
    /// Start when Setu opens.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto_start: bool,
}

impl Tunnel {
    /// Whether anything other than this computer (or, for a remote tunnel,
    /// the server itself) can connect to the port.
    pub fn is_shared(&self) -> bool {
        !is_loopback(&self.bind_address)
    }

    /// Checks every field, so a hand-edited or hostile file cannot make
    /// Setu listen or connect somewhere odd.
    pub fn validate(&self) -> Result<(), String> {
        if !valid_id(&self.id) {
            return Err(format!(
                "tunnel id {:?} must be 1 to 40 lowercase letters, digits or -",
                self.id
            ));
        }
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
            return Err("a tunnel needs a name of up to 80 characters".into());
        }
        if self.host.trim().is_empty() || self.host.chars().any(char::is_control) {
            return Err(format!("tunnel {name:?} needs a saved host"));
        }
        if !valid_address(&self.bind_address) {
            return Err(format!(
                "{:?} is not an address to listen on",
                self.bind_address
            ));
        }
        if self.bind_port == 0 {
            return Err(format!(
                "tunnel {name:?} needs a port to listen on (1-65535)"
            ));
        }
        match self.kind {
            TunnelKind::Dynamic => {
                if self.target_host.is_some() || self.target_port.is_some() {
                    return Err(format!(
                        "tunnel {name:?} is a SOCKS proxy, which has no fixed destination"
                    ));
                }
            }
            TunnelKind::Local | TunnelKind::Remote => {
                match self.target_host.as_deref() {
                    Some(h) if valid_address(h) => {}
                    _ => return Err(format!("tunnel {name:?} needs a destination host")),
                }
                match self.target_port {
                    Some(p) if p != 0 => {}
                    _ => return Err(format!("tunnel {name:?} needs a destination port")),
                }
            }
        }
        Ok(())
    }

    /// `-L 5433:127.0.0.1:5432` and the like, as OpenSSH would write it.
    pub fn summary(&self) -> String {
        let bind = if self.is_shared() {
            format!("{}:{}", self.bind_address, self.bind_port)
        } else {
            self.bind_port.to_string()
        };
        let target = || {
            format!(
                "{}:{}",
                self.target_host.as_deref().unwrap_or("?"),
                self.target_port.unwrap_or(0)
            )
        };
        match self.kind {
            TunnelKind::Local => format!("-L {bind}:{}", target()),
            TunnelKind::Remote => format!("-R {bind}:{}", target()),
            TunnelKind::Dynamic => format!("-D {bind}"),
        }
    }
}

/// Loopback: only this computer can connect.
pub fn is_loopback(address: &str) -> bool {
    let a = address.trim();
    a.eq_ignore_ascii_case("localhost")
        || a.parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

fn valid_id(id: &str) -> bool {
    (1..=40).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A host name or IP address: no spaces, no shell or option characters.
fn valid_address(address: &str) -> bool {
    let a = address.trim();
    !a.is_empty()
        && a.len() <= 253
        && a == address
        && !a.starts_with('-')
        && a.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':'))
}

/// The tunnels file.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TunnelList {
    #[serde(default)]
    pub version: u32,
    #[serde(default, rename = "tunnel")]
    pub tunnels: Vec<Tunnel>,
}

impl TunnelList {
    /// Checks every tunnel, and that ids are unique and no two tunnels
    /// listen on the same port.
    pub fn validate(&self) -> Result<(), String> {
        if self.tunnels.len() > MAX_TUNNELS {
            return Err(format!("more than {MAX_TUNNELS} tunnels"));
        }
        for (i, t) in self.tunnels.iter().enumerate() {
            t.validate()?;
            for other in &self.tunnels[..i] {
                if other.id == t.id {
                    return Err(format!("two tunnels have the id {:?}", t.id));
                }
                // Remote tunnels listen on their servers, not here.
                let here = |t: &Tunnel| t.kind != TunnelKind::Remote;
                if here(t) && here(other) && other.bind_port == t.bind_port {
                    return Err(format!(
                        "{:?} and {:?} both listen on port {}",
                        other.name, t.name, t.bind_port
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&Tunnel> {
        self.tunnels.iter().find(|t| t.id == id)
    }

    /// Adds `tunnel`, or replaces the one with its id.
    pub fn upsert(&mut self, tunnel: Tunnel) -> Result<(), String> {
        let mut next = self.clone();
        match next.tunnels.iter_mut().find(|t| t.id == tunnel.id) {
            Some(slot) => *slot = tunnel,
            None => next.tunnels.push(tunnel),
        }
        next.validate()?;
        *self = next;
        Ok(())
    }

    /// Removes a tunnel; false when there was none with that id.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.tunnels.len();
        self.tunnels.retain(|t| t.id != id);
        self.tunnels.len() != before
    }

    /// A new id made from `name`, not used by any tunnel yet.
    pub fn new_id(&self, name: &str) -> String {
        let mut base: String = name
            .to_ascii_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        base = base
            .split('-')
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        base.truncate(32);
        let base = base.trim_end_matches('-');
        let base = if base.is_empty() { "tunnel" } else { base };
        let mut id = base.to_string();
        let mut n = 2;
        while self.get(&id).is_some() {
            id = format!("{base}-{n}");
            n += 1;
        }
        id
    }
}

/// Reads and writes `tunnels.toml`.
#[derive(Debug, Clone)]
pub struct TunnelStore {
    path: PathBuf,
}

impl TunnelStore {
    pub fn in_dir(config_dir: &Path) -> Self {
        TunnelStore {
            path: config_dir.join(TUNNELS_FILE),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads the file; a missing file is an empty list. A file that is too
    /// big, newer, or fails a check is an error (and is never written over).
    pub fn load(&self) -> Result<TunnelList, String> {
        let shown = self.path.display();
        match std::fs::metadata(&self.path) {
            Ok(m) if m.len() > MAX_FILE_BYTES => {
                return Err(format!("{shown} is too big to be a tunnels file"))
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(TunnelList::default()),
            Err(e) => return Err(format!("could not read {shown}: {e}")),
        }
        let text = std::fs::read_to_string(&self.path)
            .map_err(|e| format!("could not read {shown}: {e}"))?;
        let list: TunnelList = toml::from_str(&text)
            .map_err(|e| format!("{shown} is not a valid tunnels file: {}", e.message()))?;
        if list.version > TUNNELS_VERSION {
            return Err(format!(
                "{shown} was written by a newer version of Setu; update Setu to change it"
            ));
        }
        list.validate()
            .map_err(|e| format!("{shown} has a problem: {e}"))?;
        Ok(list)
    }

    /// Saves atomically (temp file + rename), 0600, after checking it.
    pub fn save(&self, list: &TunnelList) -> Result<(), String> {
        list.validate()?;
        let mut list = list.clone();
        list.version = TUNNELS_VERSION;
        let body = toml::to_string(&list).map_err(|e| e.to_string())?;
        let text = format!(
            "# Setu saved tunnels. Plain TOML; no passwords are stored here.\n# Format: docs/user/tunnels.md in the Setu repo.\n\n{body}"
        );
        write_private(&self.path, text.as_bytes())
            .map_err(|e| format!("could not save {}: {e}", self.path.display()))
    }
}

fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(dir)?;
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
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(id: &str, port: u16) -> Tunnel {
        Tunnel {
            id: id.into(),
            name: format!("Tunnel {id}"),
            host: "db-01".into(),
            kind: TunnelKind::Local,
            bind_address: LOOPBACK.into(),
            bind_port: port,
            target_host: Some("127.0.0.1".into()),
            target_port: Some(5432),
            auto_start: false,
        }
    }

    #[test]
    fn round_trip_keeps_defaults_out_and_is_private() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = TunnelStore::in_dir(&dir.path().join("config"));
        let mut list = TunnelList::default();
        list.upsert(local("db", 5433)).expect("add");
        let mut socks = local("socks", 1080);
        socks.kind = TunnelKind::Dynamic;
        socks.target_host = None;
        socks.target_port = None;
        socks.auto_start = true;
        list.upsert(socks).expect("add");
        store.save(&list).expect("save");

        let text = std::fs::read_to_string(store.path()).expect("read");
        assert!(text.starts_with("# Setu saved tunnels."));
        assert!(text.contains("kind = \"dynamic\""));
        assert!(
            !text.contains("bind_address"),
            "loopback is the default: {text}"
        );
        assert!(text.contains("auto_start = true"));
        let loaded = store.load().expect("load");
        assert_eq!(loaded.tunnels, list.tunnels);
        assert_eq!(loaded.version, TUNNELS_VERSION);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).expect("meta").permissions().mode() & 0o777;
            assert_eq!(mode(store.path()), 0o600);
            assert_eq!(mode(&dir.path().join("config")), 0o700);
        }
    }

    #[test]
    fn hand_written_file_loads_with_loopback_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = TunnelStore::in_dir(dir.path());
        std::fs::write(
            store.path(),
            "version = 1\n[[tunnel]]\nid = \"web\"\nname = \"Web\"\nhost = \"web-01\"\nkind = \"local\"\nbind_port = 8080\ntarget_host = \"localhost\"\ntarget_port = 80\n",
        )
        .expect("write");
        let list = store.load().expect("load");
        assert_eq!(list.tunnels[0].bind_address, "127.0.0.1");
        assert!(!list.tunnels[0].is_shared());
        assert_eq!(list.tunnels[0].summary(), "-L 8080:localhost:80");
    }

    #[test]
    fn bad_values_are_refused() {
        let check = |f: &dyn Fn(&mut Tunnel)| {
            let mut t = local("x", 5433);
            f(&mut t);
            t.validate()
        };
        assert!(check(&|_| {}).is_ok());
        assert!(check(&|t| t.id = "Bad Id".into()).is_err());
        assert!(check(&|t| t.id = String::new()).is_err());
        assert!(check(&|t| t.name = " ".into()).is_err());
        assert!(check(&|t| t.name = "a\u{1b}[31m".into()).is_err());
        assert!(check(&|t| t.bind_port = 0).is_err());
        assert!(check(&|t| t.bind_address = "0.0.0.0 -oProxyCommand=x".into()).is_err());
        assert!(check(&|t| t.target_host = Some("-oProxyCommand=x".into())).is_err());
        assert!(check(&|t| t.target_host = Some("a;b".into())).is_err());
        assert!(check(&|t| t.target_host = None).is_err());
        assert!(check(&|t| t.target_port = Some(0)).is_err());
        assert!(check(&|t| t.host = String::new()).is_err());
        assert!(
            check(&|t| {
                t.kind = TunnelKind::Dynamic;
            })
            .is_err(),
            "a SOCKS proxy has no destination"
        );
        assert!(check(&|t| t.target_host = Some("::1".into())).is_ok());
    }

    #[test]
    fn duplicates_and_port_clashes_are_refused() {
        let mut list = TunnelList::default();
        list.upsert(local("a", 5433)).expect("add");
        assert!(list.upsert(local("b", 5433)).is_err(), "same local port");
        assert_eq!(
            list.tunnels.len(),
            1,
            "a refused change leaves the list as it was"
        );
        // A remote tunnel listens on the server, so it may reuse the number.
        let mut remote = local("r", 5433);
        remote.kind = TunnelKind::Remote;
        list.upsert(remote).expect("remote");
        // Replacing by id is an edit, not a clash.
        let mut edited = local("a", 5433);
        edited.name = "Renamed".into();
        list.upsert(edited).expect("edit");
        assert_eq!(list.get("a").map(|t| t.name.as_str()), Some("Renamed"));
        assert!(list.remove("a"));
        assert!(!list.remove("a"));
    }

    #[test]
    fn newer_big_or_broken_files_are_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = TunnelStore::in_dir(dir.path());
        std::fs::write(store.path(), "version = 9\n").expect("write");
        assert!(store
            .load()
            .err()
            .unwrap_or_default()
            .contains("newer version"));
        std::fs::write(store.path(), "version = 1\n[[tunnel]]\nid = 3\n").expect("write");
        assert!(store.load().is_err());
        std::fs::write(
            store.path(),
            "version = 1\n[[tunnel]]\nid = \"a\"\nname = \"A\"\nhost = \"h\"\nkind = \"local\"\nbind_address = \"0.0.0.0\"\nbind_port = 0\n",
        )
        .expect("write");
        assert!(store.load().err().unwrap_or_default().contains("port"));
        std::fs::write(store.path(), vec![b'#'; 600 * 1024]).expect("write");
        assert!(store.load().err().unwrap_or_default().contains("too big"));
    }

    #[test]
    fn loopback_and_ids() {
        assert!(is_loopback("127.0.0.1"));
        assert!(is_loopback("::1"));
        assert!(is_loopback("localhost"));
        assert!(!is_loopback("0.0.0.0"));
        assert!(!is_loopback("192.168.1.5"));
        let mut list = TunnelList::default();
        assert_eq!(list.new_id("Postgres on db-01!"), "postgres-on-db-01");
        list.upsert(local("postgres-on-db-01", 1)).expect("add");
        assert_eq!(list.new_id("Postgres on db-01"), "postgres-on-db-01-2");
        assert_eq!(list.new_id("???"), "tunnel");
    }
}
