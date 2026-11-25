//! Network tools for Setu, run from this computer.
//!
//! - [`check_port`]: can a TCP connection be made, and how long does it take
//!   (a "TCP ping": it needs no administrator rights, unlike ICMP ping).
//! - [`lookup`]: what addresses a name resolves to, using the system resolver.
//! - [`wake`]: send a Wake-on-LAN magic packet.
//! - [`tls::check`]: look at a server's TLS certificate (who it is for, who
//!   signed it, when it expires, whether this computer trusts it).
//!
//! Every input is checked here ([`check_target`], [`parse_mac`]) so callers
//! that pass on text from a window cannot reach anything but a host name or
//! address. Nothing is ever run in a shell.

pub mod tls;

use serde::Serialize;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

/// Most attempts one port check makes.
pub const MAX_ATTEMPTS: u32 = 10;
/// Longest wait for one attempt.
pub const MAX_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NetError {
    #[error("{0:?} is not a host name or IP address")]
    BadTarget(String),
    #[error("{0:?} is not a MAC address (like 3c:52:82:1a:9e:07)")]
    BadMac(String),
    #[error("could not look up {name}: {reason}")]
    Lookup { name: String, reason: String },
    #[error("could not send the Wake-on-LAN packet: {0}")]
    Send(String),
    #[error("{0}")]
    Tls(String),
}

/// A host name or IP address, nothing else: no spaces, no option-like
/// leading `-`, no URL or shell characters. Returns it trimmed.
pub fn check_target(text: &str) -> Result<String, NetError> {
    let t = text.trim();
    let ok = !t.is_empty()
        && t.len() <= 253
        && !t.starts_with('-')
        && t.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':'));
    if ok {
        Ok(t.to_string())
    } else {
        Err(NetError::BadTarget(text.chars().take(80).collect()))
    }
}

/// One attempt of a port check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "result", rename_all = "camelCase")]
pub enum Attempt {
    /// Connected; how long it took, in microseconds.
    Open { micros: u64, address: String },
    /// Something answered "no one is listening".
    Refused { address: String },
    /// No answer in time (a firewall that drops, or a host that is down).
    TimedOut,
    /// Any other failure (no route, name not found), in words.
    Failed { reason: String },
}

/// Connects to `host:port` up to `attempts` times, one after another.
pub async fn check_port(
    host: &str,
    port: u16,
    attempts: u32,
    timeout: Duration,
) -> Result<Vec<Attempt>, NetError> {
    let host = check_target(host)?;
    let attempts = attempts.clamp(1, MAX_ATTEMPTS);
    let timeout = timeout.min(MAX_TIMEOUT);
    let mut out = Vec::new();
    for i in 0..attempts {
        if i > 0 {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        out.push(attempt(&host, port, timeout).await);
    }
    Ok(out)
}

async fn attempt(host: &str, port: u16, timeout: Duration) -> Attempt {
    let start = Instant::now();
    match tokio::time::timeout(timeout, tokio::net::TcpStream::connect((host, port))).await {
        Ok(Ok(tcp)) => Attempt::Open {
            micros: u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX),
            address: tcp.peer_addr().map(|a| a.to_string()).unwrap_or_default(),
        },
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => Attempt::Refused {
            address: format!("{host}:{port}"),
        },
        Ok(Err(e)) => Attempt::Failed {
            reason: e.to_string(),
        },
        Err(_) => Attempt::TimedOut,
    }
}

/// Summary of a port check, for one line of text.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub sent: usize,
    pub open: usize,
    pub min_ms: Option<f64>,
    pub avg_ms: Option<f64>,
    pub max_ms: Option<f64>,
}

pub fn summarize(attempts: &[Attempt]) -> Summary {
    let times: Vec<f64> = attempts
        .iter()
        .filter_map(|a| match a {
            Attempt::Open { micros, .. } => Some(*micros as f64 / 1000.0),
            _ => None,
        })
        .collect();
    let (min, max) = times
        .iter()
        .fold((None, None), |(lo, hi): (Option<f64>, Option<f64>), &t| {
            (
                Some(lo.map_or(t, |l| l.min(t))),
                Some(hi.map_or(t, |h| h.max(t))),
            )
        });
    Summary {
        sent: attempts.len(),
        open: times.len(),
        min_ms: min,
        avg_ms: (!times.is_empty()).then(|| times.iter().sum::<f64>() / times.len() as f64),
        max_ms: max,
    }
}

/// The addresses `name` resolves to here, IPv4 first, without duplicates.
pub async fn lookup(name: &str) -> Result<Vec<IpAddr>, NetError> {
    let name = check_target(name)?;
    if let Ok(ip) = name.parse::<IpAddr>() {
        return Ok(vec![ip]);
    }
    let found = tokio::time::timeout(MAX_TIMEOUT, tokio::net::lookup_host((name.as_str(), 0)))
        .await
        .map_err(|_| NetError::Lookup {
            name: name.clone(),
            reason: "no answer in time".into(),
        })?
        .map_err(|e| NetError::Lookup {
            name: name.clone(),
            reason: e.to_string(),
        })?;
    let mut ips: Vec<IpAddr> = Vec::new();
    for ip in found.map(|a: SocketAddr| a.ip()) {
        if !ips.contains(&ip) {
            ips.push(ip);
        }
    }
    ips.sort_by_key(|ip| ip.is_ipv6());
    Ok(ips)
}

/// Reads `3c:52:82:1a:9e:07`, `3C-52-82-1A-9E-07` or `3c5282 1a9e07`.
pub fn parse_mac(text: &str) -> Result<[u8; 6], NetError> {
    let hex: String = text
        .chars()
        .filter(|c| !matches!(c, ':' | '-' | '.' | ' '))
        .collect();
    let bad = || NetError::BadMac(text.chars().take(40).collect());
    if hex.len() != 12 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(bad());
    }
    let mut mac = [0u8; 6];
    for (i, byte) in mac.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| bad())?;
    }
    Ok(mac)
}

/// Six 0xff bytes, then the MAC sixteen times.
pub fn magic_packet(mac: [u8; 6]) -> [u8; 102] {
    let mut packet = [0xffu8; 102];
    for i in 0..16 {
        let at = 6 + i * 6;
        packet[at..at + 6].copy_from_slice(&mac);
    }
    packet
}

/// Broadcasts a magic packet for `mac` to UDP port 9 on `broadcast`
/// (255.255.255.255 when `None`). Only IPv4 broadcast addresses are allowed,
/// so this cannot be used to send packets to an arbitrary host.
pub async fn wake(mac: [u8; 6], broadcast: Option<Ipv4Addr>) -> Result<(), NetError> {
    let to = broadcast.unwrap_or(Ipv4Addr::BROADCAST);
    if !is_broadcast_like(to) {
        return Err(NetError::Send(format!(
            "{to} is not a broadcast address (it should end in .255, like 192.168.1.255)"
        )));
    }
    let send = |e: std::io::Error| NetError::Send(e.to_string());
    let socket = tokio::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .await
        .map_err(send)?;
    socket.set_broadcast(true).map_err(send)?;
    socket
        .send_to(&magic_packet(mac), (to, 9))
        .await
        .map_err(send)?;
    Ok(())
}

/// 255.255.255.255 or an address whose last byte is 255 (a /24 broadcast,
/// the usual home and office case). Loopback is allowed for tests.
fn is_broadcast_like(ip: Ipv4Addr) -> bool {
    ip.is_broadcast() || ip.octets()[3] == 255 || ip.is_loopback()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_are_names_or_addresses_only() {
        assert_eq!(check_target(" db.internal ").as_deref(), Ok("db.internal"));
        assert!(check_target("10.0.0.5").is_ok());
        assert!(check_target("fe80::1").is_ok());
        for bad in [
            "",
            "-oProxyCommand=x",
            "a b",
            "host;rm",
            "http://x",
            "a$(id)",
            "a\tb",
        ] {
            assert!(check_target(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn macs_and_magic_packets() {
        let mac = [0x3c, 0x52, 0x82, 0x1a, 0x9e, 0x07];
        assert_eq!(parse_mac("3c:52:82:1a:9e:07"), Ok(mac));
        assert_eq!(parse_mac("3C-52-82-1A-9E-07"), Ok(mac));
        assert_eq!(parse_mac("3c52.821a.9e07"), Ok(mac));
        assert!(parse_mac("3c:52:82:1a:9e").is_err());
        assert!(parse_mac("zz:52:82:1a:9e:07").is_err());
        let p = magic_packet(mac);
        assert_eq!(&p[..6], &[0xff; 6]);
        assert!((0..16).all(|i| p[6 + i * 6..12 + i * 6] == mac));
    }

    #[test]
    fn only_broadcast_addresses_are_woken() {
        assert!(is_broadcast_like(Ipv4Addr::BROADCAST));
        assert!(is_broadcast_like(Ipv4Addr::new(192, 168, 1, 255)));
        assert!(!is_broadcast_like(Ipv4Addr::new(8, 8, 8, 8)));
    }

    #[tokio::test]
    async fn wake_sends_the_packet() {
        let rx = tokio::net::UdpSocket::bind("127.0.0.1:9").await;
        // Port 9 may need rights on this system; then just check refusal.
        assert!(wake([1; 6], Some(Ipv4Addr::new(8, 8, 8, 8))).await.is_err());
        if let Ok(rx) = rx {
            wake([1; 6], Some(Ipv4Addr::LOCALHOST)).await.expect("send");
            let mut buf = [0u8; 200];
            let n = rx.recv(&mut buf).await.expect("recv");
            assert_eq!(&buf[..n], &magic_packet([1; 6])[..]);
        }
    }

    #[tokio::test]
    async fn open_refused_and_summary() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let open = listener.local_addr().expect("addr").port();
        let tries = check_port("127.0.0.1", open, 3, Duration::from_secs(2))
            .await
            .expect("check");
        assert_eq!(tries.len(), 3);
        assert!(tries.iter().all(|a| matches!(a, Attempt::Open { .. })));
        let s = summarize(&tries);
        assert_eq!((s.sent, s.open), (3, 3));
        assert!(s.min_ms <= s.avg_ms && s.avg_ms <= s.max_ms);

        drop(listener);
        // Windows retries a refused connection for about two seconds before
        // reporting it, so give the refusal more time than that.
        let tries = check_port("127.0.0.1", open, 1, Duration::from_secs(8))
            .await
            .expect("check");
        assert!(matches!(tries[0], Attempt::Refused { .. }), "{tries:?}");
        assert_eq!(summarize(&tries).avg_ms, None);
        // Attempts and timeouts are capped.
        assert!(check_port("bad host", 1, 1, Duration::from_secs(1))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn lookups() {
        assert_eq!(
            lookup("127.0.0.1").await,
            Ok(vec![IpAddr::V4(Ipv4Addr::LOCALHOST)])
        );
        let local = lookup("localhost").await.expect("localhost");
        assert!(local.iter().all(|ip| ip.is_loopback()), "{local:?}");
        assert!(lookup("no such;name").await.is_err());
    }
}
