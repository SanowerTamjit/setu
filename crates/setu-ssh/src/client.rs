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
        for name in [
            "id_ed25519",
            "id_ecdsa",
            "id_ed25519_sk",
            "id_ecdsa_sk",
            "id_rsa",
        ] {
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

impl SshClient {
    /// The host this connection logged in to (the last hop, not a jump host).
    pub fn host(&self) -> &str {
        &self.host
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
    for o in opts.jump.iter().chain(std::iter::once(opts)) {
        known_hosts::check_host(&o.host).map_err(|_| SshError::Connect {
            host: o.host.clone(),
            port: o.port,
            reason: "that is not a valid host name or address".into(),
        })?;
    }
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
                if crate::keys::is_security_key(key.algorithm()) {
                    try_security_key(handle, &opts.user, path, &key).await?
                } else {
                    let hash = handle.best_supported_rsa_hash().await?.flatten();
                    handle
                        .authenticate_publickey(
                            &opts.user,
                            PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                        )
                        .await?
                        .success()
                }
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

type Agent = russh::keys::agent::client::AgentClient<
    Box<dyn russh::keys::agent::client::AgentStream + Send + Unpin>,
>;

/// The running ssh-agent: `SSH_AUTH_SOCK` on Unix, the OpenSSH agent pipe
/// or Pageant on Windows. `None` when there is none.
async fn connect_agent() -> Option<Agent> {
    use russh::keys::agent::client::AgentClient;
    #[cfg(unix)]
    {
        AgentClient::connect_env().await.ok().map(|a| a.dynamic())
    }
    #[cfg(windows)]
    {
        match AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
            Ok(a) => Some(a.dynamic()),
            Err(_) => AgentClient::connect_pageant()
                .await
                .ok()
                .map(|a| a.dynamic()),
        }
    }
}

/// Tries every key the agent holds. No agent running is not an error. A
/// security key that cannot sign (unplugged, or not touched in time) is
/// skipped so the agent's other keys still get their turn.
async fn try_agent(handle: &mut Handle<Verifier>, user: &str) -> Result<bool, SshError> {
    let Some(mut agent) = connect_agent().await else {
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
        let security_key = crate::keys::is_security_key(key.algorithm());
        match handle
            .authenticate_publickey_with(user, key, hash, &mut agent)
            .await
        {
            Ok(result) if result.success() => return Ok(true),
            Ok(_) => continue,
            Err(_) if security_key => continue,
            Err(e) => return Err(SshError::Protocol(format!("ssh-agent: {e}"))),
        }
    }
    Ok(false)
}

/// Signs in with a FIDO2/U2F key file (`id_ed25519_sk` and the like). The
/// file holds only a handle; the signature comes from the device, which
/// OpenSSH's agent talks to. So the key is handed to the agent if it does
/// not hold it yet, and the agent asks the device to sign.
async fn try_security_key(
    handle: &mut Handle<Verifier>,
    user: &str,
    path: &Path,
    key: &russh::keys::PrivateKey,
) -> Result<bool, SshError> {
    let fail = |reason: String| SshError::SecurityKey {
        path: path.to_path_buf(),
        reason,
    };
    let mut agent = connect_agent().await.ok_or_else(|| {
        fail("security keys sign through ssh-agent, and none is running. Start ssh-agent (on Windows, the OpenSSH Authentication Agent service) and try again".into())
    })?;
    let public = key.public_key().clone();
    let held = agent
        .request_identities()
        .await
        .map_err(|e| fail(format!("ssh-agent did not answer: {e}")))?
        .into_iter()
        .any(|id| matches!(id, russh::keys::agent::AgentIdentity::PublicKey { key, .. } if key.key_data() == public.key_data()));
    if !held {
        agent.add_identity(key, &[]).await.map_err(|e| {
            fail(format!(
                "ssh-agent would not take the key ({e}). Its OpenSSH may be older than 8.2, which added security keys"
            ))
        })?;
    }
    match handle
        .authenticate_publickey_with(user, public, None, &mut agent)
        .await
    {
        Ok(result) => Ok(result.success()),
        Err(_) => Err(fail(
            "the key did not sign. Check that it is plugged in, touch it when it blinks, and enter its PIN if asked".into(),
        )),
    }
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
        // Prompt themes (Starship, Oh My Posh, Powerlevel10k) look for
        // COLORTERM to use 24-bit colour. Most servers only accept LANG and
        // LC_* (AcceptEnv), so don't wait for an answer and don't fail.
        let _ = channel.set_env(false, "COLORTERM", "truecolor").await;
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

    /// Runs one command without a PTY, collecting stdout and stderr together
    /// as they arrive. Only the last `max_bytes` are kept, so a chatty
    /// command cannot use up memory. Returns the output and the exit code.
    pub async fn run_command(
        &self,
        command: &str,
        max_bytes: usize,
    ) -> Result<(Vec<u8>, Option<u32>), SshError> {
        let mut channel = self.handle.channel_open_session().await?;
        channel.exec(true, command).await?;
        let mut out: Vec<u8> = Vec::new();
        let mut code = None;
        let keep = |data: &[u8], out: &mut Vec<u8>| {
            out.extend_from_slice(data);
            if out.len() > max_bytes {
                let cut = out.len() - max_bytes;
                out.drain(..cut);
            }
        };
        while let Some(msg) = channel.wait().await {
            match msg {
                russh::ChannelMsg::Data { data } => keep(&data, &mut out),
                russh::ChannelMsg::ExtendedData { data, .. } => keep(&data, &mut out),
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
