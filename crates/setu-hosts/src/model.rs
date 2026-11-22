use crate::HostsError;
use serde::{Deserialize, Serialize};

/// The hosts file format this version of Setu reads and writes.
pub const FORMAT_VERSION: u32 = 1;

/// How Setu authenticates to a saved host. No secret is ever stored.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Auth {
    /// ssh-agent, then the default keys in `~/.ssh`, like OpenSSH.
    #[default]
    Auto,
    /// Only keys held by ssh-agent (or Pageant on Windows).
    Agent,
    /// The key file in `key_file`; Setu asks for its passphrase if needed.
    Key,
    /// Ask for the password at every connection.
    Password,
}

impl Auth {
    fn is_auto(&self) -> bool {
        *self == Auth::Auto
    }
}

fn default_port() -> u16 {
    22
}

fn is_default_port(port: &u16) -> bool {
    *port == 22
}

/// One saved host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    /// What the user sees and searches for. Unique in the inventory.
    pub name: String,
    /// Address or DNS name to connect to.
    pub host: String,
    #[serde(default = "default_port", skip_serializing_if = "is_default_port")]
    pub port: u16,
    /// Login name; when empty Setu uses your local user name, like OpenSSH.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Slash-separated folder path, such as `Production/Web`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Auth::is_auto")]
    pub auth: Auth,
    /// Private key path for `auth = "key"`; `~/` is expanded at connect time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_file: Option<String>,
    /// ProxyJump host as `[user@]host[:port]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl Host {
    /// A host with only the required fields set.
    pub fn new(name: impl Into<String>, host: impl Into<String>) -> Self {
        Host {
            name: name.into(),
            host: host.into(),
            port: 22,
            user: None,
            folder: None,
            tags: Vec::new(),
            auth: Auth::Auto,
            key_file: None,
            jump: None,
            notes: None,
        }
    }

    /// Trims and tidies every field, then checks the host can be connected to.
    pub fn normalize(&mut self) -> Result<(), HostsError> {
        self.name = self.name.trim().to_string();
        self.host = self.host.trim().to_string();
        trim_opt(&mut self.user);
        trim_opt(&mut self.key_file);
        trim_opt(&mut self.jump);
        if let Some(notes) = &self.notes {
            if notes.trim().is_empty() {
                self.notes = None;
            }
        }
        self.folder = self.folder.as_deref().and_then(normalize_folder);

        let mut tags: Vec<String> = Vec::new();
        for tag in &self.tags {
            let tag = tag.trim();
            if !tag.is_empty() && !tags.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
                tags.push(tag.to_string());
            }
        }
        self.tags = tags;

        if self.name.is_empty() {
            return Err(HostsError::Invalid("a host needs a name".into()));
        }
        if self.host.is_empty() {
            return Err(HostsError::Invalid(format!(
                "`{}` needs an address to connect to",
                self.name
            )));
        }
        if self.host.chars().any(char::is_whitespace) {
            return Err(HostsError::Invalid(format!(
                "the address `{}` contains a space",
                self.host
            )));
        }
        // These would mean something else in known_hosts (patterns, markers,
        // lists of hosts); no real address contains them.
        if let Some(c) = self.host.chars().find(|c| {
            c.is_control()
                || matches!(
                    c,
                    ',' | '*' | '?' | '!' | '|' | '@' | '[' | ']' | '#' | '/' | '\\'
                )
        }) {
            return Err(HostsError::Invalid(format!(
                "the address `{}` contains `{}`, which no host name or address has",
                self.host.escape_debug(),
                c.escape_debug()
            )));
        }
        if self.port == 0 {
            return Err(HostsError::Invalid(
                "the port must be from 1 to 65535".into(),
            ));
        }
        if self.auth == Auth::Key && self.key_file.is_none() {
            return Err(HostsError::Invalid(format!(
                "`{}` uses a key file, but no key file is set",
                self.name
            )));
        }
        Ok(())
    }

    /// True when both entries reach the same server the same way, ignoring
    /// how they are organised (name, folder, tags, notes).
    pub fn same_connection(&self, other: &Host) -> bool {
        self.host == other.host
            && self.port == other.port
            && self.user == other.user
            && self.auth == other.auth
            && self.key_file == other.key_file
            && self.jump == other.jump
    }
}

fn trim_opt(value: &mut Option<String>) {
    *value = value
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string);
}

/// `" /Prod//Web/ "` becomes `Prod/Web`; an empty path means no folder.
fn normalize_folder(folder: &str) -> Option<String> {
    let parts: Vec<&str> = folder
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// The whole hosts file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    pub version: u32,
    #[serde(default, rename = "host", skip_serializing_if = "Vec::is_empty")]
    pub hosts: Vec<Host>,
}

impl Default for Inventory {
    fn default() -> Self {
        Inventory {
            version: FORMAT_VERSION,
            hosts: Vec::new(),
        }
    }
}

impl Inventory {
    /// Finds a host by name, ignoring ASCII case.
    pub fn get(&self, name: &str) -> Option<&Host> {
        self.hosts
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name.trim()))
    }

    fn position(&self, name: &str) -> Option<usize> {
        self.hosts
            .iter()
            .position(|h| h.name.eq_ignore_ascii_case(name.trim()))
    }

    /// Adds `host`, or replaces the host currently named `previous` (for edits
    /// and renames). Hosts keep their place in the file.
    pub fn upsert(&mut self, mut host: Host, previous: Option<&str>) -> Result<(), HostsError> {
        host.normalize()?;
        let existing = match previous {
            Some(prev) => Some(
                self.position(prev)
                    .ok_or_else(|| HostsError::NotFound(prev.to_string()))?,
            ),
            None => None,
        };
        if let Some(clash) = self.position(&host.name) {
            if Some(clash) != existing {
                return Err(HostsError::Duplicate(host.name));
            }
        }
        match existing {
            Some(i) => self.hosts[i] = host,
            None => self.hosts.push(host),
        }
        Ok(())
    }

    /// Removes the host named `name`.
    pub fn remove(&mut self, name: &str) -> Result<Host, HostsError> {
        let i = self
            .position(name)
            .ok_or_else(|| HostsError::NotFound(name.to_string()))?;
        Ok(self.hosts.remove(i))
    }

    /// Every folder in use, including parents, sorted.
    pub fn folders(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for folder in self.hosts.iter().filter_map(|h| h.folder.as_deref()) {
            let mut path = String::new();
            for part in folder.split('/') {
                if !path.is_empty() {
                    path.push('/');
                }
                path.push_str(part);
                if !out.contains(&path) {
                    out.push(path.clone());
                }
            }
        }
        out.sort();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_tidies_fields() {
        let mut h = Host::new("  web ", " 10.0.0.1 ");
        h.user = Some("  ".into());
        h.folder = Some(" /Prod//Web/ ".into());
        h.tags = vec![" nginx".into(), "NGINX".into(), "".into(), "db".into()];
        h.notes = Some("\n".into());
        h.normalize().expect("valid");
        assert_eq!(h.name, "web");
        assert_eq!(h.host, "10.0.0.1");
        assert_eq!(h.user, None);
        assert_eq!(h.folder.as_deref(), Some("Prod/Web"));
        assert_eq!(h.tags, vec!["nginx", "db"]);
        assert_eq!(h.notes, None);
    }

    #[test]
    fn normalize_rejects_unusable_hosts() {
        assert!(Host::new("", "a").normalize().is_err());
        assert!(Host::new("a", "").normalize().is_err());
        assert!(Host::new("a", "b c").normalize().is_err());
        for bad in ["*", "*.corp", "a,b", "|1|x", "@ca", "[h]:22", "h\u{7}"] {
            assert!(Host::new("a", bad).normalize().is_err(), "{bad:?}");
        }
        assert!(Host::new("a", "2001:db8::1").normalize().is_ok());
        let mut h = Host::new("a", "b");
        h.port = 0;
        assert!(h.normalize().is_err());
        let mut h = Host::new("a", "b");
        h.auth = Auth::Key;
        assert!(h.normalize().is_err());
    }

    #[test]
    fn upsert_adds_renames_and_rejects_duplicates() {
        let mut inv = Inventory::default();
        inv.upsert(Host::new("web", "10.0.0.1"), None).expect("add");
        inv.upsert(Host::new("db", "10.0.0.2"), None).expect("add");
        assert!(matches!(
            inv.upsert(Host::new("WEB", "x"), None),
            Err(HostsError::Duplicate(_))
        ));
        // Rename keeps the position.
        inv.upsert(Host::new("web-01", "10.0.0.1"), Some("web"))
            .expect("rename");
        assert_eq!(inv.hosts[0].name, "web-01");
        // Renaming onto another host is refused.
        assert!(inv.upsert(Host::new("db", "x"), Some("web-01")).is_err());
        // Changing only the case of its own name is fine.
        inv.upsert(Host::new("Web-01", "10.0.0.1"), Some("web-01"))
            .expect("case change");
        assert!(matches!(
            inv.upsert(Host::new("x", "y"), Some("missing")),
            Err(HostsError::NotFound(_))
        ));
        inv.remove("db").expect("remove");
        assert_eq!(inv.hosts.len(), 1);
    }

    #[test]
    fn folders_include_parents() {
        let mut inv = Inventory::default();
        let mut a = Host::new("a", "a");
        a.folder = Some("Prod/Web".into());
        let mut b = Host::new("b", "b");
        b.folder = Some("Dev".into());
        inv.upsert(a, None).expect("a");
        inv.upsert(b, None).expect("b");
        inv.upsert(Host::new("c", "c"), None).expect("c");
        assert_eq!(inv.folders(), vec!["Dev", "Prod", "Prod/Web"]);
    }
}
