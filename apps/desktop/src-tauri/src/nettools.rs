//! Network tools: port check (TCP ping), DNS lookup, ping, TLS certificate
//! check and Wake-on-LAN, from this computer or through a saved host.
//!
//! Every target is checked by `setu_net::check_target` (a host name or
//! address, nothing else) before it is used. From this computer nothing is
//! run: checks are made in Rust. Through a host, the port check opens and
//! closes a forwarding channel; DNS and ping run one fixed command on the
//! server (`getent ahosts NAME`, `ping -c 4 -W 2 NAME`) with the checked name,
//! and the output is shown as plain text. The TLS check makes its handshake
//! here, over a forwarding channel when it goes through a host, so the
//! server only carries bytes. Connections never prompt, like
//! tunnels and workflow runs.

use crate::hosts::Hosts;
use crate::ssh::{check_access, connect_client, ConnectRequest, Login};
use crate::vault::VaultState;
use crate::workflows::plain_text;
use serde::Serialize;
use setu_core::extend::Extensions;
use setu_net::Attempt;
use setu_ssh::SshClient;
use std::time::Duration;
use tauri::State;

/// Output kept from a command on a server.
const MAX_OUTPUT: usize = 16 * 1024;
const TIMEOUT: Duration = Duration::from_secs(5);

/// Logs in to the saved host `name` the unattended way.
async fn connect_saved(
    name: &str,
    hosts: &Hosts,
    vault: &VaultState,
    extensions: &Extensions,
) -> Result<SshClient, String> {
    let host = hosts
        .inventory()?
        .hosts
        .into_iter()
        .find(|h| h.name == name)
        .ok_or_else(|| format!("{name:?} is not a saved host"))?;
    check_access(extensions, Login::for_host(&host))
        .await
        .map_err(|e| e.unattended_message())?;
    connect_client(ConnectRequest::for_host(&host), vault)
        .await
        .map(|(client, _)| client)
        .map_err(|e| e.unattended_message())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortReport {
    attempts: Vec<Attempt>,
    summary: setu_net::Summary,
    /// Where the check ran from: "this computer" or a saved host's name.
    from: String,
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn net_check_port(
    hosts: State<'_, Hosts>,
    vault: State<'_, VaultState>,
    extensions: State<'_, Extensions>,
    host: String,
    port: u16,
    attempts: u32,
    through: Option<String>,
) -> Result<PortReport, String> {
    let target = setu_net::check_target(&host).map_err(|e| e.to_string())?;
    if port == 0 {
        return Err("the port must be from 1 to 65535".into());
    }
    let attempts = attempts.clamp(1, setu_net::MAX_ATTEMPTS);
    let (list, from) = match through.filter(|t| !t.is_empty()) {
        None => (
            setu_net::check_port(&target, port, attempts, TIMEOUT)
                .await
                .map_err(|e| e.to_string())?,
            "this computer".to_string(),
        ),
        Some(name) => {
            let client = connect_saved(&name, &hosts, &vault, &extensions).await?;
            let mut list = Vec::new();
            for i in 0..attempts {
                if i > 0 {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                list.push(match client.check_port(&target, port, TIMEOUT).await {
                    Ok(took) => Attempt::Open {
                        micros: u64::try_from(took.as_micros()).unwrap_or(u64::MAX),
                        address: format!("{target}:{port}"),
                    },
                    Err(e) => Attempt::Failed {
                        reason: e.to_string(),
                    },
                });
            }
            let _ = client.disconnect().await;
            (list, name)
        }
    };
    Ok(PortReport {
        summary: setu_net::summarize(&list),
        attempts: list,
        from,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupReport {
    addresses: Vec<String>,
    from: String,
}

#[tauri::command]
pub async fn net_lookup(
    hosts: State<'_, Hosts>,
    vault: State<'_, VaultState>,
    extensions: State<'_, Extensions>,
    name: String,
    through: Option<String>,
) -> Result<LookupReport, String> {
    let name = setu_net::check_target(&name).map_err(|e| e.to_string())?;
    match through.filter(|t| !t.is_empty()) {
        None => Ok(LookupReport {
            addresses: setu_net::lookup(&name)
                .await
                .map_err(|e| e.to_string())?
                .iter()
                .map(ToString::to_string)
                .collect(),
            from: "this computer".into(),
        }),
        Some(host) => {
            let client = connect_saved(&host, &hosts, &vault, &extensions).await?;
            let result = client
                .run_command(&format!("getent ahosts {name}"), MAX_OUTPUT)
                .await;
            let _ = client.disconnect().await;
            let (out, code) = result.map_err(|e| e.to_string())?;
            let addresses = parse_getent(&String::from_utf8_lossy(&out));
            if addresses.is_empty() {
                return Err(match code {
                    Some(2) => format!("{host} could not find {name}"),
                    _ => {
                        format!("{host} could not look up {name} (getent is needed on the server)")
                    }
                });
            }
            Ok(LookupReport {
                addresses,
                from: host,
            })
        }
    }
}

/// The distinct addresses in `getent ahosts` output, in order.
fn parse_getent(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in text.lines().filter_map(|l| l.split_whitespace().next()) {
        if word.parse::<std::net::IpAddr>().is_ok() && !out.iter().any(|a| a == word) {
            out.push(word.to_string());
        }
    }
    out
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PingReport {
    /// What the server printed, as plain text.
    output: String,
    ok: bool,
    from: String,
}

/// ICMP ping needs rights an app does not have, so it runs on a saved host.
#[tauri::command]
pub async fn net_ping(
    hosts: State<'_, Hosts>,
    vault: State<'_, VaultState>,
    extensions: State<'_, Extensions>,
    host: String,
    through: String,
) -> Result<PingReport, String> {
    let target = setu_net::check_target(&host).map_err(|e| e.to_string())?;
    let client = connect_saved(&through, &hosts, &vault, &extensions).await?;
    let result = client
        .run_command(&format!("ping -c 4 -W 2 {target}"), MAX_OUTPUT)
        .await;
    let _ = client.disconnect().await;
    let (out, code) = result.map_err(|e| e.to_string())?;
    Ok(PingReport {
        output: plain_text(&String::from_utf8_lossy(&out)),
        ok: code == Some(0),
        from: through,
    })
}

/// Looks at the TLS certificate of `host:port`, checked for `name` (the
/// host when empty) against this computer's trusted roots.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn net_tls(
    hosts: State<'_, Hosts>,
    vault: State<'_, VaultState>,
    extensions: State<'_, Extensions>,
    host: String,
    port: u16,
    name: Option<String>,
    through: Option<String>,
) -> Result<TlsResult, String> {
    let target = setu_net::check_target(&host).map_err(|e| e.to_string())?;
    if port == 0 {
        return Err("the port must be from 1 to 65535".into());
    }
    let name = match name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => setu_net::check_target(n).map_err(|e| e.to_string())?,
        None => target.clone(),
    };
    let (report, from) = match through.filter(|t| !t.is_empty()) {
        None => (
            setu_net::tls::check(&target, port, &name, TIMEOUT)
                .await
                .map_err(|e| e.to_string())?,
            "this computer".to_string(),
        ),
        Some(via) => {
            let client = connect_saved(&via, &hosts, &vault, &extensions).await?;
            let result = match client.open_tcp(&target, port, TIMEOUT).await {
                Ok(stream) => {
                    setu_net::tls::check_over(stream, &name, TIMEOUT, setu_net::tls::system_roots())
                        .await
                        .map_err(|e| e.to_string())
                }
                Err(e) => Err(e.to_string()),
            };
            let _ = client.disconnect().await;
            (result?, via)
        }
    };
    Ok(TlsResult { report, name, from })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlsResult {
    #[serde(flatten)]
    report: setu_net::tls::TlsReport,
    /// The name the certificate was checked for.
    name: String,
    from: String,
}

#[tauri::command]
pub async fn net_wake(mac: String, broadcast: Option<String>) -> Result<(), String> {
    let mac = setu_net::parse_mac(&mac).map_err(|e| e.to_string())?;
    let broadcast = match broadcast
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
    {
        None => None,
        Some(b) => Some(
            b.parse::<std::net::Ipv4Addr>()
                .map_err(|_| format!("{b:?} is not an IPv4 broadcast address"))?,
        ),
    };
    setu_net::wake(mac, broadcast)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn getent_output_is_read() {
        let text = "10.0.0.5        STREAM db.internal\n10.0.0.5        DGRAM  \nfd00::5 STREAM \nnot-an-ip x\n";
        assert_eq!(parse_getent(text), vec!["10.0.0.5", "fd00::5"]);
        assert!(parse_getent("").is_empty());
    }
}
