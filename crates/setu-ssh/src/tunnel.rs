//! SSH tunnels: local, remote and dynamic (SOCKS5) port forwarding.
//!
//! - **Local** (`ssh -L`): Setu listens on a port on this computer, and each
//!   connection to it is carried through SSH to a host and port the server
//!   can reach.
//! - **Remote** (`ssh -R`): the server listens on a port, and each connection
//!   to it is carried back to a host and port this computer can reach.
//! - **Dynamic** (`ssh -D`): Setu runs a small SOCKS5 proxy on this computer;
//!   each app that uses it says where it wants to go, and the server makes
//!   that connection.
//!
//! A [`Forwarding`] runs until it is stopped or dropped, or the SSH
//! connection closes. Choosing which addresses a tunnel may listen on (for
//! example loopback only) is the caller's job: this module listens where it
//! is told. Connections the server opens for a remote port this client never
//! asked for are refused.

use crate::client::SshClient;
use crate::SshError;
use russh::client::{ChannelOpenHandle, Msg};
use russh::{Channel, ChannelOpenFailure};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

/// How long an app has to finish the SOCKS handshake, or a local target to
/// accept a connection the server forwarded.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// Live numbers for one forwarding, cheap to read from any thread.
#[derive(Debug, Default)]
pub struct TunnelStats {
    active: AtomicUsize,
    total: AtomicU64,
    failed: AtomicU64,
}

impl TunnelStats {
    /// Connections being carried right now.
    pub fn active(&self) -> usize {
        self.active.load(Ordering::Relaxed)
    }

    /// Connections carried since the forwarding started.
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// Connections that could not be opened at the other end.
    pub fn failed(&self) -> u64 {
        self.failed.load(Ordering::Relaxed)
    }

    fn opened(self: &Arc<Self>) -> ActiveGuard {
        self.total.fetch_add(1, Ordering::Relaxed);
        self.active.fetch_add(1, Ordering::Relaxed);
        ActiveGuard(Arc::clone(self))
    }

    fn fail(&self) {
        self.failed.fetch_add(1, Ordering::Relaxed);
    }
}

/// Counts a connection as active for as long as it lives.
struct ActiveGuard(Arc<TunnelStats>);

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }
}

/// A running port forward. Stops when dropped.
#[derive(Debug)]
pub struct Forwarding {
    stop: watch::Sender<bool>,
    stats: Arc<TunnelStats>,
    /// Where it listens: a local address, or the server-side address and
    /// port for a remote forward.
    listening: String,
    port: u16,
    /// For a remote forward: undo the request on the server when stopped.
    remote: Option<(Arc<SshClient>, String, u32)>,
}

impl Forwarding {
    /// Live connection counts.
    pub fn stats(&self) -> &Arc<TunnelStats> {
        &self.stats
    }

    /// The port it listens on. When port 0 was asked for, this is the port
    /// the system (or the server) picked.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// `address:port` it listens on, for showing to people.
    pub fn listening(&self) -> &str {
        &self.listening
    }

    /// Stops listening and closes every connection it carries.
    pub async fn stop(mut self) {
        let _ = self.stop.send(true);
        if let Some((client, address, port)) = self.remote.take() {
            client.forget_remote(port);
            let _ = client.handle().cancel_tcpip_forward(address, port).await;
        }
    }
}

impl Drop for Forwarding {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        if let Some((client, address, port)) = self.remote.take() {
            client.forget_remote(port);
            // Tell the server too, when there is a runtime to do it on.
            if let Ok(rt) = tokio::runtime::Handle::try_current() {
                rt.spawn(async move {
                    let _ = client.handle().cancel_tcpip_forward(address, port).await;
                });
            }
        }
    }
}

/// Where connections the server forwards for one remote port go.
#[derive(Debug, Clone)]
pub(crate) struct RemoteTarget {
    host: String,
    port: u16,
    stats: Arc<TunnelStats>,
    stop: watch::Receiver<bool>,
}

/// Remote forwards this client asked for, by the port the server listens on.
pub(crate) type RemoteForwards = Arc<Mutex<HashMap<u32, RemoteTarget>>>;

/// Called by the connection's handler when the server opens a channel for a
/// remote forward. Only ports this client asked for are accepted.
pub(crate) async fn accept_forwarded(
    forwards: &RemoteForwards,
    channel: Channel<Msg>,
    connected_port: u32,
    reply: ChannelOpenHandle,
) {
    let target = forwards
        .lock()
        .ok()
        .and_then(|f| f.get(&connected_port).cloned());
    let Some(target) = target else {
        reply
            .reject(ChannelOpenFailure::AdministrativelyProhibited)
            .await;
        return;
    };
    reply.accept().await;
    tokio::spawn(async move {
        let mut stop = target.stop.clone();
        let connect = TcpStream::connect((target.host.as_str(), target.port));
        let tcp = match tokio::time::timeout(HANDSHAKE_TIMEOUT, connect).await {
            Ok(Ok(tcp)) => tcp,
            _ => {
                target.stats.fail();
                let _ = channel.close().await;
                return;
            }
        };
        let _active = target.stats.opened();
        pipe(tcp, channel, &mut stop).await;
    });
}

/// Copies both ways between a TCP connection and an SSH channel until
/// either side closes or the forwarding stops.
async fn pipe(mut tcp: TcpStream, channel: Channel<Msg>, stop: &mut watch::Receiver<bool>) {
    let mut stream = channel.into_stream();
    tokio::select! {
        _ = tokio::io::copy_bidirectional(&mut tcp, &mut stream) => {}
        _ = stopped(stop) => {}
    }
    let _ = stream.shutdown().await;
}

/// Resolves once the forwarding is told to stop (or its owner is gone).
async fn stopped(stop: &mut watch::Receiver<bool>) {
    while !*stop.borrow() {
        if stop.changed().await.is_err() {
            return;
        }
    }
}

async fn bind(address: &str, port: u16) -> Result<TcpListener, SshError> {
    TcpListener::bind((address, port))
        .await
        .map_err(|e| SshError::Tunnel(format!("could not listen on {address}:{port}: {e}")))
}

fn shown(addr: SocketAddr) -> String {
    addr.to_string()
}

impl SshClient {
    /// Local forward (`ssh -L`): listens on `bind_address:bind_port` here
    /// and carries each connection to `target_host:target_port` as seen from
    /// the server. Port 0 picks a free port; see [`Forwarding::port`].
    pub async fn forward_local(
        self: &Arc<Self>,
        bind_address: &str,
        bind_port: u16,
        target_host: &str,
        target_port: u16,
    ) -> Result<Forwarding, SshError> {
        check_target(target_host)?;
        let listener = bind(bind_address, bind_port).await?;
        let local = listener
            .local_addr()
            .map_err(|e| SshError::Tunnel(e.to_string()))?;
        let (stop, rx) = watch::channel(false);
        let stats = Arc::new(TunnelStats::default());
        let client = Arc::clone(self);
        let target = target_host.to_string();
        let counts = Arc::clone(&stats);
        tokio::spawn(accept_loop(listener, rx, move |tcp, peer, mut stop| {
            let client = Arc::clone(&client);
            let target = target.clone();
            let stats = Arc::clone(&counts);
            async move {
                let channel = client
                    .handle()
                    .channel_open_direct_tcpip(
                        target,
                        u32::from(target_port),
                        peer.ip().to_string(),
                        u32::from(peer.port()),
                    )
                    .await;
                match channel {
                    Ok(channel) => {
                        let _active = stats.opened();
                        pipe(tcp, channel, &mut stop).await;
                    }
                    Err(_) => stats.fail(),
                }
            }
        }));
        Ok(Forwarding {
            stop,
            stats,
            listening: shown(local),
            port: local.port(),
            remote: None,
        })
    }

    /// Dynamic forward (`ssh -D`): a SOCKS5 proxy on `bind_address:bind_port`
    /// here. Each app that connects names a destination and the server
    /// connects to it. Only SOCKS5 CONNECT without a password is offered.
    pub async fn forward_dynamic(
        self: &Arc<Self>,
        bind_address: &str,
        bind_port: u16,
    ) -> Result<Forwarding, SshError> {
        let listener = bind(bind_address, bind_port).await?;
        let local = listener
            .local_addr()
            .map_err(|e| SshError::Tunnel(e.to_string()))?;
        let (stop, rx) = watch::channel(false);
        let stats = Arc::new(TunnelStats::default());
        let client = Arc::clone(self);
        let counts = Arc::clone(&stats);
        tokio::spawn(accept_loop(listener, rx, move |tcp, peer, stop| {
            socks_connection(Arc::clone(&client), Arc::clone(&counts), tcp, peer, stop)
        }));
        Ok(Forwarding {
            stop,
            stats,
            listening: shown(local),
            port: local.port(),
            remote: None,
        })
    }

    /// Remote forward (`ssh -R`): asks the server to listen on
    /// `remote_address:remote_port` and carries each connection back to
    /// `target_host:target_port` as seen from this computer. Remote port 0
    /// lets the server pick; see [`Forwarding::port`]. Whether the server
    /// listens on other than loopback is up to its `GatewayPorts` setting.
    pub async fn forward_remote(
        self: &Arc<Self>,
        remote_address: &str,
        remote_port: u16,
        target_host: &str,
        target_port: u16,
    ) -> Result<Forwarding, SshError> {
        check_target(target_host)?;
        let (stop, rx) = watch::channel(false);
        let stats = Arc::new(TunnelStats::default());
        let asked = u32::from(remote_port);
        let given = self
            .handle()
            .tcpip_forward(remote_address, asked)
            .await
            .map_err(|e| {
                SshError::Tunnel(format!(
                    "the server would not listen on {remote_address}:{remote_port} ({e}). It may not allow forwarding (AllowTcpForwarding), or the port is in use"
                ))
            })?;
        // The server only reports a port when it picked one.
        let port = if asked == 0 { given } else { asked };
        let port16 = u16::try_from(port)
            .map_err(|_| SshError::Tunnel(format!("the server picked an invalid port {port}")))?;
        self.remember_remote(
            port,
            RemoteTarget {
                host: target_host.to_string(),
                port: target_port,
                stats: Arc::clone(&stats),
                stop: rx,
            },
        );
        Ok(Forwarding {
            stop,
            stats,
            listening: format!("{remote_address}:{port16} on the server"),
            port: port16,
            remote: Some((Arc::clone(self), remote_address.to_string(), port)),
        })
    }
}

/// Accepts connections until stopped, handing each to `serve` on its own
/// task. Stopping drops the listener, so the port is free again at once.
async fn accept_loop<F, Fut>(listener: TcpListener, mut stop: watch::Receiver<bool>, serve: F)
where
    F: Fn(TcpStream, SocketAddr, watch::Receiver<bool>) -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((tcp, peer)) => {
                    let _ = tcp.set_nodelay(true);
                    tokio::spawn(serve(tcp, peer, stop.clone()));
                }
                // A failed accept (too many open files, say) should not end the tunnel.
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            },
            _ = stopped(&mut stop) => break,
        }
    }
}

/// A destination must at least look like a host name or address.
fn check_target(host: &str) -> Result<(), SshError> {
    crate::known_hosts::check_host(host)
        .map_err(|_| SshError::Tunnel(format!("{host:?} is not a valid host name or address")))
}

// ---- SOCKS5 (RFC 1928), CONNECT only, no authentication ----

const SOCKS_VERSION: u8 = 5;
const NO_AUTH: u8 = 0x00;
const NO_ACCEPTABLE_METHOD: u8 = 0xff;
const CMD_CONNECT: u8 = 0x01;
const REPLY_OK: u8 = 0x00;
const REPLY_GENERAL_FAILURE: u8 = 0x01;
const REPLY_HOST_UNREACHABLE: u8 = 0x04;
const REPLY_COMMAND_NOT_SUPPORTED: u8 = 0x07;
const REPLY_ADDRESS_NOT_SUPPORTED: u8 = 0x08;

/// What a SOCKS client asked for.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SocksRequest {
    Connect {
        host: String,
        port: u16,
    },
    /// A request this proxy does not serve, with the reply code to send.
    Refused(u8),
}

/// Reads a SOCKS5 greeting and answers it. Returns false (after telling the
/// client) when it does not offer "no authentication".
async fn socks_greeting<S>(s: &mut S) -> std::io::Result<bool>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    let mut head = [0u8; 2];
    s.read_exact(&mut head).await?;
    if head[0] != SOCKS_VERSION {
        return Ok(false);
    }
    let mut methods = vec![0u8; usize::from(head[1])];
    s.read_exact(&mut methods).await?;
    if methods.contains(&NO_AUTH) {
        s.write_all(&[SOCKS_VERSION, NO_AUTH]).await?;
        Ok(true)
    } else {
        s.write_all(&[SOCKS_VERSION, NO_ACCEPTABLE_METHOD]).await?;
        Ok(false)
    }
}

/// Reads one SOCKS5 request.
pub(crate) async fn socks_request<S>(s: &mut S) -> std::io::Result<SocksRequest>
where
    S: AsyncReadExt + Unpin,
{
    let mut head = [0u8; 4];
    s.read_exact(&mut head).await?;
    let [version, command, _reserved, kind] = head;
    if version != SOCKS_VERSION {
        return Ok(SocksRequest::Refused(REPLY_GENERAL_FAILURE));
    }
    let host = match kind {
        0x01 => {
            let mut ip = [0u8; 4];
            s.read_exact(&mut ip).await?;
            std::net::Ipv4Addr::from(ip).to_string()
        }
        0x03 => {
            let len = s.read_u8().await?;
            let mut name = vec![0u8; usize::from(len)];
            s.read_exact(&mut name).await?;
            match String::from_utf8(name) {
                Ok(name) if crate::known_hosts::check_host(&name).is_ok() => name,
                _ => {
                    // Still read the port so the reply lands on a clean stream.
                    let _ = s.read_u16().await?;
                    return Ok(SocksRequest::Refused(REPLY_HOST_UNREACHABLE));
                }
            }
        }
        0x04 => {
            let mut ip = [0u8; 16];
            s.read_exact(&mut ip).await?;
            std::net::Ipv6Addr::from(ip).to_string()
        }
        _ => return Ok(SocksRequest::Refused(REPLY_ADDRESS_NOT_SUPPORTED)),
    };
    let port = s.read_u16().await?;
    if command != CMD_CONNECT {
        return Ok(SocksRequest::Refused(REPLY_COMMAND_NOT_SUPPORTED));
    }
    Ok(SocksRequest::Connect { host, port })
}

/// A SOCKS5 reply. The bound address is always 0.0.0.0:0: clients doing
/// CONNECT do not use it, and the real one is on the server.
fn socks_reply(code: u8) -> [u8; 10] {
    [SOCKS_VERSION, code, 0, 0x01, 0, 0, 0, 0, 0, 0]
}

async fn socks_connection(
    client: Arc<SshClient>,
    stats: Arc<TunnelStats>,
    mut tcp: TcpStream,
    peer: SocketAddr,
    mut stop: watch::Receiver<bool>,
) {
    let handshake = async {
        if !socks_greeting(&mut tcp).await? {
            return Ok(None);
        }
        socks_request(&mut tcp).await.map(Some)
    };
    let request = match tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake).await {
        Ok(Ok(Some(request))) => request,
        _ => return,
    };
    let (host, port) = match request {
        SocksRequest::Connect { host, port } => (host, port),
        SocksRequest::Refused(code) => {
            let _ = tcp.write_all(&socks_reply(code)).await;
            return;
        }
    };
    let channel = client
        .handle()
        .channel_open_direct_tcpip(
            host,
            u32::from(port),
            peer.ip().to_string(),
            u32::from(peer.port()),
        )
        .await;
    match channel {
        Ok(channel) => {
            if tcp.write_all(&socks_reply(REPLY_OK)).await.is_err() {
                return;
            }
            let _active = stats.opened();
            pipe(tcp, channel, &mut stop).await;
        }
        Err(_) => {
            stats.fail();
            let _ = tcp.write_all(&socks_reply(REPLY_HOST_UNREACHABLE)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn parse(bytes: &[u8]) -> SocksRequest {
        let mut input = bytes;
        socks_request(&mut input).await.expect("parse")
    }

    #[tokio::test]
    async fn socks_requests_are_read() {
        assert_eq!(
            parse(&[5, 1, 0, 1, 10, 0, 0, 7, 0x1f, 0x90]).await,
            SocksRequest::Connect {
                host: "10.0.0.7".into(),
                port: 8080
            }
        );
        let mut name = vec![5, 1, 0, 3, 11];
        name.extend_from_slice(b"example.com");
        name.extend_from_slice(&443u16.to_be_bytes());
        assert_eq!(
            parse(&name).await,
            SocksRequest::Connect {
                host: "example.com".into(),
                port: 443
            }
        );
        let mut v6 = vec![5, 1, 0, 4];
        v6.extend_from_slice(&std::net::Ipv6Addr::LOCALHOST.octets());
        v6.extend_from_slice(&22u16.to_be_bytes());
        assert_eq!(
            parse(&v6).await,
            SocksRequest::Connect {
                host: "::1".into(),
                port: 22
            }
        );
    }

    #[tokio::test]
    async fn unsupported_socks_requests_are_refused() {
        // BIND and UDP ASSOCIATE are not offered.
        assert_eq!(
            parse(&[5, 2, 0, 1, 127, 0, 0, 1, 0, 80]).await,
            SocksRequest::Refused(REPLY_COMMAND_NOT_SUPPORTED)
        );
        assert_eq!(
            parse(&[5, 1, 0, 9]).await,
            SocksRequest::Refused(REPLY_ADDRESS_NOT_SUPPORTED)
        );
        assert_eq!(
            parse(&[4, 1, 0, 1, 127, 0, 0, 1, 0, 80]).await,
            SocksRequest::Refused(REPLY_GENERAL_FAILURE)
        );
        // A name with a space or control character never reaches the server.
        let mut bad = vec![5, 1, 0, 3, 8];
        bad.extend_from_slice(b"a b\n-oX");
        bad.push(b'y');
        bad.extend_from_slice(&80u16.to_be_bytes());
        assert_eq!(
            parse(&bad).await,
            SocksRequest::Refused(REPLY_HOST_UNREACHABLE)
        );
        // A cut-off request is an error, not a connection.
        let mut short: &[u8] = &[5, 1, 0, 1, 127];
        assert!(socks_request(&mut short).await.is_err());
    }

    #[tokio::test]
    async fn greeting_needs_no_auth_offered() {
        let (mut client, mut server) = tokio::io::duplex(64);
        client.write_all(&[5, 2, 0x02, 0x00]).await.expect("write");
        assert!(socks_greeting(&mut server).await.expect("greeting"));
        let mut answer = [0u8; 2];
        client.read_exact(&mut answer).await.expect("read");
        assert_eq!(answer, [5, NO_AUTH]);

        let (mut client, mut server) = tokio::io::duplex(64);
        client.write_all(&[5, 1, 0x02]).await.expect("write");
        assert!(!socks_greeting(&mut server).await.expect("greeting"));
        client.read_exact(&mut answer).await.expect("read");
        assert_eq!(answer, [5, NO_ACCEPTABLE_METHOD]);
    }

    #[test]
    fn stats_count_connections() {
        let stats = Arc::new(TunnelStats::default());
        let a = stats.opened();
        let b = stats.opened();
        assert_eq!((stats.active(), stats.total()), (2, 2));
        drop(a);
        stats.fail();
        assert_eq!((stats.active(), stats.total(), stats.failed()), (1, 2, 1));
        drop(b);
        assert_eq!(stats.active(), 0);
    }
}
