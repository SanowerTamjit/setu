//! Connecting and authenticating.

use crate::known_hosts::{self, HostKeyInfo, Verdict};
use crate::shell::{ShellSize, SshShell};
use crate::SshError;
use russh::client::{self, Handle};
use russh::keys::{PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zeroize::Zeroizing;

/// One way to prove who you are to the server.
#[derive(Clone)]
pub enum Auth {
    /// Keys held by the running ssh-agent (`SSH_AUTH_SOCK` on Unix, the
    /// OpenSSH agent pipe or Pageant on Windows).
    Agent,
    /// A private key file, with its passphrase if it is encrypted.
    KeyFile {
        path: PathBuf,
        passphrase: Option<Zeroizing<String>>,
    },
    Password(Zeroizing<String>),
}

impl std::fmt::Debug for Auth {
    // Never print secrets.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Auth::Agent => write!(f, "Agent"),
            Auth::KeyFile { path, passphrase } => f
                .debug_struct("KeyFile")
                .field("path", path)
                .field("passphrase", &passphrase.as_ref().map(|_| "<redacted>"))
                .finish(),
            Auth::Password(_) => write!(f, "Password(<redacted>)"),
        }
    }
}

impl Auth {
    fn label(&self) -> String {
        match self {
            Auth::Agent => "ssh-agent".into(),
            Auth::KeyFile { path, .. } => format!("key {}", path.display()),
            Auth::Password(_) => "password".into(),
        }
    }

    /// What OpenSSH tries when nothing is configured: the agent, then the
    /// default key files that exist in `ssh_dir`.
    pub fn defaults(ssh_dir: &Path) -> Vec<Auth> {
        let mut out = vec![Auth::Agent];
        for name in ["id_ed25519", "id_ecdsa", "id_rsa"] {
            let path = ssh_dir.join(name);
            if path.is_file() {
                out.push(Auth::KeyFile {
                    path,
                    passphrase: None,
                });
            }
        }
        out
    }
}

/// Where and how to connect.
#[derive(Debug, Clone)]
pub struct ConnectOptions {
    pub host: String,
    pub port: u16,
    pub user: String,
    /// Tried in order until one succeeds. Empty means [`Auth::defaults`].
    pub auth: Vec<Auth>,
    /// Hosts to hop through first (ProxyJump), outermost first.
    pub jump: Vec<ConnectOptions>,
    /// The `known_hosts` file to verify against.
    pub known_hosts: PathBuf,
    pub timeout: Duration,
}

impl ConnectOptions {
    /// Options with defaults: port 22, default auth, `~/.ssh/known_hosts`.
    pub fn new(host: impl Into<String>, user: impl Into<String>) -> Self {
        ConnectOptions {
            host: host.into(),
            port: 22,
            user: user.into(),
            auth: Vec::new(),
            jump: Vec::new(),
            known_hosts: known_hosts::default_path().unwrap_or_else(|| "known_hosts".into()),
            timeout: Duration::from_secs(15),
        }
    }

    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    pub fn auth(mut self, auth: Auth) -> Self {
        self.auth.push(auth);
        self
    }

    pub fn known_hosts(mut self, path: impl Into<PathBuf>) -> Self {
        self.known_hosts = path.into();
        self
    }

    pub fn jump(mut self, via: ConnectOptions) -> Self {
        self.jump.push(via);
        self
    }
}

/// The russh handler: verifies the server key and remembers why it refused one.
struct Verifier {
    host: String,
    port: u16,
    known_hosts: PathBuf,
    refusal: Arc<Mutex<Option<SshError>>>,
}

impl client::Handler for Verifier {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            // Host certificates need a CA in known_hosts (@cert-authority); not supported yet.
            self.refuse(SshError::KnownHosts(
                "host certificates are not supported yet".into(),
            ));
            return Ok(false);
        };
        let info = HostKeyInfo::new(&self.host, self.port, key);
        match known_hosts::check(&self.known_hosts, &self.host, self.port, key) {
            Ok(Verdict::Trusted) => Ok(true),
            Ok(Verdict::Unknown) => {
                self.refuse(SshError::UnknownHostKey {
                    key: Box::new(info),
                });
                Ok(false)
            }
            Ok(Verdict::Changed { line }) => {
                self.refuse(SshError::HostKeyChanged {
                    key: Box::new(info),
                    known_hosts: self.known_hosts.clone(),
                    line,
                });
                Ok(false)
            }
            Err(e) => {
                self.refuse(e);
                Ok(false)
            }
        }
    }
}

impl Verifier {
    fn refuse(&self, err: SshError) {
        if let Ok(mut slot) = self.refusal.lock() {
            *slot = Some(err);
        }
    }
}

/// An authenticated SSH connection.
pub struct SshClient {
    handle: Handle<Verifier>,
    /// Jump connections must stay open while this one uses them.
    _via: Vec<SshClient>,
    host: String,
}

impl std::fmt::Debug for SshClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshClient")
            .field("host", &self.host)
            .finish_non_exhaustive()
    }
}

fn config() -> Arc<client::Config> {
    Arc::new(client::Config {
        // Keep idle sessions alive through NAT and firewalls.
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    })
}

/// Connects (through any jump hosts), verifies the host key and authenticates.
pub async fn connect(opts: &ConnectOptions) -> Result<SshClient, SshError> {
    let timeout = opts.timeout;
    let fut = connect_inner(opts);
    match tokio::time::timeout(timeout * (opts.jump.len() as u32 + 1), fut).await {
        Ok(res) => res,
        Err(_) => Err(SshError::Timeout {
            host: opts.host.clone(),
            port: opts.port,
        }),
    }
}

async fn connect_inner(opts: &ConnectOptions) -> Result<SshClient, SshError> {
    // Connect through the jump chain: each hop opens a direct-tcpip channel
    // to the next host, and the SSH session runs inside that channel.
    let mut via: Vec<SshClient> = Vec::new();
    for hop in &opts.jump {
        let next = match via.last() {
            None => connect_tcp(hop).await?,
            Some(prev) => connect_through(prev, hop).await?,
        };
        via.push(next);
    }
    let mut client = match via.last() {
        None => connect_tcp(opts).await?,
        Some(prev) => connect_through(prev, opts).await?,
    };
    client._via = via;
    Ok(client)
}

fn verifier(opts: &ConnectOptions) -> (Verifier, Arc<Mutex<Option<SshError>>>) {
    let refusal = Arc::new(Mutex::new(None));
    (
        Verifier {
            host: opts.host.clone(),
            port: opts.port,
            known_hosts: opts.known_hosts.clone(),
            refusal: Arc::clone(&refusal),
        },
        refusal,
    )
}

/// Prefers the verifier's reason (unknown or changed host key) over russh's generic error.
fn handshake_error(
    opts: &ConnectOptions,
    refusal: &Mutex<Option<SshError>>,
    err: russh::Error,
) -> SshError {
    if let Some(reason) = refusal.lock().ok().and_then(|mut r| r.take()) {
        return reason;
    }
    match err {
        russh::Error::IO(e) => SshError::Connect {
            host: opts.host.clone(),
            port: opts.port,
            reason: e.to_string(),
        },
        other => SshError::Protocol(other.to_string()),
    }
}

async fn connect_tcp(opts: &ConnectOptions) -> Result<SshClient, SshError> {
    let (handler, refusal) = verifier(opts);
    let handle = client::connect(config(), (opts.host.as_str(), opts.port), handler)
        .await
        .map_err(|e| handshake_error(opts, &refusal, e))?;
    finish(handle, opts).await
}

async fn connect_through(prev: &SshClient, opts: &ConnectOptions) -> Result<SshClient, SshError> {
    let channel = prev
        .handle
        .channel_open_direct_tcpip(opts.host.clone(), opts.port.into(), "127.0.0.1", 0)
        .await
        .map_err(|e| SshError::Connect {
            host: opts.host.clone(),
            port: opts.port,
            reason: format!("jump host {} could not open a tunnel: {e}", prev.host),
        })?;
    let (handler, refusal) = verifier(opts);
    let handle = client::connect_stream(config(), channel.into_stream(), handler)
        .await
        .map_err(|e| handshake_error(opts, &refusal, e))?;
    finish(handle, opts).await
}

async fn finish(
    mut handle: Handle<Verifier>,
    opts: &ConnectOptions,
) -> Result<SshClient, SshError> {
    authenticate(&mut handle, opts).await?;
    Ok(SshClient {
        handle,
        _via: Vec::new(),
        host: opts.host.clone(),
    })
}

async fn authenticate(
    handle: &mut Handle<Verifier>,
    opts: &ConnectOptions,
) -> Result<(), SshError> {
    let methods = if opts.auth.is_empty() {
        let ssh_dir = opts
            .known_hosts
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        Auth::defaults(&ssh_dir)
    } else {
        opts.auth.clone()
    };

    let mut tried = Vec::new();
    for method in &methods {
        tried.push(method.label());
        let ok = match method {
            Auth::Agent => try_agent(handle, &opts.user).await?,
            Auth::KeyFile { path, passphrase } => {
                let key = match russh::keys::load_secret_key(
                    path,
                    passphrase.as_deref().map(|p| p.as_str()),
                ) {
                    Ok(key) => key,
                    Err(russh::keys::Error::KeyIsEncrypted) => {
                        // With default keys, skip an encrypted one; when the user named it, ask.
                        if opts.auth.is_empty() {
                            continue;
                        }
                        return Err(SshError::KeyNeedsPassphrase { path: path.clone() });
                    }
                    Err(e) => {
                        return Err(SshError::Key {
                            path: path.clone(),
                            reason: e.to_string(),
                        })
                    }
                };
                let hash = handle.best_supported_rsa_hash().await?.flatten();
                handle
                    .authenticate_publickey(
                        &opts.user,
                        PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                    )
                    .await?
                    .success()
            }
            Auth::Password(pw) => handle
                .authenticate_password(&opts.user, pw.as_str())
                .await?
                .success(),
        };
        if ok {
            return Ok(());
        }
    }
    Err(SshError::AuthFailed {
        user: opts.user.clone(),
        host: opts.host.clone(),
        tried,
    })
}

/// Tries every key the agent holds. No agent running is not an error.
async fn try_agent(handle: &mut Handle<Verifier>, user: &str) -> Result<bool, SshError> {
    #[cfg(unix)]
    let agent = russh::keys::agent::client::AgentClient::connect_env().await;
    #[cfg(windows)]
    let agent = match russh::keys::agent::client::AgentClient::connect_named_pipe(
        r"\\.\pipe\openssh-ssh-agent",
    )
    .await
    {
        Ok(a) => Ok(a.dynamic()),
        Err(_) => russh::keys::agent::client::AgentClient::connect_pageant()
            .await
            .map(|a| a.dynamic()),
    };
    let Ok(mut agent) = agent else {
        return Ok(false);
    };
    let Ok(identities) = agent.request_identities().await else {
        return Ok(false);
    };
    let hash = handle.best_supported_rsa_hash().await?.flatten();
    for identity in identities {
        let russh::keys::agent::AgentIdentity::PublicKey { key, .. } = identity else {
            continue;
        };
        match handle
            .authenticate_publickey_with(user, key, hash, &mut agent)
            .await
        {
            Ok(result) if result.success() => return Ok(true),
            Ok(_) => continue,
            Err(e) => return Err(SshError::Protocol(format!("ssh-agent: {e}"))),
        }
    }
    Ok(false)
}

impl SshClient {
    /// Opens an interactive login shell with a PTY of the given size.
    /// Output is streamed to `on_output`; `on_exit` gets the exit code once.
    pub async fn open_shell<O, E>(
        &self,
        size: ShellSize,
        on_output: O,
        on_exit: E,
    ) -> Result<SshShell, SshError>
    where
        O: FnMut(&[u8]) + Send + 'static,
        E: FnOnce(Option<u32>) + Send + 'static,
    {
        let channel = self.handle.channel_open_session().await?;
        channel
            .request_pty(
                true,
                "xterm-256color",
                size.cols.into(),
                size.rows.into(),
                0,
                0,
                &[],
            )
            .await?;
        channel.request_shell(true).await?;
        Ok(SshShell::start(channel, on_output, on_exit))
    }

    /// Runs one command without a PTY and returns its stdout and exit code.
    /// Opens a channel running a subsystem (such as `sftp`) as a byte stream.
    pub(crate) async fn subsystem(
        &self,
        name: &str,
    ) -> Result<russh::ChannelStream<client::Msg>, SshError> {
        let channel = self.handle.channel_open_session().await?;
        channel.request_subsystem(true, name).await?;
        Ok(channel.into_stream())
    }

    pub async fn exec(&self, command: &str) -> Result<(Vec<u8>, Option<u32>), SshError> {
        let mut channel = self.handle.channel_open_session().await?;
        channel.exec(true, command).await?;
        let mut out = Vec::new();
        let mut code = None;
        while let Some(msg) = channel.wait().await {
            match msg {
                russh::ChannelMsg::Data { data } => out.extend_from_slice(&data),
                russh::ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status),
                _ => {}
            }
        }
        Ok((out, code))
    }

    /// Closes the connection.
    pub async fn disconnect(&self) -> Result<(), SshError> {
        self.handle
            .disconnect(russh::Disconnect::ByApplication, "", "en")
            .await?;
        Ok(())
    }
}
