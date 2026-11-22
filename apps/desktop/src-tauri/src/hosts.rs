//! Saved hosts: list, add, edit, delete, and import from ~/.ssh/config.

use serde::Serialize;
use setu_core::paths::{self, Os, ProcessEnv};
use setu_hosts::import::{self, Candidate, Summary};
use setu_hosts::ssh_config::SshConfig;
use setu_hosts::{Host, HostStore, Inventory};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

/// The hosts file, with a lock so two commands never write it at once.
#[derive(Debug)]
pub struct Hosts {
    store: Result<HostStore, String>,
    lock: Mutex<()>,
}

impl Hosts {
    pub fn from_env() -> Self {
        let store = paths::config_dir(&ProcessEnv, Os::current())
            .map(|dir| HostStore::in_dir(&dir))
            .map_err(|e| e.to_string());
        Hosts {
            store,
            lock: Mutex::new(()),
        }
    }

    fn store(&self) -> Result<&HostStore, String> {
        self.store.as_ref().map_err(Clone::clone)
    }

    /// Loads, changes and saves the inventory while holding the lock.
    fn update<T>(
        &self,
        change: impl FnOnce(&mut Inventory) -> Result<T, setu_hosts::HostsError>,
    ) -> Result<(T, HostsView), String> {
        let store = self.store()?;
        let _guard = self.lock.lock().map_err(|_| "hosts lock poisoned")?;
        let mut inventory = store.load().map_err(|e| e.to_string())?;
        let out = change(&mut inventory).map_err(|e| e.to_string())?;
        store.save(&inventory).map_err(|e| e.to_string())?;
        Ok((out, view(store, inventory)))
    }
}

#[derive(Debug, Serialize)]
pub struct HostsView {
    hosts: Vec<Host>,
    folders: Vec<String>,
    /// Where the file lives, shown so users can find and back it up.
    path: String,
}

fn view(store: &HostStore, inventory: Inventory) -> HostsView {
    HostsView {
        folders: inventory.folders(),
        hosts: inventory.hosts,
        path: store.path().display().to_string(),
    }
}

#[tauri::command]
pub fn hosts_list(state: State<'_, Hosts>) -> Result<HostsView, String> {
    let store = state.store()?;
    let _guard = state.lock.lock().map_err(|_| "hosts lock poisoned")?;
    let inventory = store.load().map_err(|e| e.to_string())?;
    Ok(view(store, inventory))
}

/// Adds a host, or replaces the one named `previous` when editing.
#[tauri::command]
pub fn host_save(
    state: State<'_, Hosts>,
    host: Host,
    previous: Option<String>,
) -> Result<HostsView, String> {
    state
        .update(|inv| inv.upsert(host, previous.as_deref()))
        .map(|(_, v)| v)
}

#[tauri::command]
pub fn host_delete(state: State<'_, Hosts>, name: String) -> Result<HostsView, String> {
    state.update(|inv| inv.remove(&name)).map(|(_, v)| v)
}

#[derive(Debug, Serialize)]
pub struct Scan {
    path: String,
    found: bool,
    candidates: Vec<Candidate>,
    warnings: Vec<String>,
}

fn ssh_paths() -> Result<(PathBuf, PathBuf), String> {
    let ssh = paths::ssh_dir(&ProcessEnv, Os::current()).map_err(|e| e.to_string())?;
    let home = ssh.parent().map(PathBuf::from).unwrap_or_default();
    Ok((ssh.join("config"), home))
}

/// Reads ~/.ssh/config and compares its hosts with the saved ones.
#[tauri::command]
pub fn ssh_config_scan(state: State<'_, Hosts>) -> Result<Scan, String> {
    let (path, home) = ssh_paths()?;
    let config = match SshConfig::load(&path, &home) {
        Ok(config) => config,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Scan {
                path: path.display().to_string(),
                found: false,
                candidates: Vec::new(),
                warnings: Vec::new(),
            })
        }
        Err(e) => return Err(format!("could not read {}: {e}", path.display())),
    };
    let store = state.store()?;
    let inventory = store.load().map_err(|e| e.to_string())?;
    Ok(Scan {
        path: path.display().to_string(),
        found: true,
        candidates: import::plan(&config, &inventory),
        warnings: config.warnings,
    })
}

#[derive(Debug, Serialize)]
pub struct Imported {
    summary: Summary,
    view: HostsView,
}

#[tauri::command]
pub fn ssh_config_import(
    state: State<'_, Hosts>,
    hosts: Vec<Host>,
    folder: Option<String>,
) -> Result<Imported, String> {
    let folder = folder.filter(|f| !f.trim().is_empty());
    state
        .update(|inv| import::apply(inv, hosts, folder.as_deref()))
        .map(|(summary, view)| Imported { summary, view })
}

#[derive(Debug, Serialize)]
pub struct FileScan {
    /// What the file was read as, such as "PuTTY sessions".
    format: String,
    candidates: Vec<Candidate>,
    /// Entries left out on purpose, such as telnet sessions.
    skipped: Vec<String>,
}

/// Reads hosts from another tool's file (PuTTY .reg, MobaXterm, CSV,
/// Ansible). Nothing is saved until `ssh_config_import`.
#[tauri::command]
pub fn hosts_parse_file(
    state: State<'_, Hosts>,
    file_name: String,
    bytes: Vec<u8>,
) -> Result<FileScan, String> {
    use setu_hosts::sources;
    if bytes.len() > 5 * 1024 * 1024 {
        return Err("that file is over 5 MB; is it really a list of hosts?".into());
    }
    let text = sources::decode_text(&bytes);
    let format = sources::detect(&file_name, &text).ok_or_else(|| {
        "could not tell what kind of file this is. Setu reads PuTTY .reg exports, MobaXterm .mxtsessions, CSV and Ansible inventories".to_string()
    })?;
    let parsed = sources::parse(format, &text);
    let inventory = state.store()?.load().map_err(|e| e.to_string())?;
    Ok(FileScan {
        format: format.label().to_string(),
        candidates: import::plan_hosts(parsed.hosts, &inventory),
        skipped: parsed.skipped,
    })
}
