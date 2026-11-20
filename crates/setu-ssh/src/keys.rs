//! Local checks on private keys and ssh-agent, used by `setu doctor`.
//!
//! Nothing here reads a passphrase or prints key material: a key is only
//! loaded without a passphrase to learn whether it is encrypted.

use std::io::Read;
use std::path::Path;

/// What loading a private key without a passphrase showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyState {
    /// Loads without a passphrase.
    Plain,
    /// Needs a passphrase.
    Encrypted,
    /// Could not be read or parsed, with the reason.
    Unreadable(String),
}

/// Loads `path` without a passphrase to see whether it is usable.
pub fn key_state(path: &Path) -> KeyState {
    match russh::keys::load_secret_key(path, None) {
        Ok(_) => KeyState::Plain,
        Err(russh::keys::Error::KeyIsEncrypted) => KeyState::Encrypted,
        Err(e) => KeyState::Unreadable(e.to_string()),
    }
}

/// Whether `path` starts like a PEM or OpenSSH private key. Reads at most
/// the first line, so it is cheap to run on every file in `~/.ssh`.
pub fn looks_like_private_key(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = Vec::with_capacity(64);
    if file.take(64).read_to_end(&mut head).is_err() {
        return false;
    }
    let head = String::from_utf8_lossy(&head);
    let first = head.lines().next().unwrap_or_default();
    first.starts_with("-----BEGIN ") && first.contains("PRIVATE KEY")
}

/// The SHA-256 fingerprint of the public key next to `private_key`
/// (`<path>.pub`), if there is a readable one. Used to tell whether the
/// agent already holds that key without asking for its passphrase.
pub fn public_fingerprint(private_key: &Path) -> Option<String> {
    let mut pub_path = private_key.as_os_str().to_owned();
    pub_path.push(".pub");
    let text = std::fs::read_to_string(pub_path).ok()?;
    let key = russh::keys::PublicKey::from_openssh(text.trim()).ok()?;
    Some(key.fingerprint(russh::keys::HashAlg::Sha256).to_string())
}

/// One key held by the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentKey {
    /// Such as `ssh-ed25519`.
    pub algorithm: String,
    /// `SHA256:…`, as `ssh-add -l` shows it.
    pub fingerprint: String,
    pub comment: String,
}

/// Whether an ssh-agent could be reached, and what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentState {
    /// No agent is configured (`SSH_AUTH_SOCK` unset, or no agent pipe).
    NotConfigured,
    /// An agent is configured but did not answer, with the reason.
    Unreachable(String),
    /// The agent answered with these keys (possibly none).
    Keys(Vec<AgentKey>),
}

/// Asks the running agent for its keys: `SSH_AUTH_SOCK` on Unix, the
/// OpenSSH agent pipe or Pageant on Windows.
pub async fn agent_state() -> AgentState {
    #[cfg(unix)]
    {
        match std::env::var_os("SSH_AUTH_SOCK").filter(|v| !v.is_empty()) {
            None => AgentState::NotConfigured,
            Some(sock) => agent_state_at(Path::new(&sock)).await,
        }
    }
    #[cfg(windows)]
    {
        use russh::keys::agent::client::AgentClient;
        match AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
            Ok(agent) => identities(agent.dynamic()).await,
            Err(_) => match AgentClient::connect_pageant().await {
                Ok(agent) => identities(agent.dynamic()).await,
                Err(_) => AgentState::NotConfigured,
            },
        }
    }
}

/// Asks the agent listening on the Unix socket `sock` for its keys.
#[cfg(unix)]
pub async fn agent_state_at(sock: &Path) -> AgentState {
    match russh::keys::agent::client::AgentClient::connect_uds(sock).await {
        Ok(agent) => identities(agent).await,
        Err(e) => AgentState::Unreachable(format!("{}: {e}", sock.display())),
    }
}

async fn identities<S>(mut agent: russh::keys::agent::client::AgentClient<S>) -> AgentState
where
    S: russh::keys::agent::client::AgentStream + Unpin + Send + 'static,
{
    use russh::keys::agent::AgentIdentity;
    use russh::keys::HashAlg;
    match agent.request_identities().await {
        Ok(ids) => AgentState::Keys(
            ids.into_iter()
                .map(|id| match id {
                    AgentIdentity::PublicKey { key, comment } => AgentKey {
                        algorithm: key.algorithm().as_str().to_string(),
                        fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
                        comment,
                    },
                    AgentIdentity::Certificate {
                        certificate,
                        comment,
                    } => AgentKey {
                        algorithm: certificate.algorithm().as_str().to_string(),
                        fingerprint: certificate
                            .public_key()
                            .fingerprint(HashAlg::Sha256)
                            .to_string(),
                        comment,
                    },
                })
                .collect(),
        ),
        Err(e) => AgentState::Unreachable(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_private_key_headers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let write = |name: &str, text: &str| {
            let p = dir.path().join(name);
            std::fs::write(&p, text).expect("write");
            p
        };
        assert!(looks_like_private_key(&write(
            "a",
            "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n"
        )));
        assert!(looks_like_private_key(&write(
            "b",
            "-----BEGIN RSA PRIVATE KEY-----\n"
        )));
        assert!(!looks_like_private_key(&write(
            "c.pub",
            "ssh-ed25519 AAAA x\n"
        )));
        assert!(!looks_like_private_key(&write(
            "known_hosts",
            "host ssh-ed25519 AAAA\n"
        )));
        assert!(!looks_like_private_key(&dir.path().join("missing")));
    }

    #[test]
    fn garbage_key_is_unreadable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("id_bad");
        std::fs::write(&p, "-----BEGIN OPENSSH PRIVATE KEY-----\nnot base64\n").expect("write");
        assert!(matches!(key_state(&p), KeyState::Unreadable(_)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn missing_agent_socket_is_unreachable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = agent_state_at(&dir.path().join("nope.sock")).await;
        assert!(matches!(state, AgentState::Unreachable(_)));
    }
}
