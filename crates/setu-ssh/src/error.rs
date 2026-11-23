use crate::known_hosts::HostKeyInfo;
use std::path::PathBuf;

/// Errors from SSH connections.
///
/// Messages are safe to show to users: they never contain secrets.
#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error("could not reach {host}:{port}: {reason}")]
    Connect {
        host: String,
        port: u16,
        reason: String,
    },

    #[error("timed out connecting to {host}:{port}")]
    Timeout { host: String, port: u16 },

    /// The server's key is not in `known_hosts`. Show `key` to the user and
    /// call [`crate::known_hosts::trust`] only if they accept it.
    #[error("the host key for {} is not known ({} {})", .key.host_label(), .key.algorithm, .key.fingerprint)]
    UnknownHostKey { key: Box<HostKeyInfo> },

    /// The server's key differs from the one in `known_hosts`. This can mean a
    /// man-in-the-middle attack, so it is never overridden automatically.
    #[error(
        "WARNING: the host key for {} has CHANGED (now {} {}); known_hosts line {line} has a different key",
        .key.host_label(), .key.algorithm, .key.fingerprint
    )]
    HostKeyChanged {
        key: Box<HostKeyInfo>,
        known_hosts: PathBuf,
        line: usize,
    },

    #[error("authentication failed for {user}@{host}; tried: {}", .tried.join(", "))]
    AuthFailed {
        user: String,
        host: String,
        tried: Vec<String>,
    },

    /// The key file is encrypted; ask the user for its passphrase.
    #[error("the key {path} needs a passphrase")]
    KeyNeedsPassphrase { path: PathBuf },

    #[error("could not load the key {path}: {reason}")]
    Key { path: PathBuf, reason: String },

    /// A FIDO2/U2F key could not be used: no agent, or the device did not sign.
    #[error("could not use the security key {path}: {reason}")]
    SecurityKey { path: PathBuf, reason: String },

    #[error("known_hosts error: {0}")]
    KnownHosts(String),

    #[error("SSH protocol error: {0}")]
    Protocol(String),

    #[error("the SSH session is closed")]
    Closed,

    /// A file operation failed; `path` is the local or remote path involved.
    #[error("{path}: {reason}")]
    Files { path: String, reason: String },

    #[error("the transfer was cancelled")]
    Cancelled,
}

impl From<russh::Error> for SshError {
    fn from(e: russh::Error) -> Self {
        SshError::Protocol(e.to_string())
    }
}
