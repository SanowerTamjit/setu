//! A shell running inside a pseudo-terminal.
//!
//! [`PtySession::spawn`] starts a program in a new PTY and calls your output
//! handler from a background thread with every chunk the program writes. Input
//! goes in through [`PtySession::write`], and [`PtySession::resize`] keeps the
//! PTY in step with the terminal widget.
//!
//! Output is passed through as raw bytes: decoding escape sequences is the job
//! of the terminal emulator (xterm.js in the desktop app).
//!
//! On Windows, ConPTY starts by asking the terminal for the cursor position
//! (`ESC [ 6 n`) and holds back the program's output until it gets an answer.
//! A real terminal emulator answers on its own. Headless callers (tests, the
//! CLI) must answer too; [`CURSOR_POSITION_REQUEST`] and
//! [`CURSOR_POSITION_REPLY`] are provided for that.

use crate::shells::ShellProfile;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

/// The Device Status Report a terminal sends to ask for the cursor position.
pub const CURSOR_POSITION_REQUEST: &[u8] = b"\x1b[6n";

/// A reply to [`CURSOR_POSITION_REQUEST`] saying the cursor is at row 1, column 1.
pub const CURSOR_POSITION_REPLY: &[u8] = b"\x1b[1;1R";

/// Errors from PTY sessions.
#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("could not open a pseudo-terminal: {0}")]
    Open(String),
    #[error("could not start `{program}`: {reason}")]
    Spawn { program: String, reason: String },
    #[error("terminal I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not resize the terminal: {0}")]
    Resize(String),
    #[error("the session's internal state was poisoned by a panic")]
    Poisoned,
}

/// Terminal size in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
}

impl Default for PtySize {
    fn default() -> Self {
        PtySize { rows: 24, cols: 80 }
    }
}

impl From<PtySize> for portable_pty::PtySize {
    fn from(s: PtySize) -> Self {
        portable_pty::PtySize {
            rows: s.rows.max(1),
            cols: s.cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

/// What to run and how.
#[derive(Debug, Clone)]
pub struct SpawnOptions {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Working directory; defaults to the user's home directory.
    pub cwd: Option<PathBuf>,
    /// Extra environment variables, added to the inherited environment.
    pub env: Vec<(String, String)>,
    pub size: PtySize,
}

impl SpawnOptions {
    /// Runs `program` with no arguments.
    pub fn new(program: impl Into<PathBuf>) -> Self {
        SpawnOptions {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            size: PtySize::default(),
        }
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn size(mut self, size: PtySize) -> Self {
        self.size = size;
        self
    }
}

impl From<&ShellProfile> for SpawnOptions {
    fn from(profile: &ShellProfile) -> Self {
        SpawnOptions::new(&profile.program).args(profile.args.clone())
    }
}

/// How the program ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus {
    pub code: u32,
}

impl ExitStatus {
    pub fn success(&self) -> bool {
        self.code == 0
    }
}

/// A running program inside a pseudo-terminal.
///
/// Dropping the session kills the program if it is still running.
pub struct PtySession {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    reader: Option<JoinHandle<()>>,
    size: Mutex<PtySize>,
}

impl std::fmt::Debug for PtySession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PtySession")
            .field("size", &self.size.lock().ok().map(|s| *s))
            .finish_non_exhaustive()
    }
}

fn lock<T: ?Sized>(m: &Mutex<T>) -> Result<MutexGuard<'_, T>, PtyError> {
    m.lock().map_err(|_| PtyError::Poisoned)
}

impl PtySession {
    /// Starts the program and streams its output to `on_output` from a
    /// background thread. The thread ends when the PTY closes.
    pub fn spawn<F>(opts: SpawnOptions, mut on_output: F) -> Result<Self, PtyError>
    where
        F: FnMut(&[u8]) + Send + 'static,
    {
        let pair = native_pty_system()
            .openpty(opts.size.into())
            .map_err(|e| PtyError::Open(e.to_string()))?;

        let mut cmd = CommandBuilder::new(&opts.program);
        cmd.args(&opts.args);
        // Advertise a modern terminal; xterm.js supports 256 colours and true colour.
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "Setu");
        cmd.env("TERM_PROGRAM_VERSION", setu_core::VERSION);
        for (k, v) in &opts.env {
            cmd.env(k, v);
        }
        match &opts.cwd {
            Some(dir) => cmd.cwd(dir),
            None => {
                if let Some(home) = home_dir() {
                    cmd.cwd(home);
                }
            }
        }

        let child = pair.slave.spawn_command(cmd).map_err(|e| PtyError::Spawn {
            program: opts.program.display().to_string(),
            reason: e.to_string(),
        })?;
        // The slave end belongs to the child now. Keeping it open here would
        // stop the reader from seeing end-of-file when the child exits.
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::Open(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| PtyError::Open(e.to_string()))?;

        let reader_thread = std::thread::Builder::new()
            .name("setu-pty-reader".into())
            .spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => on_output(&buf[..n]),
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        // EIO is how Linux reports that the child closed the PTY.
                        Err(_) => break,
                    }
                }
            })?;

        Ok(PtySession {
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            child: Mutex::new(child),
            reader: Some(reader_thread),
            size: Mutex::new(opts.size),
        })
    }

    /// Sends input (keystrokes, pasted text) to the program.
    pub fn write(&self, data: &[u8]) -> Result<(), PtyError> {
        let mut w = lock(&self.writer)?;
        w.write_all(data)?;
        w.flush()?;
        Ok(())
    }

    /// Changes the terminal size; the program receives `SIGWINCH` (or the ConPTY equivalent).
    pub fn resize(&self, size: PtySize) -> Result<(), PtyError> {
        lock(&self.master)?
            .resize(size.into())
            .map_err(|e| PtyError::Resize(e.to_string()))?;
        *lock(&self.size)? = size;
        Ok(())
    }

    /// The current terminal size.
    pub fn size(&self) -> PtySize {
        self.size.lock().map(|s| *s).unwrap_or_default()
    }

    /// Returns the exit status if the program has ended, without blocking.
    pub fn try_wait(&self) -> Result<Option<ExitStatus>, PtyError> {
        Ok(lock(&self.child)?.try_wait()?.map(|s| ExitStatus {
            code: s.exit_code(),
        }))
    }

    /// Blocks until the program ends, or until `timeout` passes (`Ok(None)`).
    pub fn wait_timeout(&self, timeout: Duration) -> Result<Option<ExitStatus>, PtyError> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(Some(status));
            }
            if std::time::Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Terminates the program.
    pub fn kill(&self) -> Result<(), PtyError> {
        // Kill through the child itself: on Windows a cloned killer handle can
        // be rejected by TerminateProcess ("The handle is invalid").
        lock(&self.child)?.kill()?;
        Ok(())
    }

    /// The program's process id, if the platform reports one.
    pub fn process_id(&self) -> Option<u32> {
        self.child.lock().ok().and_then(|c| c.process_id())
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        if matches!(self.try_wait(), Ok(None)) {
            let _ = self.kill();
        }
        // Do not join the reader: on Windows (ConPTY) it can stay blocked until
        // the master is dropped, which happens after this. It exits on its own.
        drop(self.reader.take());
    }
}

fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}
