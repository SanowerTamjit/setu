//! Problems: what needs attention across saved hosts, in one list.
//!
//! The list joins three things: the last health check of each host
//! (`setu_health::check`, one fixed read-only command run over SSH), the
//! alerts in force on hosts being watched in Monitoring, and workflows
//! whose latest run failed. Snoozed findings are kept aside until their
//! time (`problems.json`, see `setu_health::snooze`).
//!
//! A check never prompts: it logs in the way monitoring and workflow runs
//! do (vault or ssh-agent), asking the build's access policy first, and a
//! host that cannot be checked becomes a finding that says why. Results
//! live in memory only; nothing a server sent is written to disk.

use crate::hosts::Hosts;
use crate::monitor::Monitors;
use crate::ssh::{check_access, connect_client, ConnectRequest, Login};
use crate::vault::VaultState;
use crate::workflows::Workflows;
use serde::Serialize;
use setu_core::extend::Extensions;
use setu_core::paths::{self, Os, ProcessEnv};
use setu_health::check::{self, Limits};
use setu_health::snooze::{self, Snoozed, Snoozes};
use setu_health::{sources, Counts, Finding, Seen};
use setu_hosts::Host;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::ipc::Channel;
use tauri::State;
use tokio::sync::Semaphore;

/// Hosts checked at the same time.
const PARALLEL: usize = 6;
/// Most hosts in one check; more is refused rather than queued for long.
const MAX_HOSTS: usize = 500;
/// Longest a login plus the check may take on one host.
const HOST_TIMEOUT: Duration = Duration::from_secs(30);

/// The last check of one host.
#[derive(Debug, Clone)]
struct Report {
    at_ms: u64,
    findings: Vec<Finding>,
}

pub struct Problems {
    reports: Arc<Mutex<HashMap<String, Report>>>,
    seen: Mutex<Seen>,
    checking: AtomicBool,
    snoozes: Result<Snoozes, String>,
}

impl std::fmt::Debug for Problems {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Problems").finish_non_exhaustive()
    }
}

impl Problems {
    pub fn from_env() -> Self {
        Problems {
            reports: Arc::default(),
            seen: Mutex::default(),
            checking: AtomicBool::new(false),
            snoozes: paths::config_dir(&ProcessEnv, Os::current())
                .map(|dir| Snoozes::in_dir(&dir))
                .map_err(|e| e.to_string()),
        }
    }

    fn snoozes(&self) -> Result<&Snoozes, String> {
        self.snoozes.as_ref().map_err(Clone::clone)
    }
}

fn now_ms() -> u64 {
    setu_workflow::engine::now_ms()
}

/// What the Problems view shows.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProblemsView {
    findings: Vec<Finding>,
    snoozed: Vec<Snoozed>,
    counts: Counts,
    /// Saved hosts that have been checked since Setu started, and when the
    /// oldest of those checks was (to say "checked 5 minutes ago").
    checked: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    checked_at_ms: Option<u64>,
    checking: bool,
}

/// Everything in the list right now.
#[tauri::command]
pub fn problems_list(
    problems: State<'_, Problems>,
    hosts: State<'_, Hosts>,
    monitors: State<'_, Monitors>,
    workflows: State<'_, Workflows>,
) -> Result<ProblemsView, String> {
    // A host removed from the inventory takes its findings with it.
    let saved: Vec<String> = hosts
        .inventory()
        .map(|i| i.hosts.into_iter().map(|h| h.name).collect())
        .unwrap_or_default();
    let mut all: Vec<Finding> = Vec::new();
    let (checked, checked_at_ms) = {
        let mut reports = problems
            .reports
            .lock()
            .map_err(|_| "problems lock poisoned")?;
        reports.retain(|host, _| saved.contains(host));
        for r in reports.values() {
            all.extend(r.findings.iter().cloned());
        }
        (reports.len(), reports.values().map(|r| r.at_ms).min())
    };
    for (host, alerts) in monitors.alerts() {
        all.extend(sources::monitor(&host, &alerts));
    }
    all.extend(sources::workflows(&workflows.history()));
    let now = now_ms();
    {
        let mut seen = problems.seen.lock().map_err(|_| "problems lock poisoned")?;
        // Workflow findings carry the time the run finished; keep it.
        let given: HashMap<String, u64> = all
            .iter()
            .filter_map(|f| f.since_ms.map(|t| (f.id.clone(), t)))
            .collect();
        seen.stamp(&mut all, now);
        for f in &mut all {
            if let Some(t) = given.get(&f.id) {
                f.since_ms = Some(*t);
            }
        }
    }
    setu_health::sort(&mut all);
    let active = problems
        .snoozes()
        .map(|s| s.active(now))
        .unwrap_or_default();
    let (findings, snoozed) = snooze::split(all, &active);
    Ok(ProblemsView {
        counts: setu_health::counts(&findings),
        findings,
        snoozed,
        checked,
        checked_at_ms,
        checking: problems.checking.load(Ordering::Relaxed),
    })
}

/// Progress of a check, for the window.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CheckEvent {
    Started {
        total: usize,
    },
    /// One host is done; `problems` is how many findings it has.
    Host {
        host: String,
        done: usize,
        problems: usize,
        ok: bool,
    },
    Finished,
}

/// Checks `hosts` (saved host names), or every saved host when empty.
/// Returns when every host is done; progress comes over `on_event`.
#[tauri::command]
pub async fn problems_check(
    problems: State<'_, Problems>,
    hosts: State<'_, Hosts>,
    vault: State<'_, VaultState>,
    extensions: State<'_, Extensions>,
    names: Vec<String>,
    on_event: Channel<CheckEvent>,
) -> Result<(), String> {
    let inventory = hosts.inventory()?.hosts;
    let targets: Vec<Host> = if names.is_empty() {
        inventory
    } else {
        for n in &names {
            if !inventory.iter().any(|h| &h.name == n) {
                return Err(format!("{n:?} is not a saved host"));
            }
        }
        inventory
            .into_iter()
            .filter(|h| names.contains(&h.name))
            .collect()
    };
    if targets.len() > MAX_HOSTS {
        return Err(format!("at most {MAX_HOSTS} hosts can be checked at once"));
    }
    if problems.checking.swap(true, Ordering::AcqRel) {
        return Err("a check is already running".into());
    }
    let _busy = Busy(&problems.checking);
    let _ = on_event.send(CheckEvent::Started {
        total: targets.len(),
    });
    let gate = Arc::new(Semaphore::new(PARALLEL));
    let mut jobs = Vec::with_capacity(targets.len());
    for host in targets {
        let gate = Arc::clone(&gate);
        let vault = vault.inner().clone();
        let extensions = extensions.inner().clone();
        jobs.push(tauri::async_runtime::spawn(async move {
            let _permit = gate.acquire_owned().await;
            let findings =
                match tokio::time::timeout(HOST_TIMEOUT, check_host(&host, &vault, &extensions))
                    .await
                {
                    Ok(Ok(f)) => Ok(f),
                    Ok(Err(reason)) => Err(reason),
                    Err(_) => Err(format!(
                        "{} took more than {} seconds to answer",
                        host.name,
                        HOST_TIMEOUT.as_secs()
                    )),
                };
            (host.name, findings)
        }));
    }
    let mut done = 0;
    for job in jobs {
        let Ok((host, result)) = job.await else {
            continue;
        };
        done += 1;
        let ok = result.is_ok();
        let findings = result.unwrap_or_else(|reason| vec![check::unreachable(&host, &reason)]);
        let count = findings.len();
        if let Ok(mut reports) = problems.reports.lock() {
            reports.insert(
                host.clone(),
                Report {
                    at_ms: now_ms(),
                    findings,
                },
            );
        }
        let _ = on_event.send(CheckEvent::Host {
            host,
            done,
            problems: count,
            ok,
        });
    }
    let _ = on_event.send(CheckEvent::Finished);
    Ok(())
}

/// Marks the check as over however it ends, even if the window goes away.
struct Busy<'a>(&'a AtomicBool);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Logs in to `host` without prompting, runs the check and logs out.
async fn check_host(
    host: &Host,
    vault: &VaultState,
    extensions: &Extensions,
) -> Result<Vec<Finding>, String> {
    check_access(extensions, Login::for_host(host))
        .await
        .map_err(|e| e.unattended_message())?;
    let (client, _) = connect_client(ConnectRequest::for_host(host), vault)
        .await
        .map_err(|e| e.unattended_message())?;
    let result = client.run_command(check::COMMAND, check::MAX_OUTPUT).await;
    let _ = client.disconnect().await;
    let (out, _) = result.map_err(|e| e.to_string())?;
    let health = check::parse(&String::from_utf8_lossy(&out))?;
    Ok(check::findings(&host.name, &health, Limits::default()))
}

/// Hides a finding for `minutes`; returns when it shows again.
#[tauri::command]
pub fn problems_snooze(
    problems: State<'_, Problems>,
    id: String,
    minutes: u64,
) -> Result<u64, String> {
    problems.snoozes()?.snooze(&id, minutes, now_ms())
}

/// Shows a snoozed finding again.
#[tauri::command]
pub fn problems_unsnooze(problems: State<'_, Problems>, id: String) -> Result<(), String> {
    problems.snoozes()?.wake(&id, now_ms())
}
