//! An interactive remote shell.

use russh::client::Msg;
use russh::{Channel, ChannelMsg};
use tokio::sync::mpsc;

/// Terminal size in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellSize {
    pub cols: u16,
    pub rows: u16,
}

enum Command {
    Data(Vec<u8>),
    Resize(ShellSize),
    Close,
}

/// A running remote shell. Cheap to clone; all clones talk to the same channel.
#[derive(Debug, Clone)]
pub struct SshShell {
    tx: mpsc::UnboundedSender<Command>,
}

impl SshShell {
    pub(crate) fn start<O, E>(mut channel: Channel<Msg>, mut on_output: O, on_exit: E) -> Self
    where
        O: FnMut(&[u8]) + Send + 'static,
        E: FnOnce(Option<u32>) + Send + 'static,
    {
        let (tx, mut rx) = mpsc::unbounded_channel::<Command>();
        tokio::spawn(async move {
            let mut exit_code = None;
            loop {
                tokio::select! {
                    msg = channel.wait() => match msg {
                        Some(ChannelMsg::Data { data }) => on_output(&data),
                        // stderr goes to the same terminal, as with OpenSSH.
                        Some(ChannelMsg::ExtendedData { data, .. }) => on_output(&data),
                        Some(ChannelMsg::ExitStatus { exit_status }) => exit_code = Some(exit_status),
                        Some(ChannelMsg::Eof) => {}
                        Some(ChannelMsg::Close) | None => break,
                        Some(_) => {}
                    },
                    cmd = rx.recv() => match cmd {
                        Some(Command::Data(bytes)) => {
                            if channel.data(&bytes[..]).await.is_err() { break; }
                        }
                        Some(Command::Resize(s)) => {
                            let _ = channel.window_change(s.cols.into(), s.rows.into(), 0, 0).await;
                        }
                        Some(Command::Close) | None => {
                            let _ = channel.eof().await;
                            let _ = channel.close().await;
                            break;
                        }
                    }
                }
            }
            on_exit(exit_code);
        });
        SshShell { tx }
    }

    /// Sends keystrokes or pasted text.
    pub fn write(&self, data: &[u8]) -> Result<(), crate::SshError> {
        self.tx
            .send(Command::Data(data.to_vec()))
            .map_err(|_| crate::SshError::Closed)
    }

    /// Tells the server the terminal was resized.
    pub fn resize(&self, size: ShellSize) -> Result<(), crate::SshError> {
        self.tx
            .send(Command::Resize(size))
            .map_err(|_| crate::SshError::Closed)
    }

    /// Ends the shell session.
    pub fn close(&self) {
        let _ = self.tx.send(Command::Close);
    }
}
