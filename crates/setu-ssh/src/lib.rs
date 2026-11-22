//! SSH connections for Setu.
//!
//! Built on [`russh`]. What this crate adds on top:
//!
//! - **Strict host-key checking** against the user's `known_hosts` file. An
//!   unknown key is never accepted silently: [`connect`] fails with
//!   [`SshError::UnknownHostKey`], the UI shows the fingerprint, and only an
//!   explicit [`known_hosts::trust`] lets the next attempt through. A changed
//!   key fails with [`SshError::HostKeyChanged`] and cannot be overridden here.
//! - **Authentication the way OpenSSH users expect**: ssh-agent first, then the
//!   default keys in `~/.ssh`, or an explicit key file or password.
//! - **ProxyJump**: connect through one or more jump hosts.
//! - **Interactive shells** with a PTY, streamed to a callback like `setu-pty`.
//! - **SFTP**: browse, create, rename and delete remote files, and copy files
//!   or whole folders either way with progress and cancel ([`sftp`]).
//! - Local checks on private keys and ssh-agent for `setu doctor` ([`keys`]).
//!
//! Secrets (passwords, key passphrases) are wrapped in [`zeroize::Zeroizing`]
//! and are never logged or included in error messages.

mod client;
mod error;
pub mod keygen;
pub mod keys;
pub mod known_hosts;
pub mod sftp;
mod shell;

pub use client::{connect, Auth, ConnectOptions, SshClient};
pub use error::SshError;
pub use known_hosts::HostKeyInfo;
pub use shell::{ShellSize, SshShell};
pub use zeroize::Zeroizing;
