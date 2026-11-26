//! Monitoring: live CPU, memory, disk and network for a saved host, with
//! nothing installed on the server. Each monitor keeps one SSH connection
//! and runs `setu_monitor::COMMAND` (fixed, read-only, no input) every few
//! seconds; the output is parsed in Rust and only numbers (and cleaned
//! mount points) go to the window, over a channel.
//!
//! Logins never prompt, like tunnels and workflow runs. A monitor stops
//! when the window asks, when the window goes away, or after the
//! connection has failed too many times in a row.
//!
//! A reading over a limit becomes an alert once it has lasted the hold
//! time (`setu_monitor::alerts`). When one starts or clears, Rust sends the
//! desktop notification itself, so the window needs no permission to post
//! notifications.

use crate::hosts::Hosts;
use crate::ssh::{check_access, connect_client, ConnectFailure, ConnectRequest, Login};
use crate::vault::VaultState;
use serde::Serialize;
use setu_core::extend::Extensions;
use setu_hosts::Host;
use setu_monitor::alerts::{cleared_notice, started_notice, ActiveAlert, AlertTracker};
use setu_monitor::{Sample, Snapshot, Thresholds};
use setu_ssh::SshClient;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::watch;

/// Most monitors open at once.
const MAX_MONITORS: usize = 8;
/// Failed samples or logins in a row before a monitor gives up.
const MAX_FAILURES: u32 = 10;
/// Longest a single sample may take.
const SAMPLE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Default)]
pub struct Monitors {
    running: Mutex<HashMap<String, watch::Sender<bool>>>,
    next: AtomicU64,
}

/// What a monitor tells the window.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MonitorEvent {
    Connecting,
    Sample {
        snapshot: Box<Snapshot>,
        /// Readings over a limit, and whether each has lasted the hold time.
        active: Vec<ActiveAlert>,
    },
    /// Something went wrong; the monitor tries again unless `stopped`.
    Problem {
        message: String,
        stopped: bool,
    },
    Stopped,
}

/// Starts watching the saved host `host`; returns an id for `monitor_stop`.
/// `interval` (seconds) is clamped to 2-60, thresholds to 1-100 %, `hold`
/// (seconds a reading must stay over a limit) to 0-3600. With `notify`,
/// alerts that start or clear show a desktop notification.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn monitor_start(
    app: AppHandle,
    monitors: State<'_, Monitors>,
    hosts: State<'_, Hosts>,
    vault: State<'_, VaultState>,
    extensions: State<'_, Extensions>,
    host: String,
    interval: u32,
    cpu: Option<f64>,
    mem: Option<f64>,
    disk: Option<f64>,
    hold: Option<u64>,
    notify: Option<bool>,
    on_event: Channel<MonitorEvent>,
) -> Result<String, String> {
    let host = hosts
        .inventory()?
        .hosts
        .into_iter()
        .find(|h| h.name == host)
        .ok_or_else(|| format!("{host:?} is not a saved host"))?;
    let limit = |v: Option<f64>, default: f64| {
        v.filter(|v| v.is_finite())
            .map_or(default, |v| v.clamp(1.0, 100.0))
    };
    let d = Thresholds::default();
    let limits = Thresholds {
        cpu: limit(cpu, d.cpu),
        mem: limit(mem, d.mem),
        disk: limit(disk, d.disk),
    };
    let mut running = monitors
        .running
        .lock()
        .map_err(|_| "monitors lock poisoned")?;
    running.retain(|_, stop| !stop.is_closed());
    if running.len() >= MAX_MONITORS {
        return Err(format!(
            "at most {MAX_MONITORS} hosts can be watched at once; stop one first"
        ));
    }
    let id = format!("m{}", monitors.next.fetch_add(1, Ordering::Relaxed));
    let (stop, stopped) = watch::channel(false);
    running.insert(id.clone(), stop);
    let watcher = Watcher {
        host,
        vault: vault.inner().clone(),
        extensions: extensions.inner().clone(),
        every: Duration::from_secs(u64::from(interval.clamp(2, 60))),
        limits,
        alerts: AlertTracker::new(hold.unwrap_or(0)),
        notify: notify.unwrap_or(false).then_some(app),
        events: on_event,
        stopped,
    };
    tauri::async_runtime::spawn(watcher.run());
    Ok(id)
}

#[tauri::command]
pub fn monitor_stop(monitors: State<'_, Monitors>, id: String) {
    if let Some(stop) = monitors.running.lock().ok().and_then(|mut r| r.remove(&id)) {
        let _ = stop.send(true);
    }
}

struct Watcher {
    host: Host,
    vault: VaultState,
    extensions: Extensions,
    every: Duration,
    limits: Thresholds,
    alerts: AlertTracker,
    /// Set when alerts should show desktop notifications.
    notify: Option<AppHandle>,
    events: Channel<MonitorEvent>,
    stopped: watch::Receiver<bool>,
}

impl Watcher {
    /// False once the window has gone away.
    fn send(&self, event: MonitorEvent) -> bool {
        self.events.send(event).is_ok()
    }

    /// Shows a desktop notification, if turned on. A desktop without a
    /// notification service is not an error worth stopping for.
    fn notice(&self, (title, body): (String, String)) {
        if let Some(app) = &self.notify {
            let _ = app.notification().builder().title(title).body(body).show();
        }
    }

    /// Waits `d`; false if asked to stop meanwhile.
    async fn pause(&mut self, d: Duration) -> bool {
        if *self.stopped.borrow() {
            return false;
        }
        tokio::select! {
            _ = tokio::time::sleep(d) => !*self.stopped.borrow(),
            _ = self.stopped.changed() => false,
        }
    }

    async fn connect(&self) -> Result<SshClient, (String, bool)> {
        check_access(&self.extensions, Login::for_host(&self.host))
            .await
            .map_err(|e| (e.unattended_message(), true))?;
        match connect_client(ConnectRequest::for_host(&self.host), &self.vault).await {
            Ok((client, _)) => Ok(client),
            Err(e @ ConnectFailure::Error { .. }) => Err((e.unattended_message(), false)),
            // Host key, vault or login problems will not fix themselves.
            Err(e) => Err((e.unattended_message(), true)),
        }
    }

    async fn run(mut self) {
        let started = Instant::now();
        let mut failures = 0u32;
        'connect: loop {
            if !self.send(MonitorEvent::Connecting) {
                break;
            }
            let client = match self.connect().await {
                Ok(c) => c,
                Err((message, give_up)) => {
                    failures += 1;
                    let stopped = give_up || failures >= MAX_FAILURES;
                    if !self.send(MonitorEvent::Problem { message, stopped }) || stopped {
                        return;
                    }
                    if !self.pause(Duration::from_secs(10)).await {
                        break;
                    }
                    continue;
                }
            };
            let mut prev: Option<(Sample, Instant)> = None;
            loop {
                let at = Instant::now();
                let result = tokio::time::timeout(
                    SAMPLE_TIMEOUT,
                    client.run_command(setu_monitor::COMMAND, setu_monitor::MAX_OUTPUT),
                )
                .await;
                let problem = match result {
                    Ok(Ok((out, _))) => match setu_monitor::parse(&String::from_utf8_lossy(&out)) {
                        Some(sample) => {
                            failures = 0;
                            let snapshot = setu_monitor::snapshot(
                                prev.as_ref().map(|(s, _)| s),
                                &sample,
                                prev.as_ref()
                                    .map_or(0.0, |(_, t)| at.duration_since(*t).as_secs_f64()),
                                self.limits,
                            );
                            prev = Some((sample, at));
                            let update = self
                                .alerts
                                .update(at.duration_since(started).as_secs(), &snapshot.alerts);
                            for a in &update.started {
                                self.notice(started_notice(&self.host.name, a));
                            }
                            for a in &update.cleared {
                                self.notice(cleared_notice(&self.host.name, a));
                            }
                            if !self.send(MonitorEvent::Sample {
                                snapshot: Box::new(snapshot),
                                active: update.active,
                            }) {
                                break 'connect;
                            }
                            None
                        }
                        None => Some((
                            format!(
                                "{} did not report any figures (monitoring needs a Linux server with /proc)",
                                self.host.name
                            ),
                            true,
                        )),
                    },
                    Ok(Err(e)) if client.is_closed() => Some((format!("the connection dropped ({e})"), false)),
                    Ok(Err(e)) => Some((e.to_string(), false)),
                    Err(_) => Some(("the server took too long to answer".to_string(), false)),
                };
                if let Some((message, give_up)) = problem {
                    failures += 1;
                    let stopped = give_up || failures >= MAX_FAILURES;
                    if !self.send(MonitorEvent::Problem { message, stopped }) || stopped {
                        let _ = client.disconnect().await;
                        return;
                    }
                    if client.is_closed() {
                        if !self.pause(Duration::from_secs(5)).await {
                            break 'connect;
                        }
                        continue 'connect;
                    }
                }
                if !self.pause(self.every.saturating_sub(at.elapsed())).await {
                    let _ = client.disconnect().await;
                    break 'connect;
                }
            }
        }
        let _ = self.send(MonitorEvent::Stopped);
    }
}
