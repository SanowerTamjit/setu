//! SSH tabs: connect, handle host-key trust, and open a remote shell.

use crate::terminals::{output_sink, Session, Terminals};
use serde::{Deserialize, Serialize};
use setu_ssh::{known_hosts, Auth, ConnectOptions, HostKeyInfo, ShellSize, SshError, Zeroizing};
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
        passphrase: Option<String>,
    },
    Password {
        password: String,
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

fn auth_methods(auth: AuthRequest) -> Vec<Auth> {
    match auth {
        AuthRequest::Default => Vec::new(),
        AuthRequest::Agent => vec![Auth::Agent],
        AuthRequest::KeyFile { path, passphrase } => vec![Auth::KeyFile {
            path: expand_home(&path),
            passphrase: passphrase.filter(|p| !p.is_empty()).map(Zeroizing::new),
        }],
        AuthRequest::Password { password } => vec![Auth::Password(Zeroizing::new(password))],
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

fn build_options(req: ConnectRequest) -> Result<ConnectOptions, ConnectFailure> {
    let known = known_hosts_path()?;
    let auth = auth_methods(req.auth);
    let user = user_or_local(&req.user);
    let mut opts = ConnectOptions::new(req.host.trim(), user.as_str())
        .port(req.port.unwrap_or(22))
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
    Ok(opts)
}

#[derive(Debug, Serialize)]
pub struct OpenedSsh {
    id: u32,
    title: String,
}

#[tauri::command]
pub async fn ssh_connect(
    state: State<'_, Terminals>,
    request: ConnectRequest,
    cols: u16,
    rows: u16,
    on_output: Channel<InvokeResponseBody>,
    on_exit: Channel<u32>,
) -> Result<OpenedSsh, ConnectFailure> {
    let title = format!("{}@{}", user_or_local(&request.user), request.host.trim());
    let opts = build_options(request)?;
    let client = Arc::new(setu_ssh::connect(&opts).await?);

    let id = state.next_id();
    let sessions = Arc::clone(&state.sessions);
    let shell = client
        .open_shell(
            ShellSize { cols, rows },
            output_sink(on_output),
            move |code| {
                // 255 is what OpenSSH reports when the connection drops without a status.
                let _ = on_exit.send(code.unwrap_or(255));
                if let Ok(mut map) = sessions.lock() {
                    map.remove(&id);
                }
            },
        )
        .await
        .map_err(ConnectFailure::from)?;
    state
        .insert(
            id,
            Session::Ssh {
                shell,
                _client: client,
            },
        )
        .map_err(|message| ConnectFailure::Error { message })?;
    Ok(OpenedSsh { id, title })
}

/// Records a host key the user explicitly accepted in the trust prompt.
#[tauri::command]
pub fn ssh_trust_host(key: HostKeyInfo) -> Result<(), String> {
    let path = known_hosts::default_path().ok_or("could not find known_hosts")?;
    known_hosts::trust(&path, &key).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let opts = build_options(req).expect("options");
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
}
