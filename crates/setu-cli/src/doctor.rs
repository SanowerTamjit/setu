//! `setu doctor`: finds the setup problems that make SSH fail in confusing
//! ways, and says how to fix each one. It only reads; it never changes a
//! file, unlocks the vault or asks for a passphrase.

use serde::Serialize;
use setu_hosts::ssh_config::SshConfig;
use setu_hosts::{import, Auth, HostStore, Inventory};
use setu_ssh::keys::{self, AgentState, KeyState};
use setu_vault::{Vault, VAULT_FILE};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    Info,
    Warn,
    Fail,
}

/// One finding.
#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub section: &'static str,
    pub level: Level,
    pub title: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<String>,
    /// What to do about it, when there is something to do.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

pub const SETU: &str = "Setu";
pub const KEYS: &str = "SSH keys";
pub const AGENT: &str = "ssh-agent";
pub const CONFIG: &str = "SSH config";

impl Check {
    fn new(section: &'static str, level: Level, title: impl Into<String>) -> Self {
        Check {
            section,
            level,
            title: title.into(),
            details: Vec::new(),
            fix: None,
        }
    }

    fn fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }

    fn details(mut self, details: Vec<String>) -> Self {
        self.details = details;
        self
    }
}

/// Where to look. Tests point these at a temporary folder.
#[derive(Debug, Clone)]
pub struct Context {
    pub home: PathBuf,
    pub config_dir: Result<PathBuf, String>,
}

impl Context {
    fn ssh_dir(&self) -> PathBuf {
        self.home.join(".ssh")
    }

    /// Shows `path` with the home folder as `~`, for shorter lines.
    pub fn show(&self, path: &Path) -> String {
        match path.strip_prefix(&self.home) {
            Ok(rest) if !self.home.as_os_str().is_empty() => {
                Path::new("~").join(rest).display().to_string()
            }
            _ => path.display().to_string(),
        }
    }

    fn expand(&self, path: &str) -> PathBuf {
        match path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
            Some(rest) => self.home.join(rest),
            None if path == "~" => self.home.clone(),
            None => PathBuf::from(path),
        }
    }
}

/// Unix permission bits, or `None` where they do not apply (Windows).
fn mode(path: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).ok().map(|m| m.permissions().mode())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// Runs every check. `agent` is passed in so the slow, environment-bound
/// agent query stays out of the pure checks.
pub fn run(ctx: &Context, agent: &AgentState) -> Vec<Check> {
    let mut out = Vec::new();
    let inventory = setu_checks(ctx, &mut out);
    let config = load_ssh_config(ctx);
    let keys = key_checks(ctx, inventory.as_ref(), config.as_ref(), agent, &mut out);
    out.push(agent_check(agent, keys.encrypted));
    config_checks(ctx, config, inventory.as_ref(), &mut out);
    out
}

fn setu_checks(ctx: &Context, out: &mut Vec<Check>) -> Option<Inventory> {
    let dir = match &ctx.config_dir {
        Ok(dir) => dir.clone(),
        Err(e) => {
            out.push(
                Check::new(SETU, Level::Fail, format!("No config folder: {e}"))
                    .fix("Set HOME (or SETU_HOME to choose a folder)"),
            );
            return None;
        }
    };
    if !dir.is_dir() {
        out.push(Check::new(
            SETU,
            Level::Info,
            format!(
                "Config folder {} does not exist yet; Setu creates it when you save a host",
                ctx.show(&dir)
            ),
        ));
        return None;
    }

    let store = HostStore::in_dir(&dir);
    let inventory = if !store.path().is_file() {
        out.push(Check::new(SETU, Level::Info, "No saved hosts yet"));
        None
    } else {
        match store.load() {
            Err(e) => {
                out.push(
                    Check::new(SETU, Level::Fail, "The saved hosts file cannot be read")
                        .details(vec![e.to_string()])
                        .fix(format!(
                            "Fix the line shown in {}, or move it away to start with no hosts",
                            ctx.show(store.path())
                        )),
                );
                None
            }
            Ok(inv) => {
                let n = inv.hosts.len();
                out.push(Check::new(
                    SETU,
                    Level::Ok,
                    format!(
                        "{n} saved host{} in {}",
                        if n == 1 { "" } else { "s" },
                        ctx.show(store.path())
                    ),
                ));
                host_checks(ctx, &inv, out);
                Some(inv)
            }
        }
    };
    if let Some(m) = mode(store.path()) {
        if m & 0o022 != 0 {
            out.push(
                Check::new(
                    SETU,
                    Level::Warn,
                    "Other users can change your saved hosts (they could point a host elsewhere)",
                )
                .fix(format!("chmod 600 {}", ctx.show(store.path()))),
            );
        }
    }

    let vault = dir.join(VAULT_FILE);
    if !vault.is_file() {
        out.push(Check::new(
            SETU,
            Level::Info,
            "No vault (optional): Setu asks for passwords and passphrases when you connect",
        ));
    } else {
        match Vault::check_file(&vault) {
            Ok(()) => out.push(Check::new(
                SETU,
                Level::Ok,
                format!(
                    "Vault {} is readable (doctor never unlocks it)",
                    ctx.show(&vault)
                ),
            )),
            Err(e) => out.push(
                Check::new(SETU, Level::Fail, "The vault file cannot be opened")
                    .details(vec![e.to_string()])
                    .fix(
                        "Restore vault.json from a backup; without one, the saved secrets are lost",
                    ),
            ),
        }
        if let Some(m) = mode(&vault) {
            if m & 0o077 != 0 {
                out.push(
                    Check::new(
                        SETU,
                        Level::Warn,
                        "Other users can read the vault file (it is encrypted, but should be private)",
                    )
                    .fix(format!("chmod 600 {}", ctx.show(&vault))),
                );
            }
        }
    }
    inventory
}

fn host_checks(ctx: &Context, inv: &Inventory, out: &mut Vec<Check>) {
    for host in &inv.hosts {
        if let Err(e) = host.clone().normalize() {
            out.push(
                Check::new(SETU, Level::Fail, format!("Host {}: {e}", host.name))
                    .fix("Edit the host in Setu or in hosts.toml"),
            );
            continue;
        }
        match (&host.auth, &host.key_file) {
            (Auth::Key, None) => out.push(
                Check::new(
                    SETU,
                    Level::Fail,
                    format!("Host {} uses a key file but names none", host.name),
                )
                .fix("Set key_file, or change auth to \"auto\""),
            ),
            (_, Some(key)) if !ctx.expand(key).is_file() => out.push(
                Check::new(
                    SETU,
                    Level::Fail,
                    format!("Host {}: key file {key} not found", host.name),
                )
                .fix("Fix the path, or create the key with ssh-keygen -t ed25519"),
            ),
            _ => {}
        }
    }
}

fn load_ssh_config(ctx: &Context) -> Option<SshConfig> {
    SshConfig::load(&ctx.ssh_dir().join("config"), &ctx.home).ok()
}

struct KeySummary {
    /// Whether any key needs a passphrase that the agent does not hold.
    encrypted: bool,
}

fn key_checks(
    ctx: &Context,
    inventory: Option<&Inventory>,
    config: Option<&SshConfig>,
    agent: &AgentState,
    out: &mut Vec<Check>,
) -> KeySummary {
    let ssh = ctx.ssh_dir();
    let mut summary = KeySummary { encrypted: false };
    if !ssh.is_dir() {
        out.push(
            Check::new(KEYS, Level::Info, format!("No {} folder", ctx.show(&ssh)))
                .fix("Create a key with ssh-keygen -t ed25519, or use password hosts"),
        );
        return summary;
    }
    if let Some(m) = mode(&ssh) {
        if m & 0o022 != 0 {
            out.push(
                Check::new(
                    KEYS,
                    Level::Warn,
                    format!("Other users can write to {}", ctx.show(&ssh)),
                )
                .fix(format!("chmod 700 {}", ctx.show(&ssh))),
            );
        }
    }

    // Keys in ~/.ssh, plus keys named by saved hosts and ~/.ssh/config.
    let mut paths: BTreeSet<PathBuf> = std::fs::read_dir(&ssh)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && keys::looks_like_private_key(p))
        .collect();
    let named = inventory
        .into_iter()
        .flat_map(|inv| inv.hosts.iter().filter_map(|h| h.key_file.clone()))
        .chain(config.into_iter().flat_map(|c| {
            c.aliases()
                .iter()
                .filter_map(|a| c.resolve(a).get("identityfile").map(str::to_string))
                .collect::<Vec<_>>()
        }));
    for key in named {
        let path = ctx.expand(&key);
        if path.is_file() {
            paths.insert(path);
        }
    }

    let in_agent: BTreeSet<&str> = match agent {
        AgentState::Keys(keys) => keys.iter().map(|k| k.fingerprint.as_str()).collect(),
        _ => BTreeSet::new(),
    };

    if paths.is_empty() {
        out.push(
            Check::new(KEYS, Level::Info, "No private keys found")
                .fix("Create one with ssh-keygen -t ed25519"),
        );
    }
    for path in &paths {
        let shown = ctx.show(path);
        if let Some(m) = mode(path) {
            if m & 0o077 != 0 {
                out.push(
                    Check::new(
                        KEYS,
                        Level::Fail,
                        format!("{shown}: other users can read it, and OpenSSH refuses such keys"),
                    )
                    .fix(format!("chmod 600 {shown}")),
                );
                continue;
            }
        }
        let loaded = keys::public_fingerprint(path)
            .map(|fp| in_agent.contains(fp.as_str()))
            .unwrap_or(false);
        let agent_note = if loaded { ", loaded in ssh-agent" } else { "" };
        let check = match keys::key_state(path) {
            KeyState::Plain => Check::new(
                KEYS,
                Level::Ok,
                format!("{shown}: usable, no passphrase{agent_note}"),
            ),
            KeyState::Encrypted => {
                summary.encrypted |= !loaded;
                Check::new(
                    KEYS,
                    Level::Ok,
                    format!("{shown}: protected by a passphrase{agent_note}"),
                )
            }
            KeyState::Unreadable(reason) => Check::new(
                KEYS,
                Level::Warn,
                format!("{shown}: Setu cannot read this key"),
            )
            .details(vec![reason])
            .fix("Convert it with ssh-keygen -p -f <key> (OpenSSH format), or load it into ssh-agent"),
        };
        out.push(check);
    }

    let known = ssh.join("known_hosts");
    match std::fs::read_to_string(&known) {
        Ok(text) => {
            let n = text
                .lines()
                .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
                .count();
            out.push(Check::new(
                KEYS,
                Level::Ok,
                format!(
                    "{}: {n} known host key{}",
                    ctx.show(&known),
                    if n == 1 { "" } else { "s" }
                ),
            ));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => out.push(Check::new(
            KEYS,
            Level::Info,
            "No known_hosts yet; host keys are added after you confirm their fingerprint",
        )),
        Err(e) => out.push(
            Check::new(
                KEYS,
                Level::Fail,
                format!("{} cannot be read", ctx.show(&known)),
            )
            .details(vec![e.to_string()])
            .fix("Fix its owner and permissions so your user can read and write it"),
        ),
    }
    summary
}

fn agent_check(agent: &AgentState, encrypted_keys: bool) -> Check {
    let start_fix = if cfg!(windows) {
        "Start the \"OpenSSH Authentication Agent\" service, then ssh-add your key"
    } else {
        "Run eval \"$(ssh-agent -s)\" and then ssh-add, or enable your desktop's key agent"
    };
    match agent {
        AgentState::NotConfigured => {
            let (level, title) = if encrypted_keys {
                (
                    Level::Warn,
                    "No ssh-agent: you will type key passphrases on every connection",
                )
            } else {
                (Level::Info, "No ssh-agent running (optional)")
            };
            Check::new(AGENT, level, title).fix(start_fix)
        }
        AgentState::Unreachable(reason) => Check::new(
            AGENT,
            Level::Fail,
            "An ssh-agent is configured but does not answer",
        )
        .details(vec![reason.clone()])
        .fix(if cfg!(windows) {
            "Restart the OpenSSH Authentication Agent service"
        } else {
            "Start a new agent, or unset SSH_AUTH_SOCK if it points to an old one"
        }),
        AgentState::Keys(keys) if keys.is_empty() => {
            Check::new(AGENT, Level::Warn, "ssh-agent is running but holds no keys")
                .fix("ssh-add ~/.ssh/id_ed25519 (or the key you use)")
        }
        AgentState::Keys(keys) => Check::new(
            AGENT,
            Level::Ok,
            format!(
                "ssh-agent holds {} key{}",
                keys.len(),
                if keys.len() == 1 { "" } else { "s" }
            ),
        )
        .details(
            keys.iter()
                .map(|k| {
                    let comment = if k.comment.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", k.comment)
                    };
                    format!("{} {}{comment}", k.algorithm, k.fingerprint)
                })
                .collect(),
        ),
    }
}

fn config_checks(
    ctx: &Context,
    config: Option<SshConfig>,
    inventory: Option<&Inventory>,
    out: &mut Vec<Check>,
) {
    let path = ctx.ssh_dir().join("config");
    let shown = ctx.show(&path);
    let Some(config) = config else {
        if path.exists() {
            out.push(
                Check::new(CONFIG, Level::Fail, format!("{shown} cannot be read"))
                    .fix("Fix its owner and permissions so your user can read it"),
            );
        } else {
            out.push(Check::new(
                CONFIG,
                Level::Info,
                format!("No {shown} (optional)"),
            ));
        }
        return;
    };
    if let Some(m) = mode(&path) {
        if m & 0o022 != 0 {
            out.push(
                Check::new(
                    CONFIG,
                    Level::Fail,
                    format!("Other users can change {shown}; OpenSSH refuses it (\"Bad owner or permissions\")"),
                )
                .fix(format!("chmod 600 {shown}")),
            );
        }
    }
    let n = config.aliases().len();
    out.push(Check::new(
        CONFIG,
        Level::Ok,
        format!("{shown}: {n} host{}", if n == 1 { "" } else { "s" }),
    ));
    for warning in &config.warnings {
        out.push(Check::new(CONFIG, Level::Warn, warning.clone()));
    }
    for alias in config.aliases() {
        if let Some(key) = config.resolve(alias).get("identityfile") {
            if !ctx.expand(key).is_file() {
                out.push(
                    Check::new(
                        CONFIG,
                        Level::Warn,
                        format!("Host {alias}: IdentityFile {key} not found"),
                    )
                    .fix(format!("Fix the IdentityFile line for {alias} in {shown}")),
                );
            }
        }
    }
    let empty = Inventory::default();
    let new = import::plan(&config, inventory.unwrap_or(&empty))
        .iter()
        .filter(|c| c.status == import::Status::New)
        .count();
    if new > 0 {
        out.push(
            Check::new(
                CONFIG,
                Level::Info,
                format!(
                    "{new} host{} not saved in Setu yet",
                    if new == 1 { " is" } else { "s are" }
                ),
            )
            .fix("setu hosts import"),
        );
    }
}

/// Counts per level, for the summary line and the exit code.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Totals {
    pub ok: usize,
    pub info: usize,
    pub warn: usize,
    pub fail: usize,
}

pub fn totals(checks: &[Check]) -> Totals {
    let mut t = Totals::default();
    for c in checks {
        match c.level {
            Level::Ok => t.ok += 1,
            Level::Info => t.info += 1,
            Level::Warn => t.warn += 1,
            Level::Fail => t.fail += 1,
        }
    }
    t
}
