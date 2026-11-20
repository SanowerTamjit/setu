//! Local shell sessions for Setu.
//!
//! This crate does two things:
//!
//! - [`shells`] finds the shells installed on this machine (bash, zsh, fish,
//!   PowerShell, cmd and WSL distros) so Setu can open a working terminal on
//!   first launch without asking the user anything.
//! - [`session`] runs a shell inside a pseudo-terminal (via `portable-pty`):
//!   write input, read output on a background thread, resize, and wait for exit.
//!
//! It contains no UI code. The desktop app and the CLI both use it.

pub mod session;
pub mod shells;

pub use session::{ExitStatus, PtyError, PtySession, PtySize, SpawnOptions};
pub use shells::{default_shell, detect_shells, ShellKind, ShellProfile};
