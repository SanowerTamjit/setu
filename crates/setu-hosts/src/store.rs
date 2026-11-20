use crate::{HostsError, Inventory, FORMAT_VERSION};
use std::io::Write;
use std::path::{Path, PathBuf};

/// File name of the inventory inside Setu's config folder.
pub const HOSTS_FILE: &str = "hosts.toml";

const HEADER: &str = "\
# Setu saved hosts. Plain TOML: safe to edit by hand, back up or keep in Git.
# No passwords or keys are stored here. Format: docs/hosts.md in the Setu repo.

";

/// Reads and writes the inventory file.
#[derive(Debug, Clone)]
pub struct HostStore {
    path: PathBuf,
}

impl HostStore {
    /// A store for `hosts.toml` inside `config_dir`.
    pub fn in_dir(config_dir: &Path) -> Self {
        HostStore {
            path: config_dir.join(HOSTS_FILE),
        }
    }

    /// A store for an explicit file path.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        HostStore { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads the inventory. A missing file is an empty inventory.
    pub fn load(&self) -> Result<Inventory, HostsError> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Inventory::default()),
            Err(source) => {
                return Err(HostsError::Read {
                    path: self.path.clone(),
                    source,
                })
            }
        };
        let inventory: Inventory = toml::from_str(&text).map_err(|e| HostsError::Parse {
            path: self.path.clone(),
            message: e.message().to_string(),
        })?;
        if inventory.version > FORMAT_VERSION {
            return Err(HostsError::NewerFormat {
                path: self.path.clone(),
                found: inventory.version,
            });
        }
        Ok(inventory)
    }

    /// Saves the inventory, replacing the file atomically so a crash never
    /// leaves a half-written hosts file.
    pub fn save(&self, inventory: &Inventory) -> Result<(), HostsError> {
        if inventory.version > FORMAT_VERSION {
            return Err(HostsError::NewerFormat {
                path: self.path.clone(),
                found: inventory.version,
            });
        }
        let mut current = inventory.clone();
        current.version = FORMAT_VERSION;
        let body = toml::to_string(&current).map_err(|e| HostsError::Invalid(e.to_string()))?;

        let write_err = |source| HostsError::Write {
            path: self.path.clone(),
            source,
        };
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(write_err)?;
        }
        let mut tmp_name = self.path.as_os_str().to_owned();
        tmp_name.push(".tmp");
        let tmp = PathBuf::from(tmp_name);
        let result = (|| {
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(HEADER.as_bytes())?;
            file.write_all(body.as_bytes())?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&tmp, &self.path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result.map_err(write_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Auth, Host};

    #[test]
    fn missing_file_is_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let inv = HostStore::in_dir(dir.path()).load().expect("load");
        assert!(inv.hosts.is_empty());
    }

    #[test]
    fn round_trip_writes_readable_toml() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = HostStore::in_dir(&dir.path().join("nested"));
        let mut inv = Inventory::default();
        let mut web = Host::new("web-01", "10.0.4.21");
        web.user = Some("deploy".into());
        web.folder = Some("Production/Web".into());
        web.tags = vec!["nginx".into()];
        web.auth = Auth::Key;
        web.key_file = Some("~/.ssh/id_ed25519".into());
        inv.upsert(web, None).expect("add");
        let mut db = Host::new("db", "db.internal");
        db.port = 2222;
        inv.upsert(db, None).expect("add");
        store.save(&inv).expect("save");

        let text = std::fs::read_to_string(store.path()).expect("read");
        assert!(text.starts_with("# Setu saved hosts."));
        assert!(text.contains("[[host]]\nname = \"web-01\""));
        assert!(text.contains("tags = [\"nginx\"]"));
        // Defaults stay out of the file.
        assert!(!text.contains("port = 22\n"));
        assert!(!text.contains("auth = \"auto\""));
        assert!(text.contains("port = 2222"));

        assert_eq!(store.load().expect("load"), inv);
        assert!(!store.path().with_extension("toml.tmp").exists());
    }

    #[test]
    fn hand_written_file_loads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = HostStore::in_dir(dir.path());
        std::fs::write(
            store.path(),
            "version = 1\n\n# my bastion\n[[host]]\nname = \"bastion\"\nhost = \"203.0.113.10\"\nauth = \"agent\"\n",
        )
        .expect("write");
        let inv = store.load().expect("load");
        assert_eq!(inv.hosts[0].auth, Auth::Agent);
        assert_eq!(inv.hosts[0].port, 22);
    }

    #[test]
    fn bad_and_newer_files_are_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = HostStore::in_dir(dir.path());
        std::fs::write(store.path(), "version = 1\n[[host]]\nname = 3\n").expect("write");
        assert!(matches!(store.load(), Err(HostsError::Parse { .. })));

        std::fs::write(store.path(), "version = 9\n").expect("write");
        assert!(matches!(
            store.load(),
            Err(HostsError::NewerFormat { found: 9, .. })
        ));
        let newer = Inventory {
            version: 9,
            hosts: vec![],
        };
        assert!(store.save(&newer).is_err());
    }
}
