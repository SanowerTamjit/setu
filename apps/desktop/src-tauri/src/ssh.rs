//! SSH tabs: connect, handle host-key trust, and open a remote shell.

use crate::terminals::{recorded_sink, Session, Terminals};
use crate::vault::{Availability, VaultState};
use serde::{Deserialize, Serialize};
use setu_core::extend::{
    AuditAction, AuditEvent, AuditOutcome, ConnectCheck, Extensions, PolicyDecision, SessionStart,
    RECORDING_NOTICE,
};
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
pub(crate) type ToSave = Option<(SecretKind, String, Zeroizing<String>)>;

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

pub(crate) fn user_or_local(user: &str) -> String {
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
    extensions: State<'_, Extensions>,
    hosts: State<'_, crate::hosts::Hosts>,
    request: ConnectRequest,
    cols: u16,
    rows: u16,
    on_output: Channel<InvokeResponseBody>,
    on_exit: Channel<u32>,
) -> Result<OpenedSsh, ConnectFailure> {
    // A refused login is audited by `check_connect` itself.
    let saved = hosts.inventory().map(|i| i.hosts).unwrap_or_default();
    let login = request.login_for(&saved);
    check_access(&extensions, login.clone()).await?;
    let target = request.audit_target();
    let result = connect_and_open(
        window,
        &state,
        &vault,
        &extensions,
        login,
        request,
        cols,
        rows,
        on_output,
        on_exit,
    )
    .await;
    extensions.audit(connect_event(target, &result));
    if let Err(ConnectFailure::UnknownHostKey { key }) = &result {
        offered.offer(key);
    }
    result
}

/// A login to check against the build's access policy, owned so it can
/// move to a blocking thread (an Enterprise build may ask its server).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Login {
    pub name: String,
    pub address: String,
    pub port: u16,
    pub login: String,
    pub tags: Vec<String>,
}

impl Login {
    pub(crate) fn for_host(host: &setu_hosts::Host) -> Self {
        Login {
            name: host.name.clone(),
            address: host.host.trim().to_string(),
            port: host.port,
            login: user_or_local(host.user.as_deref().unwrap_or_default()),
            tags: host.tags.clone(),
        }
    }
}

/// Asks the access policy before connecting. The open-source build allows
/// every login; a refusal becomes an error the user can read.
pub(crate) async fn check_access(
    extensions: &Extensions,
    login: Login,
) -> Result<(), ConnectFailure> {
    let extensions = extensions.clone();
    let decision = tauri::async_runtime::spawn_blocking(move || {
        extensions.check_connect(&ConnectCheck {
            name: &login.name,
            address: &login.address,
            port: login.port,
            login: &login.login,
            tags: &login.tags,
        })
    })
    .await
    .map_err(|_| ConnectFailure::Error {
        message: "the access check stopped unexpectedly".into(),
    })?;
    refusal(decision).map_or(Ok(()), Err)
}

fn refusal(decision: PolicyDecision) -> Option<ConnectFailure> {
    match decision {
        PolicyDecision::Allow => None,
        PolicyDecision::Deny { reason } => Some(ConnectFailure::Error {
            message: format!("Your organization does not allow this login: {reason}"),
        }),
    }
}

/// The audit record of a login: where, and why it failed (never a secret:
/// [`ConnectFailure`] holds no password or passphrase).
fn connect_event(target: String, result: &Result<OpenedSsh, ConnectFailure>) -> AuditEvent {
    let (outcome, detail) = match result {
        Ok(_) => (AuditOutcome::Succeeded, String::new()),
        Err(e) => (AuditOutcome::Failed, e.audit_reason().to_string()),
    };
    AuditEvent {
        action: AuditAction::SshConnect,
        outcome,
        target,
        detail,
    }
}

#[allow(clippy::too_many_arguments)]
async fn connect_and_open(
    window: tauri::Window,
    state: &Terminals,
    vault: &VaultState,
    extensions: &Extensions,
    login: Login,
    request: ConnectRequest,
    cols: u16,
    rows: u16,
    on_output: Channel<InvokeResponseBody>,
    on_exit: Channel<u32>,
) -> Result<OpenedSsh, ConnectFailure> {
    let title = format!("{}@{}", user_or_local(&request.user), request.host.trim());
    let (client, to_save) = connect_client(request, vault).await?;
    let id = open_shell(
        state,
        window.label(),
        Arc::new(client),
        extensions,
        Arc::new(login),
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

/// Connects and logs in, trying the vault's passphrase once when a key asks
/// for one. Returns the secret to save once the caller knows the login worked.
pub(crate) async fn connect_client(
    request: ConnectRequest,
    vault: &VaultState,
) -> Result<(setu_ssh::SshClient, ToSave), ConnectFailure> {
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
    Ok((client, to_save))
}

impl ConnectRequest {
    /// The login this request makes, named and tagged as the saved host
    /// with the same address and port when there is one.
    fn login_for(&self, saved: &[setu_hosts::Host]) -> Login {
        let address = self.host.trim();
        let port = self.port.unwrap_or(22);
        let host = saved
            .iter()
            .find(|h| h.host.trim().eq_ignore_ascii_case(address) && h.port == port);
        Login {
            name: host.map_or_else(|| address.to_string(), |h| h.name.clone()),
            address: address.to_string(),
            port,
            login: user_or_local(&self.user),
            tags: host.map(|h| h.tags.clone()).unwrap_or_default(),
        }
    }

    /// `user@host:port` for an audit trail.
    fn audit_target(&self) -> String {
        format!(
            "{}@{}:{}",
            user_or_local(&self.user),
            self.host.trim(),
            self.port.unwrap_or(22)
        )
    }

    /// How a saved host connects, for runs nobody types into: secrets come
    /// from the vault only, nothing is saved.
    pub(crate) fn for_host(host: &setu_hosts::Host) -> Self {
        use setu_hosts::Auth as HostAuth;
        let auth = match host.auth {
            HostAuth::Auto => AuthRequest::Default,
            HostAuth::Agent => AuthRequest::Agent,
            HostAuth::Key => AuthRequest::KeyFile {
                path: host
                    .key_file
                    .clone()
                    .unwrap_or_else(|| "~/.ssh/id_ed25519".into()),
                passphrase: None,
                save: false,
            },
            HostAuth::Password => AuthRequest::Password {
                password: String::new(),
                save: false,
            },
        };
        ConnectRequest {
            host: host.host.clone(),
            port: Some(host.port),
            user: host.user.clone().unwrap_or_default(),
            auth,
            jump: host.jump.clone(),
        }
    }
}

impl ConnectFailure {
    /// A short, fixed reason for an audit trail.
    fn audit_reason(&self) -> &'static str {
        match self {
            ConnectFailure::UnknownHostKey { .. } => "unknown host key",
            ConnectFailure::HostKeyChanged { .. } => "host key changed",
            ConnectFailure::NeedsPassphrase { .. } => "key needs a passphrase",
            ConnectFailure::NeedsPassword { .. } => "needs a password",
            ConnectFailure::Error { .. } => "connection or login failed",
        }
    }

    /// One sentence for a run log, where nobody can answer a prompt.
    pub(crate) fn unattended_message(&self) -> String {
        match self {
            ConnectFailure::UnknownHostKey { .. } => "Setu does not know this server's host key yet. Connect to it once from Hosts to check and trust the key, then try again.".into(),
            ConnectFailure::HostKeyChanged { .. } => "the server's host key has CHANGED. Setu will not connect; connect from Hosts to see the warning.".into(),
            ConnectFailure::NeedsPassphrase { path, .. } => format!("the key {path} needs a passphrase. Save it in the vault (connect once and tick Save) or load the key into ssh-agent."),
            ConnectFailure::NeedsPassword { .. } => "this host logs in with a password and the vault has none for it (or is locked). Unlock the vault, or save the password when you connect.".into(),
            ConnectFailure::Error { message } => message.clone(),
        }
    }
}

enum ShellError {
    Ssh(SshError),
    State(String),
}

/// Asks the build whether to record a shell that is opening.
fn start_recording(
    extensions: &Extensions,
    login: &Login,
    cols: u16,
    rows: u16,
) -> Option<setu_core::extend::Recording> {
    extensions.start_recording(&SessionStart {
        name: &login.name,
        address: &login.address,
        port: login.port,
        login: &login.login,
        tags: &login.tags,
        cols,
        rows,
    })
}

/// Opens an interactive shell on `client` and registers it as terminal
/// session. When the build records it, the terminal says so first.
#[allow(clippy::too_many_arguments)]
async fn open_shell(
    state: &Terminals,
    window: &str,
    client: Arc<setu_ssh::SshClient>,
    extensions: &Extensions,
    login: Arc<Login>,
    cols: u16,
    rows: u16,
    on_output: Channel<InvokeResponseBody>,
    on_exit: Channel<u32>,
) -> Result<u32, ShellError> {
    let id = state.next_id();
    let pipes = state.pipes(id, on_output, on_exit, window);
    let recording = start_recording(extensions, &login, cols, rows);
    if recording.is_some() {
        pipes.notice(RECORDING_NOTICE);
    }
    let on_exit = state.exit_handler(id, Arc::clone(&pipes));
    let ends = recording.clone();
    let shell = client
        .open_shell(
            ShellSize { cols, rows },
            recorded_sink(pipes, recording.clone()),
            // 255 is what OpenSSH reports when the connection drops without a status.
            move |code| {
                if let Some(r) = &ends {
                    r.finish(code);
                }
                on_exit(code.unwrap_or(255))
            },
        )
        .await;
    let shell = match shell {
        Ok(s) => s,
        Err(e) => {
            if let Some(r) = &recording {
                r.finish(None);
            }
            return Err(ShellError::Ssh(e));
        }
    };
    state
        .insert(
            id,
            Session::Ssh {
                shell,
                client,
                login,
                recording,
            },
        )
        .map_err(ShellError::State)?;
    Ok(id)
}

/// Opens another shell on the connection of SSH terminal `terminal_id`, for a
/// split pane: no new login, and it shares the connection's jump hosts.
// A Tauri command's arguments are what the UI sends; grouping them would only move the list.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn ssh_open_shell(
    window: tauri::Window,
    state: State<'_, Terminals>,
    extensions: State<'_, Extensions>,
    terminal_id: u32,
    cols: u16,
    rows: u16,
    on_output: Channel<InvokeResponseBody>,
    on_exit: Channel<u32>,
) -> Result<u32, String> {
    let client = state
        .ssh_client(terminal_id)
        .ok_or("that pane is not an open SSH connection")?;
    let login = state
        .ssh_login(terminal_id)
        .ok_or("that pane is not an open SSH connection")?;
    open_shell(
        &state,
        window.label(),
        client,
        &extensions,
        login,
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

    #[derive(Debug, Default)]
    struct Recorder(std::sync::Mutex<Vec<AuditEvent>>);

    impl setu_core::extend::AuditSink for Recorder {
        fn record(&self, event: &AuditEvent) {
            if let Ok(mut events) = self.0.lock() {
                events.push(event.clone());
            }
        }
    }

    #[derive(Debug)]
    struct OnlyDeploy;

    impl setu_core::extend::AccessPolicy for OnlyDeploy {
        fn check_connect(&self, c: &ConnectCheck<'_>) -> PolicyDecision {
            if c.login == "deploy" && c.tags.iter().any(|t| t == "web") {
                PolicyDecision::Allow
            } else {
                PolicyDecision::Deny {
                    reason: "no access rule covers this host".into(),
                }
            }
        }
    }

    fn request(host: &str, port: Option<u16>, user: &str) -> ConnectRequest {
        ConnectRequest {
            host: host.into(),
            port,
            user: user.into(),
            auth: AuthRequest::Agent,
            jump: None,
        }
    }

    #[test]
    fn a_typed_login_takes_the_saved_hosts_name_and_tags() {
        let mut web = setu_hosts::Host::new("web1", "10.0.0.5");
        web.tags = vec!["web".into()];
        let saved = vec![web];
        let login = request(" 10.0.0.5 ", None, "deploy").login_for(&saved);
        assert_eq!(login.name, "web1");
        assert_eq!(login.tags, vec!["web".to_string()]);
        assert_eq!((login.address.as_str(), login.port), ("10.0.0.5", 22));
        // Another port is another host: not saved, so no tags.
        let other = request("10.0.0.5", Some(2222), "deploy").login_for(&saved);
        assert_eq!(other.name, "10.0.0.5");
        assert!(other.tags.is_empty());
    }

    #[test]
    fn a_refused_login_does_not_connect_and_says_why() {
        let rec = Arc::new(Recorder::default());
        let ext = Extensions::default()
            .with_audit(rec.clone())
            .with_access(Arc::new(OnlyDeploy));
        let mut web = setu_hosts::Host::new("web1", "10.0.0.5");
        web.tags = vec!["web".into()];
        web.user = Some("deploy".into());
        let ok = tauri::async_runtime::block_on(check_access(&ext, Login::for_host(&web)));
        assert!(ok.is_ok());
        web.user = Some("root".into());
        let refused = tauri::async_runtime::block_on(check_access(&ext, Login::for_host(&web)));
        let Err(ConnectFailure::Error { message }) = refused else {
            panic!("expected a refusal");
        };
        assert!(message.contains("does not allow this login"), "{message}");
        assert!(
            message.contains("no access rule covers this host"),
            "{message}"
        );
        let events = rec.0.lock().expect("lock").clone();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].outcome, AuditOutcome::Blocked);
        assert_eq!(events[0].target, "root@10.0.0.5:22");
        // The open-source build lets it through.
        let open = tauri::async_runtime::block_on(check_access(
            &Extensions::default(),
            Login::for_host(&web),
        ));
        assert!(open.is_ok());
    }

    #[test]
    fn login_audit_has_where_and_why_but_no_secret() {
        let request = ConnectRequest {
            host: " db1.example ".into(),
            port: Some(2222),
            user: "ops".into(),
            auth: AuthRequest::Password {
                password: "hunter2".into(),
                save: false,
            },
            jump: None,
        };
        let target = request.audit_target();
        assert_eq!(target, "ops@db1.example:2222");
        let failed: Result<OpenedSsh, ConnectFailure> = Err(ConnectFailure::Error {
            message: "Permission denied for hunter2".into(),
        });
        let e = connect_event(target, &failed);
        assert_eq!(e.action, AuditAction::SshConnect);
        assert_eq!(e.outcome, AuditOutcome::Failed);
        assert_eq!(e.detail, "connection or login failed");
        assert!(!format!("{e:?}").contains("hunter2"));
    }

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
