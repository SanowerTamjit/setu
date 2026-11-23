//! SSH tabs: connect, handle host-key trust, and open a remote shell.

use crate::terminals::{output_sink, Session, Terminals};
use crate::vault::{Availability, VaultState};
use serde::{Deserialize, Serialize};
use setu_ssh::{known_hosts, Auth, ConnectOptions, HostKeyInfo, ShellSize, SshError, Zeroizing};
use setu_vault::SecretKind;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::State;

/// How the user chose to authenticate in the connect form.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AuthRequest {
    /// ssh-agent, then the default keys in ~/.ssh (what OpenSSH does).
    Default,
    Agent,
    #[serde(rename_all = "camelCase")]
    KeyFile {
        path: String,
        /// Empty means "use the vault's, if it has one".
        passphrase: Option<String>,
        /// Save the passphrase in the vault once the connection works.
        #[serde(default)]
        save: bool,
    },
    Password {
        /// Empty means "use the vault's, if it has one".
        password: String,
        #[serde(default)]
        save: bool,
    },
}

impl std::fmt::Debug for AuthRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthRequest::Default => write!(f, "Default"),
            AuthRequest::Agent => write!(f, "Agent"),
            AuthRequest::KeyFile { path, .. } => write!(f, "KeyFile({path})"),
            AuthRequest::Password { .. } => write!(f, "Password(<redacted>)"),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectRequest {
    host: String,
    port: Option<u16>,
    user: String,
    auth: AuthRequest,
    /// Optional ProxyJump host as `user@host:port` (user and port optional).
    jump: Option<String>,
}

/// Why a connection did not open, in a shape the UI can act on.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ConnectFailure {
    /// Ask the user to verify and trust the key, then retry.
    UnknownHostKey {
        key: Box<HostKeyInfo>,
    },
    /// Possible man-in-the-middle: show a warning and do not offer to continue.
    #[serde(rename_all = "camelCase")]
    HostKeyChanged {
        key: Box<HostKeyInfo>,
        known_hosts: String,
        line: usize,
    },
    /// Ask for the key's passphrase, then retry.
    NeedsPassphrase {
        path: String,
        vault: Availability,
    },
    /// Ask for the password (none was typed and the vault has none), then retry.
    NeedsPassword {
        vault: Availability,
    },
    Error {
        message: String,
    },
}

impl From<SshError> for ConnectFailure {
    fn from(e: SshError) -> Self {
        match e {
            SshError::UnknownHostKey { key } => ConnectFailure::UnknownHostKey { key },
            SshError::HostKeyChanged {
                key,
                known_hosts,
                line,
            } => ConnectFailure::HostKeyChanged {
                key,
                known_hosts: known_hosts.display().to_string(),
                line,
            },
            SshError::KeyNeedsPassphrase { path } => ConnectFailure::NeedsPassphrase {
                path: path.display().to_string(),
                vault: Availability::None,
            },
            other => ConnectFailure::Error {
                message: other.to_string(),
            },
        }
    }
}

fn known_hosts_path() -> Result<PathBuf, ConnectFailure> {
    known_hosts::default_path().ok_or_else(|| ConnectFailure::Error {
        message: "could not find your home directory for known_hosts".into(),
    })
}

/// Expands a leading `~/` so users can type `~/.ssh/id_ed25519`.
fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
        if let Some(home) = home {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

/// A secret the user typed and asked to keep, saved once the connection works.
type ToSave = Option<(SecretKind, String, Zeroizing<String>)>;

/// Turns the form's choice into auth methods, filling empty secrets from the
/// vault. `password_target` is the vault key for a login password.
fn auth_methods(
    auth: AuthRequest,
    password_target: &str,
    vault: &VaultState,
) -> Result<(Vec<Auth>, ToSave), ConnectFailure> {
    Ok(match auth {
        AuthRequest::Default => (Vec::new(), None),
        AuthRequest::Agent => (vec![Auth::Agent], None),
        AuthRequest::KeyFile {
            path,
            passphrase,
            save,
        } => {
            let path = expand_home(&path);
            let target = path.display().to_string();
            let typed = passphrase.filter(|p| !p.is_empty()).map(Zeroizing::new);
            let to_save = match (&typed, save) {
                (Some(p), true) => Some((SecretKind::KeyPassphrase, target.clone(), p.clone())),
                _ => None,
            };
            let passphrase = typed.or_else(|| vault.get(SecretKind::KeyPassphrase, &target));
            (vec![Auth::KeyFile { path, passphrase }], to_save)
        }
        AuthRequest::Password { password, save } => {
            if password.is_empty() {
                let stored = vault.get(SecretKind::SshPassword, password_target).ok_or(
                    ConnectFailure::NeedsPassword {
                        vault: vault.availability(),
                    },
                )?;
                (vec![Auth::Password(stored)], None)
            } else {
                let password = Zeroizing::new(password);
                let to_save = save.then(|| {
                    (
                        SecretKind::SshPassword,
                        password_target.to_string(),
                        password.clone(),
                    )
                });
                (vec![Auth::Password(password)], to_save)
            }
        }
    })
}

/// The vault key for a login password.
fn password_target(user: &str, host: &str, port: u16) -> String {
    format!("{user}@{}:{port}", host.to_ascii_lowercase())
}

/// Puts a passphrase from the vault on every use of `key` in the chain, so a
/// retry can unlock the key that asked for it.
fn with_passphrase(
    opts: &mut ConnectOptions,
    key: &std::path::Path,
    passphrase: &Zeroizing<String>,
) {
    if opts.auth.is_empty() {
        if let Some(dir) = key.parent() {
            opts.auth = Auth::defaults(dir);
        }
    }
    for auth in &mut opts.auth {
        if let Auth::KeyFile {
            path,
            passphrase: p @ None,
        } = auth
        {
            if path == key {
                *p = Some(passphrase.clone());
            }
        }
    }
    for hop in &mut opts.jump {
        with_passphrase(hop, key, passphrase);
    }
}

/// Parses `[user@]host[:port]`, defaulting the user to the target's.
fn parse_jump(spec: &str, default_user: &str) -> Result<(String, String, u16), String> {
    let spec = spec.trim();
    let (user, rest) = match spec.rsplit_once('@') {
        Some((u, r)) => (u.to_string(), r),
        None => (default_user.to_string(), spec),
    };
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>()
                .map_err(|_| format!("invalid jump host port `{p}`"))?,
        ),
        None => (rest.to_string(), 22),
    };
    if host.is_empty() || user.is_empty() {
        return Err(format!("invalid jump host `{spec}`; use user@host:port"));
    }
    Ok((user, host, port))
}

/// The local login name, which OpenSSH also uses when no user is given.
pub fn local_user() -> String {
    ["USER", "USERNAME", "LOGNAME"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
        .unwrap_or_else(|| "root".into())
}

fn user_or_local(user: &str) -> String {
    match user.trim() {
        "" => local_user(),
        u => u.to_string(),
    }
}

fn build_options(
    req: ConnectRequest,
    vault: &VaultState,
) -> Result<(ConnectOptions, ToSave), ConnectFailure> {
    let known = known_hosts_path()?;
    let user = user_or_local(&req.user);
    let port = req.port.unwrap_or(22);
    let target = password_target(&user, req.host.trim(), port);
    let (auth, to_save) = auth_methods(req.auth, &target, vault)?;
    let mut opts = ConnectOptions::new(req.host.trim(), user.as_str())
        .port(port)
        .known_hosts(&known);
    opts.auth = auth.clone();
    // ProxyJump takes a comma-separated chain, connected in order.
    let hops = req.jump.as_deref().unwrap_or("");
    for spec in hops.split(',').filter(|j| !j.trim().is_empty()) {
        let (hop_user, host, port) =
            parse_jump(spec, &user).map_err(|message| ConnectFailure::Error { message })?;
        let mut hop = ConnectOptions::new(host, hop_user)
            .port(port)
            .known_hosts(&known);
        // Jump hosts usually take the same credentials (agent or key).
        hop.auth = auth.clone();
        opts = opts.jump(hop);
    }
    Ok((opts, to_save))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedSsh {
    id: u32,
    title: String,
    /// The typed password or passphrase was saved in the vault.
    saved_to_vault: bool,
}

// A Tauri command's arguments are what the UI sends; grouping them would only move the list.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn ssh_connect(
    window: tauri::Window,
    state: State<'_, Terminals>,
    vault: State<'_, VaultState>,
    offered: State<'_, OfferedKeys>,
    request: ConnectRequest,
    cols: u16,
    rows: u16,
    on_output: Channel<InvokeResponseBody>,
    on_exit: Channel<u32>,
) -> Result<OpenedSsh, ConnectFailure> {
    let result = connect_and_open(
        window, &state, &vault, request, cols, rows, on_output, on_exit,
    )
    .await;
    if let Err(ConnectFailure::UnknownHostKey { key }) = &result {
        offered.offer(key);
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn connect_and_open(
    window: tauri::Window,
    state: &Terminals,
    vault: &VaultState,
    request: ConnectRequest,
    cols: u16,
    rows: u16,
    on_output: Channel<InvokeResponseBody>,
    on_exit: Channel<u32>,
) -> Result<OpenedSsh, ConnectFailure> {
    let title = format!("{}@{}", user_or_local(&request.user), request.host.trim());
    let (opts, to_save) = build_options(request, vault)?;
    let needs_passphrase = |path: PathBuf| ConnectFailure::NeedsPassphrase {
        path: path.display().to_string(),
        vault: vault.availability(),
    };
    let client = match setu_ssh::connect(&opts).await {
        Ok(client) => client,
        // An encrypted key with no passphrase given: try the vault's once.
        Err(SshError::KeyNeedsPassphrase { path }) => {
            let Some(passphrase) =
                vault.get(SecretKind::KeyPassphrase, &path.display().to_string())
            else {
                return Err(needs_passphrase(path));
            };
            let mut retry = opts.clone();
            with_passphrase(&mut retry, &path, &passphrase);
            match setu_ssh::connect(&retry).await {
                Ok(client) => client,
                Err(SshError::KeyNeedsPassphrase { path }) => return Err(needs_passphrase(path)),
                Err(e) => return Err(e.into()),
            }
        }
        Err(e) => return Err(e.into()),
    };
    let id = open_shell(
        state,
        window.label(),
        Arc::new(client),
        cols,
        rows,
        on_output,
        on_exit,
    )
    .await
    .map_err(|e| match e {
        ShellError::Ssh(e) => ConnectFailure::from(e),
        ShellError::State(message) => ConnectFailure::Error { message },
    })?;
    // Saving is best effort: the connection already works.
    let saved_to_vault = match to_save {
        Some((kind, target, secret)) => vault.save(kind, &target, secret).unwrap_or(false),
        None => false,
    };
    Ok(OpenedSsh {
        id,
        title,
        saved_to_vault,
    })
}

enum ShellError {
    Ssh(SshError),
    State(String),
}

/// Opens an interactive shell on `client` and registers it as terminal session.
async fn open_shell(
    state: &Terminals,
    window: &str,
    client: Arc<setu_ssh::SshClient>,
    cols: u16,
    rows: u16,
    on_output: Channel<InvokeResponseBody>,
    on_exit: Channel<u32>,
) -> Result<u32, ShellError> {
    let id = state.next_id();
    let pipes = state.pipes(id, on_output, on_exit, window);
    let on_exit = state.exit_handler(id, Arc::clone(&pipes));
    let shell = client
        .open_shell(
            ShellSize { cols, rows },
            output_sink(pipes),
            // 255 is what OpenSSH reports when the connection drops without a status.
            move |code| on_exit(code.unwrap_or(255)),
        )
        .await
        .map_err(ShellError::Ssh)?;
    state
        .insert(id, Session::Ssh { shell, client })
        .map_err(ShellError::State)?;
    Ok(id)
}

/// Opens another shell on the connection of SSH terminal `terminal_id`, for a
/// split pane: no new login, and it shares the connection's jump hosts.
#[tauri::command]
pub async fn ssh_open_shell(
    window: tauri::Window,
    state: State<'_, Terminals>,
    terminal_id: u32,
    cols: u16,
    rows: u16,
    on_output: Channel<InvokeResponseBody>,
    on_exit: Channel<u32>,
) -> Result<u32, String> {
    let client = state
        .ssh_client(terminal_id)
        .ok_or("that pane is not an open SSH connection")?;
    open_shell(
        &state,
        window.label(),
        client,
        cols,
        rows,
        on_output,
        on_exit,
    )
    .await
    .map_err(|e| match e {
        ShellError::Ssh(e) => e.to_string(),
        ShellError::State(message) => message,
    })
}

/// Host keys servers presented that are not in known_hosts yet, waiting for
/// the person to accept or refuse them in the trust prompt. Only these can be
/// trusted: the web view cannot write a key of its own choosing.
#[derive(Debug, Default)]
pub struct OfferedKeys(std::sync::Mutex<Vec<HostKeyInfo>>);

/// Enough for a jump chain and a few tabs connecting at once.
const MAX_OFFERED: usize = 16;

impl OfferedKeys {
    fn offer(&self, key: &HostKeyInfo) {
        let mut list = self.0.lock().unwrap_or_else(|p| p.into_inner());
        list.retain(|k| k != key);
        if list.len() >= MAX_OFFERED {
            list.remove(0);
        }
        list.push(key.clone());
    }

    /// Removes and returns `key` if a server offered exactly it.
    fn take(&self, key: &HostKeyInfo) -> Option<HostKeyInfo> {
        let mut list = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let i = list.iter().position(|k| k == key)?;
        Some(list.remove(i))
    }
}

/// Records a host key the user explicitly accepted in the trust prompt.
#[tauri::command]
pub fn ssh_trust_host(offered: State<'_, OfferedKeys>, key: HostKeyInfo) -> Result<(), String> {
    let key = offered
        .take(&key)
        .ok_or("that host key was not offered by a server; connect again to see it")?;
    let path = known_hosts::default_path().ok_or("could not find known_hosts")?;
    known_hosts::trust(&path, &key).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_offered_keys_can_be_trusted() {
        let key: HostKeyInfo = serde_json::from_str(
            r#"{"host":"h","port":22,"algorithm":"ssh-ed25519","fingerprint":"SHA256:x","openssh":"ssh-ed25519 AAAA"}"#,
        )
        .expect("key");
        let offered = OfferedKeys::default();
        assert!(offered.take(&key).is_none());
        offered.offer(&key);
        let mut other = key.clone();
        other.host = "*".into();
        assert!(offered.take(&other).is_none());
        assert_eq!(offered.take(&key), Some(key.clone()));
        // Each offer is good for one trust.
        assert!(offered.take(&key).is_none());
        for port in 0..40 {
            let mut k = key.clone();
            k.port = port;
            offered.offer(&k);
        }
        assert!(offered.0.lock().map(|l| l.len()).unwrap_or(0) <= MAX_OFFERED);
    }

    #[test]
    fn jump_spec_parsing() {
        assert_eq!(
            parse_jump("ops@bastion.example:2222", "me"),
            Ok(("ops".into(), "bastion.example".into(), 2222))
        );
        assert_eq!(
            parse_jump("bastion", "me"),
            Ok(("me".into(), "bastion".into(), 22))
        );
        assert!(parse_jump("bastion:notaport", "me").is_err());
        assert!(parse_jump("@", "me").is_err());
    }

    #[test]
    fn jump_chains_and_default_user() {
        let req: ConnectRequest = serde_json::from_str(
            r#"{"host":"db","user":"","auth":{"kind":"agent"},"jump":"ops@a:2200, b"}"#,
        )
        .expect("parse");
        let (opts, _) = build_options(req, &VaultState::from_env()).expect("options");
        assert_eq!(opts.user, local_user());
        let hops: Vec<_> = opts
            .jump
            .iter()
            .map(|h| (h.host.as_str(), h.port))
            .collect();
        assert_eq!(hops, [("a", 2200), ("b", 22)]);
        assert_eq!(opts.jump[0].user, "ops");
        assert_eq!(opts.jump[1].user, local_user());
    }

    #[test]
    fn auth_request_debug_hides_secrets() {
        let req = AuthRequest::Password {
            password: "hunter2".into(),
            save: true,
        };
        assert!(!format!("{req:?}").contains("hunter2"));
    }

    #[test]
    fn auth_request_deserializes_from_the_ui_shape() {
        let req: AuthRequest = serde_json::from_str(
            r#"{"kind":"keyFile","path":"~/.ssh/id_ed25519","passphrase":null}"#,
        )
        .expect("parse");
        assert!(matches!(req, AuthRequest::KeyFile { .. }));
        let req: AuthRequest = serde_json::from_str(r#"{"kind":"default"}"#).expect("parse");
        assert!(matches!(req, AuthRequest::Default));
    }

    #[test]
    fn empty_password_without_vault_asks_for_one() {
        let vault = VaultState::from_env();
        let err = auth_methods(
            AuthRequest::Password {
                password: String::new(),
                save: false,
            },
            "me@host:22",
            &vault,
        )
        .expect_err("needs a password");
        assert!(matches!(err, ConnectFailure::NeedsPassword { .. }));
    }

    #[test]
    fn typed_secrets_are_saved_only_when_asked() {
        let vault = VaultState::from_env();
        let (_, to_save) = auth_methods(
            AuthRequest::Password {
                password: "pw".into(),
                save: true,
            },
            "me@host:22",
            &vault,
        )
        .expect("auth");
        let (kind, target, _) = to_save.expect("to save");
        assert_eq!(kind, SecretKind::SshPassword);
        assert_eq!(target, "me@host:22");
        let (_, to_save) = auth_methods(
            AuthRequest::Password {
                password: "pw".into(),
                save: false,
            },
            "me@host:22",
            &vault,
        )
        .expect("auth");
        assert!(to_save.is_none());
        assert_eq!(
            password_target("Me", "Web.Example", 2222),
            "Me@web.example:2222"
        );
    }

    #[test]
    fn vault_passphrase_reaches_every_hop() {
        let key = PathBuf::from("/home/me/.ssh/id_ed25519");
        let mut opts = ConnectOptions::new("db", "me");
        opts.auth = vec![Auth::KeyFile {
            path: key.clone(),
            passphrase: None,
        }];
        let mut hop = ConnectOptions::new("bastion", "me");
        hop.auth = opts.auth.clone();
        opts = opts.jump(hop);
        with_passphrase(&mut opts, &key, &Zeroizing::new("pp".into()));
        for o in [&opts, &opts.jump[0]] {
            assert!(matches!(
                &o.auth[0],
                Auth::KeyFile {
                    passphrase: Some(_),
                    ..
                }
            ));
        }
    }
}
