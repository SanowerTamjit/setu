//! Server management: a saved host's systemd services, their status and
//! logs, and changes to them (start, stop, restart, reload, enable,
//! disable).
//!
//! Every command run on the server is built in `setu_server::services`
//! from a checked unit name; the window never sends command text. A change
//! is two steps: `server_service_command` returns the exact command for the
//! window to show, and once the person agrees `server_service_action`
//! builds the same command again from the same action and unit, asks the
//! build's command policy, runs it and records it in the audit trail
//! (`server.change`, the command, never its output).
//!
//! Logins never prompt, like tunnels and monitoring. One connection per
//! host is kept for a few minutes so moving between services is quick.
//! `sudo` is only ever run with `-n`: Setu never types a password into it.

use crate::hosts::Hosts;
use crate::ssh::{check_access, connect_client, user_or_local, ConnectRequest, Login};
use crate::vault::VaultState;
use setu_core::extend::{
    AuditAction, AuditEvent, AuditOutcome, CommandCheck, CommandOrigin, Extensions, PolicyDecision,
};
use setu_hosts::Host;
use setu_server::services::{
    self, ActionResult, LogLevel, ServiceAction, ServiceDetails, ServiceList, ServiceLogs,
};
use setu_ssh::SshClient;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::State;
use tokio::sync::Mutex;

/// A kept connection is closed after this long unused.
const IDLE: Duration = Duration::from_secs(5 * 60);
/// Most connections kept at once.
const MAX_KEPT: usize = 8;
/// Longest a read (list, status, logs) may take.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest a change may take: systemd waits for a service to start.
const ACTION_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Default)]
pub struct ServerSessions {
    kept: Mutex<HashMap<String, (Arc<SshClient>, Instant)>>,
}

fn saved_host(hosts: &Hosts, name: &str) -> Result<Host, String> {
    hosts
        .inventory()?
        .hosts
        .into_iter()
        .find(|h| h.name == name)
        .ok_or_else(|| format!("{name:?} is not a saved host"))
}

impl ServerSessions {
    /// A connection to `host`: the kept one if it is still open, else a new
    /// login (asking the access policy first).
    async fn client(
        &self,
        host: &Host,
        vault: &VaultState,
        extensions: &Extensions,
        fresh: bool,
    ) -> Result<Arc<SshClient>, String> {
        let mut kept = self.kept.lock().await;
        let now = Instant::now();
        kept.retain(|name, (client, used)| {
            let keep = !client.is_closed()
                && now.duration_since(*used) < IDLE
                && !(fresh && *name == host.name);
            if !keep {
                let client = Arc::clone(client);
                tauri::async_runtime::spawn(async move {
                    let _ = client.disconnect().await;
                });
            }
            keep
        });
        if let Some((client, used)) = kept.get_mut(&host.name) {
            *used = now;
            return Ok(Arc::clone(client));
        }
        check_access(extensions, Login::for_host(host))
            .await
            .map_err(|e| e.unattended_message())?;
        let (client, _) = connect_client(ConnectRequest::for_host(host), vault)
            .await
            .map_err(|e| e.unattended_message())?;
        let client = Arc::new(client);
        if kept.len() >= MAX_KEPT {
            if let Some(oldest) = kept
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(n, _)| n.clone())
            {
                if let Some((c, _)) = kept.remove(&oldest) {
                    tauri::async_runtime::spawn(async move {
                        let _ = c.disconnect().await;
                    });
                }
            }
        }
        kept.insert(host.name.clone(), (Arc::clone(&client), now));
        Ok(client)
    }

    /// Runs `command` on `host`, logging in again once if the kept
    /// connection had dropped.
    async fn run(
        &self,
        host: &Host,
        vault: &VaultState,
        extensions: &Extensions,
        command: &str,
        max: usize,
        timeout: Duration,
    ) -> Result<(String, Option<u32>), String> {
        let mut fresh = false;
        loop {
            let client = self.client(host, vault, extensions, fresh).await?;
            match tokio::time::timeout(timeout, client.run_command(command, max)).await {
                Ok(Ok((out, code))) => {
                    return Ok((String::from_utf8_lossy(&out).into_owned(), code))
                }
                Ok(Err(_)) if client.is_closed() && !fresh => fresh = true,
                Ok(Err(e)) => return Err(e.to_string()),
                Err(_) => return Err(format!("{} took too long to answer", host.name)),
            }
        }
    }
}

/// The saved host's services: loaded ones and installed ones.
#[tauri::command]
pub async fn server_services(
    sessions: State<'_, ServerSessions>,
    hosts: State<'_, Hosts>,
    vault: State<'_, VaultState>,
    extensions: State<'_, Extensions>,
    host: String,
) -> Result<ServiceList, String> {
    let host = saved_host(&hosts, &host)?;
    let (out, _) = sessions
        .run(
            &host,
            &vault,
            &extensions,
            services::LIST_COMMAND,
            services::MAX_LIST_OUTPUT,
            READ_TIMEOUT,
        )
        .await?;
    services::parse_list(&out)
}

/// One service's status.
#[tauri::command]
pub async fn server_service_details(
    sessions: State<'_, ServerSessions>,
    hosts: State<'_, Hosts>,
    vault: State<'_, VaultState>,
    extensions: State<'_, Extensions>,
    host: String,
    unit: String,
    sudo: bool,
) -> Result<ServiceDetails, String> {
    let host = saved_host(&hosts, &host)?;
    let unit = services::check_unit(&unit)?;
    let command = services::details_command(unit, sudo)?;
    let (out, code) = sessions
        .run(
            &host,
            &vault,
            &extensions,
            &command,
            services::MAX_DETAILS_OUTPUT,
            READ_TIMEOUT,
        )
        .await?;
    services::parse_details(unit, &out).map_err(|said| match code {
        Some(0) => said,
        _ => services::explain(&said, sudo).unwrap_or(said),
    })
}

/// A service's newest journal lines. `level` is "all", "warnings" or
/// "errors"; `lines` is clamped.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn server_service_logs(
    sessions: State<'_, ServerSessions>,
    hosts: State<'_, Hosts>,
    vault: State<'_, VaultState>,
    extensions: State<'_, Extensions>,
    host: String,
    unit: String,
    lines: u32,
    level: String,
    sudo: bool,
) -> Result<ServiceLogs, String> {
    let host = saved_host(&hosts, &host)?;
    let command = services::logs_command(&unit, lines, LogLevel::parse(&level), sudo)?;
    let (out, _) = sessions
        .run(
            &host,
            &vault,
            &extensions,
            &command,
            services::MAX_LOG_OUTPUT * 2,
            READ_TIMEOUT,
        )
        .await?;
    // Show the command without the environment prefix, as a person would type it.
    let shown = command
        .split_once("; ")
        .map_or(command.as_str(), |(_, c)| c)
        .trim_end_matches(" 2>&1")
        .to_string();
    Ok(services::clean_logs(&out, shown))
}

/// The exact command a change would run, for the window to show before
/// asking. Nothing is run.
#[tauri::command]
pub fn server_service_command(action: String, unit: String, sudo: bool) -> Result<String, String> {
    services::action_command(ServiceAction::parse(&action)?, &unit, sudo)
}

/// Makes a change the person agreed to. The command is built again here
/// from `action` and `unit`, exactly as `server_service_command` showed it.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn server_service_action(
    sessions: State<'_, ServerSessions>,
    hosts: State<'_, Hosts>,
    vault: State<'_, VaultState>,
    extensions: State<'_, Extensions>,
    host: String,
    action: String,
    unit: String,
    sudo: bool,
) -> Result<ActionResult, String> {
    let host = saved_host(&hosts, &host)?;
    let command = services::action_command(ServiceAction::parse(&action)?, &unit, sudo)?;
    let audit = |outcome: AuditOutcome| {
        extensions.audit(AuditEvent {
            action: AuditAction::ServerChange,
            outcome,
            target: target(&host),
            detail: command.clone(),
        });
    };
    let check = CommandCheck {
        host: &host.name,
        command: &command,
        origin: CommandOrigin::Server { tool: "services" },
    };
    if let PolicyDecision::Deny { reason } = extensions.check_command(&check) {
        return Err(format!(
            "Your organization's policy does not allow this: {reason}"
        ));
    }
    match sessions
        .run(
            &host,
            &vault,
            &extensions,
            &command,
            services::MAX_ACTION_OUTPUT,
            ACTION_TIMEOUT,
        )
        .await
    {
        Ok((out, code)) => {
            let result = services::action_result(command.clone(), &out, code, sudo);
            audit(if result.ok {
                AuditOutcome::Succeeded
            } else {
                AuditOutcome::Failed
            });
            Ok(result)
        }
        Err(e) => {
            audit(AuditOutcome::Failed);
            Err(e)
        }
    }
}

/// Closes the kept connection to `host`, if any (the view moved on).
#[tauri::command]
pub async fn server_disconnect(
    sessions: State<'_, ServerSessions>,
    host: String,
) -> Result<(), String> {
    if let Some((client, _)) = sessions.kept.lock().await.remove(&host) {
        let _ = client.disconnect().await;
    }
    Ok(())
}

/// `user@host:port`, as the other audit records write it.
fn target(host: &Host) -> String {
    format!(
        "{}@{}:{}",
        user_or_local(host.user.as_deref().unwrap_or_default()),
        host.host.trim(),
        host.port
    )
}
