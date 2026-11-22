//! Turning `~/.ssh/config` hosts into saved hosts.

use crate::ssh_config::SshConfig;
use crate::{Auth, Host, HostsError, Inventory};
use serde::Serialize;

/// How an imported host relates to the saved ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Not saved yet.
    New,
    /// Saved with the same connection settings.
    Unchanged,
    /// Saved under this name, but the settings differ.
    Changed,
    /// Cannot be imported; `notes` says why.
    Invalid,
}

/// One host found in the SSH config, ready to show in the import list.
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub host: Host,
    pub status: Status,
    /// What could not be carried over, worded for the user.
    pub notes: Vec<String>,
}

/// Expands the `%h` and `%%` tokens OpenSSH allows in `HostName`.
fn expand_hostname(value: &str, alias: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('h') => out.push_str(alias),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

/// OpenSSH looks jump hosts up in the config too, so `ProxyJump bastion`
/// means the `Host bastion` entry. Spell that out as `user@address:port`,
/// because Setu connects to the hop directly.
fn expand_jump_hop(config: &SshConfig, hop: &str) -> String {
    let (user, rest) = match hop.rsplit_once('@') {
        Some((u, r)) => (Some(u), r),
        None => (None, hop),
    };
    let (name, port) = match rest.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (rest, None),
    };
    if !config
        .aliases()
        .iter()
        .any(|a| a.eq_ignore_ascii_case(name))
    {
        return hop.to_string();
    }
    let resolved = config.resolve(name);
    let address = resolved
        .get("hostname")
        .map(|h| expand_hostname(h, name))
        .unwrap_or_else(|| name.to_string());
    let user = user.or(resolved.get("user"));
    let port = port.or(resolved.get("port")).filter(|p| *p != "22");
    let mut out = String::new();
    if let Some(user) = user {
        out.push_str(user);
        out.push('@');
    }
    out.push_str(&address);
    if let Some(port) = port {
        out.push(':');
        out.push_str(port);
    }
    out
}

/// Builds the saved-host form of one SSH config alias.
pub fn host_from_config(config: &SshConfig, alias: &str) -> (Host, Vec<String>) {
    let resolved = config.resolve(alias);
    let mut notes = Vec::new();
    let address = resolved
        .get("hostname")
        .map(|h| expand_hostname(h, alias))
        .unwrap_or_else(|| alias.to_string());
    let mut host = Host::new(alias, address);
    if let Some(port) = resolved.get("port") {
        match port.parse::<u16>() {
            Ok(p) if p > 0 => host.port = p,
            _ => notes.push(format!("Port `{port}` is not a number; using 22.")),
        }
    }
    host.user = resolved.get("user").map(str::to_string);
    if let Some(key) = resolved.get("identityfile") {
        if !key.eq_ignore_ascii_case("none") {
            host.auth = Auth::Key;
            host.key_file = Some(key.to_string());
        }
    }
    if let Some(jump) = resolved.get("proxyjump") {
        if !jump.eq_ignore_ascii_case("none") {
            host.jump = Some(
                jump.split(',')
                    .map(|hop| expand_jump_hop(config, hop.trim()))
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
    }
    if resolved.has("proxycommand") && host.jump.is_none() {
        notes.push("Uses ProxyCommand, which Setu does not run; imported without it.".into());
    }
    (host, notes)
}

/// Lists every concrete host in `config` with its status against `inventory`.
pub fn plan(config: &SshConfig, inventory: &Inventory) -> Vec<Candidate> {
    plan_hosts(
        config
            .aliases()
            .iter()
            .map(|alias| host_from_config(config, alias))
            .collect(),
        inventory,
    )
}

/// Gives each host found elsewhere (another tool's file) its status
/// against `inventory`.
pub fn plan_hosts(hosts: Vec<(Host, Vec<String>)>, inventory: &Inventory) -> Vec<Candidate> {
    hosts
        .into_iter()
        .map(|(mut host, mut notes)| {
            if let Err(e) = host.normalize() {
                notes.push(e.to_string());
                return Candidate {
                    host,
                    status: Status::Invalid,
                    notes,
                };
            }
            let status = match inventory.get(&host.name) {
                None => Status::New,
                Some(saved) if saved.same_connection(&host) => Status::Unchanged,
                Some(_) => Status::Changed,
            };
            if status == Status::Changed {
                notes.push(
                    "A saved host has this name; importing updates its connection settings.".into(),
                );
            }
            Candidate {
                host,
                status,
                notes,
            }
        })
        .collect()
}

/// What [`apply`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub added: usize,
    pub updated: usize,
}

/// Adds the chosen hosts to `inventory`. New hosts go into `folder` (their
/// own folder, if they came with one, goes inside it); hosts that are
/// already saved keep their folder, tags and notes and only take the new
/// connection settings.
pub fn apply(
    inventory: &mut Inventory,
    hosts: Vec<Host>,
    folder: Option<&str>,
) -> Result<Summary, HostsError> {
    let mut summary = Summary::default();
    for mut host in hosts {
        match inventory.get(&host.name).cloned() {
            Some(saved) => {
                if saved.same_connection(&host) {
                    continue;
                }
                host.name = saved.name.clone();
                host.folder = saved.folder.clone();
                host.tags = saved.tags.clone();
                host.notes = saved.notes.clone();
                inventory.upsert(host, Some(&saved.name))?;
                summary.updated += 1;
            }
            None => {
                host.folder = match (folder, host.folder.take()) {
                    (Some(f), Some(own)) => Some(format!("{f}/{own}")),
                    (Some(f), None) => Some(f.to_string()),
                    (None, own) => own,
                };
                inventory.upsert(host, None)?;
                summary.added += 1;
            }
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const CONFIG: &str = "\
Host *
  User me
Host web-01
  HostName 10.0.4.%h
  Port 2222
  IdentityFile ~/.ssh/work_ed25519
Host db
  HostName db.internal
  ProxyJump ops@bastion
Host legacy
  ProxyCommand nc -X 5 -x proxy:1080 %h %p
Host none-key
  IdentityFile none
Host *.corp
  User corp
Host bastion
  HostName 203.0.113.10
  Port 2200
Host app
  ProxyJump bastion,root@db:2022
";

    fn config() -> SshConfig {
        SshConfig::parse_with_includes(CONFIG, Path::new("/nonexistent-home"))
    }

    #[test]
    fn converts_config_hosts() {
        let config = config();
        let (web, notes) = host_from_config(&config, "web-01");
        assert_eq!(web.host, "10.0.4.web-01");
        assert_eq!(web.port, 2222);
        assert_eq!(web.user.as_deref(), Some("me"));
        assert_eq!(web.auth, Auth::Key);
        assert_eq!(web.key_file.as_deref(), Some("~/.ssh/work_ed25519"));
        assert!(notes.is_empty());

        let (db, _) = host_from_config(&config, "db");
        assert_eq!(db.jump.as_deref(), Some("ops@203.0.113.10:2200"));
        assert_eq!(db.auth, Auth::Auto);

        let (legacy, notes) = host_from_config(&config, "legacy");
        assert_eq!(legacy.host, "legacy");
        assert_eq!(notes.len(), 1);

        let (none_key, _) = host_from_config(&config, "none-key");
        assert_eq!(none_key.auth, Auth::Auto);

        // Jump hosts that are config aliases are spelled out.
        let (app, _) = host_from_config(&config, "app");
        assert_eq!(
            app.jump.as_deref(),
            Some("me@203.0.113.10:2200,root@db.internal:2022")
        );
    }

    #[test]
    fn plan_and_apply_against_saved_hosts() {
        let config = config();
        let mut inventory = Inventory::default();
        let mut saved_db = Host::new("db", "db.internal");
        saved_db.user = Some("me".into());
        saved_db.jump = Some("ops@203.0.113.10:2200".into());
        saved_db.folder = Some("Databases".into());
        inventory.upsert(saved_db, None).expect("add");
        let mut saved_web = Host::new("WEB-01", "old.address");
        saved_web.tags = vec!["nginx".into()];
        saved_web.folder = Some("Web".into());
        inventory.upsert(saved_web, None).expect("add");

        let plan = plan(&config, &inventory);
        let names: Vec<_> = plan.iter().map(|c| c.host.name.as_str()).collect();
        assert_eq!(
            names,
            ["web-01", "db", "legacy", "none-key", "bastion", "app"]
        );
        assert_eq!(plan[0].status, Status::Changed);
        assert_eq!(plan[1].status, Status::Unchanged);
        assert_eq!(plan[2].status, Status::New);

        let picked = plan.into_iter().map(|c| c.host).collect();
        let summary = apply(&mut inventory, picked, Some("ssh config")).expect("apply");
        assert_eq!(
            summary,
            Summary {
                added: 4,
                updated: 1
            }
        );
        let web = inventory.get("web-01").expect("web");
        // Organisation kept, connection updated, saved name kept.
        assert_eq!(web.name, "WEB-01");
        assert_eq!(web.folder.as_deref(), Some("Web"));
        assert_eq!(web.tags, vec!["nginx"]);
        assert_eq!(web.host, "10.0.4.web-01");
        assert_eq!(
            inventory.get("legacy").and_then(|h| h.folder.as_deref()),
            Some("ssh config")
        );
        assert_eq!(inventory.hosts.len(), 6);
    }
}
