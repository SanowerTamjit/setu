//! Tunnels: saved port forwards (`tunnels.toml`) that run over a saved
//! host's SSH login, with live status and reconnect after a drop.
//!
//! The web view sends only a tunnel id to start: the tunnel is read back
//! from disk and checked again here, and its host must be a saved host.
//! Like workflow runs, tunnels never prompt: host keys must already be
//! trusted, and passwords or passphrases come from the vault only.

use crate::hosts::Hosts;
use crate::ssh::{check_access, connect_client, ConnectFailure, ConnectRequest, Login};
use crate::vault::VaultState;
use serde::{Deserialize, Serialize};
use setu_core::extend::{AuditAction, AuditEvent, AuditOutcome, Extensions};
use setu_core::paths::{self, Os, ProcessEnv};
use setu_hosts::tunnels::{Tunnel, TunnelKind, TunnelList, TunnelStore};
use setu_hosts::Host;
use setu_ssh::tunnel::{Forwarding, TunnelStats};
use setu_ssh::SshClient;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::State;
use tokio::sync::watch;

/// Waits between reconnect attempts after a drop: 2 s, 4 s, … up to 30 s.
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// Reconnect attempts after a drop before giving up (about a quarter hour).
const MAX_RECONNECTS: u32 = 30;
/// How often a running tunnel checks that its connection is still up.
const WATCH_EVERY: Duration = Duration::from_millis(500);

/// Where a tunnel is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Stopped,
    Connecting,
    Running,
    /// The connection dropped; trying again.
    Reconnecting,
    /// Stopped by an error that needs the person (or after too many tries).
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelStatus {
    phase: Phase,
    /// What went wrong, or what is being waited for. Never a secret.
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    /// `address:port` it listens on while running.
    #[serde(skip_serializing_if = "Option::is_none")]
    listening: Option<String>,
    active: usize,
    total: u64,
    failed: u64,
    /// Reconnects since it was started.
    reconnects: u32,
}

impl TunnelStatus {
    fn stopped() -> Self {
        TunnelStatus {
            phase: Phase::Stopped,
            message: None,
            listening: None,
            active: 0,
            total: 0,
            failed: 0,
            reconnects: 0,
        }
    }
}

/// One started tunnel: its supervisor task and what it reports.
struct Running {
    stop: watch::Sender<bool>,
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    status: Mutex<Option<TunnelStatus>>,
    stats: Mutex<Option<Arc<TunnelStats>>>,
}

impl Shared {
    fn set(&self, f: impl FnOnce(&mut TunnelStatus)) {
        if let Ok(mut slot) = self.status.lock() {
            let status = slot.get_or_insert_with(TunnelStatus::stopped);
            f(status);
        }
    }

    /// The status with live connection counts.
    fn snapshot(&self) -> TunnelStatus {
        let mut status = self
            .status
            .lock()
            .ok()
            .and_then(|s| s.clone())
            .unwrap_or_else(TunnelStatus::stopped);
        if let Some(stats) = self.stats.lock().ok().and_then(|s| s.clone()) {
            status.active = stats.active();
            status.total = stats.total();
            status.failed = stats.failed();
        }
        status
    }
}

pub struct Tunnels {
    store: Result<TunnelStore, String>,
    running: Mutex<HashMap<String, Running>>,
    /// Serialises read-modify-write of the file.
    file: Mutex<()>,
}

impl std::fmt::Debug for Tunnels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tunnels").finish_non_exhaustive()
    }
}

impl Tunnels {
    pub fn from_env() -> Self {
        let store = paths::config_dir(&ProcessEnv, Os::current())
            .map(|dir| TunnelStore::in_dir(&dir))
            .map_err(|e| e.to_string());
        Self::with_store(store)
    }

    fn with_store(store: Result<TunnelStore, String>) -> Self {
        Tunnels {
            store,
            running: Mutex::new(HashMap::new()),
            file: Mutex::new(()),
        }
    }

    fn store(&self) -> Result<&TunnelStore, String> {
        self.store.as_ref().map_err(Clone::clone)
    }

    fn load(&self) -> Result<TunnelList, String> {
        let _guard = self.file.lock().map_err(|_| "tunnels lock poisoned")?;
        self.store()?.load()
    }

    fn change(&self, f: impl FnOnce(&mut TunnelList) -> Result<(), String>) -> Result<(), String> {
        let _guard = self.file.lock().map_err(|_| "tunnels lock poisoned")?;
        let store = self.store()?;
        let mut list = store.load()?;
        f(&mut list)?;
        store.save(&list)
    }

    fn status(&self, id: &str) -> TunnelStatus {
        self.running
            .lock()
            .ok()
            .and_then(|r| r.get(id).map(|t| t.shared.snapshot()))
            .unwrap_or_else(TunnelStatus::stopped)
    }

    fn is_live(&self, id: &str) -> bool {
        matches!(
            self.status(id).phase,
            Phase::Connecting | Phase::Running | Phase::Reconnecting
        )
    }

    /// Stops a tunnel and removes it from the file.
    fn delete(&self, id: &str) -> Result<(), String> {
        self.stop(id);
        self.change(|list| {
            if list.remove(id) {
                Ok(())
            } else {
                Err("that tunnel is not saved".into())
            }
        })
    }

    fn stop(&self, id: &str) {
        if let Some(running) = self.running.lock().ok().and_then(|mut r| r.remove(id)) {
            let _ = running.stop.send(true);
        }
    }

    /// Starts the saved tunnel `id` (read from disk) if it is not running.
    fn start(
        &self,
        id: &str,
        hosts: &Hosts,
        vault: &VaultState,
        extensions: &Extensions,
    ) -> Result<(), String> {
        if self.is_live(id) {
            return Ok(());
        }
        let list = self.load()?;
        let tunnel = list.get(id).cloned().ok_or("that tunnel is not saved")?;
        let host = hosts
            .inventory()?
            .hosts
            .into_iter()
            .find(|h| h.name == tunnel.host)
            .ok_or_else(|| {
                format!(
                    "the saved host {:?} is gone; edit the tunnel to pick another",
                    tunnel.host
                )
            })?;
        let (stop, stopped) = watch::channel(false);
        let shared = Arc::new(Shared::default());
        shared.set(|s| s.phase = Phase::Connecting);
        let supervisor = Supervisor {
            tunnel,
            host,
            vault: vault.clone(),
            extensions: extensions.clone(),
            shared: Arc::clone(&shared),
            stopped,
        };
        tauri::async_runtime::spawn(supervisor.run());
        let mut running = self.running.lock().map_err(|_| "tunnels lock poisoned")?;
        if let Some(old) = running.insert(id.to_string(), Running { stop, shared }) {
            let _ = old.stop.send(true);
        }
        Ok(())
    }

    /// Starts every tunnel marked "start when Setu opens". Errors show up
    /// as each tunnel's status, not here.
    pub fn start_auto(&self, hosts: &Hosts, vault: &VaultState, extensions: &Extensions) {
        let Ok(list) = self.load() else { return };
        for t in list.tunnels.iter().filter(|t| t.auto_start) {
            if let Err(message) = self.start(&t.id, hosts, vault, extensions) {
                self.failed(&t.id, message);
            }
        }
    }

    /// Records a tunnel that could not even be started.
    fn failed(&self, id: &str, message: String) {
        let shared = Arc::new(Shared::default());
        shared.set(|s| {
            s.phase = Phase::Failed;
            s.message = Some(message);
        });
        let (stop, _) = watch::channel(true);
        if let Ok(mut running) = self.running.lock() {
            running.insert(id.to_string(), Running { stop, shared });
        }
    }
}

/// Runs one tunnel: connect, forward, watch the connection, reconnect.
struct Supervisor {
    tunnel: Tunnel,
    host: Host,
    vault: VaultState,
    extensions: Extensions,
    shared: Arc<Shared>,
    stopped: watch::Receiver<bool>,
}

/// Why a connection attempt did not give a running forward.
enum Attempt {
    /// Stop trying: the person has to do something first.
    GiveUp(String),
    /// Worth trying again after a pause (network, server restarting).
    Retry(String),
}

impl Supervisor {
    async fn run(mut self) {
        // Failed tries in a row, and drops since it was started.
        let mut tries = 0u32;
        let mut reconnects = 0u32;
        let mut ever_ran = false;
        loop {
            if *self.stopped.borrow() {
                break;
            }
            match self.connect_and_forward().await {
                Ok((client, forwarding)) => {
                    ever_ran = true;
                    tries = 0;
                    let listening = forwarding.listening().to_string();
                    if let Ok(mut s) = self.shared.stats.lock() {
                        *s = Some(Arc::clone(forwarding.stats()));
                    }
                    self.shared.set(|s| {
                        s.phase = Phase::Running;
                        s.message = None;
                        s.listening = Some(listening);
                        s.reconnects = reconnects;
                    });
                    self.audit(AuditOutcome::Succeeded, "");
                    let dropped = self.watch(&client).await;
                    forwarding.stop().await;
                    let _ = client.disconnect().await;
                    if !dropped {
                        break;
                    }
                    reconnects += 1;
                    self.shared.set(|s| {
                        s.phase = Phase::Reconnecting;
                        s.message = Some("The connection dropped. Reconnecting…".into());
                        s.listening = None;
                    });
                }
                Err(Attempt::GiveUp(message)) => {
                    self.audit(AuditOutcome::Failed, "could not start");
                    self.fail(message);
                    return;
                }
                Err(Attempt::Retry(message)) => {
                    // The first start fails plainly; only a tunnel that was
                    // running keeps trying after a drop.
                    if !ever_ran || tries >= MAX_RECONNECTS {
                        self.audit(AuditOutcome::Failed, "could not connect");
                        self.fail(message);
                        return;
                    }
                    self.shared.set(|s| {
                        s.phase = Phase::Reconnecting;
                        s.message = Some(format!("{message}. Trying again…"));
                    });
                }
            }
            tries += 1;
            if !self.pause(backoff(tries)).await {
                break;
            }
            self.shared.set(|s| s.phase = Phase::Connecting);
        }
        self.shared.set(|s| {
            s.phase = Phase::Stopped;
            s.message = None;
            s.listening = None;
        });
    }

    async fn connect_and_forward(&self) -> Result<(Arc<SshClient>, Forwarding), Attempt> {
        check_access(&self.extensions, Login::for_host(&self.host))
            .await
            .map_err(|e| Attempt::GiveUp(e.unattended_message()))?;
        let client = match connect_client(ConnectRequest::for_host(&self.host), &self.vault).await {
            Ok((client, _)) => Arc::new(client),
            Err(e @ ConnectFailure::Error { .. }) => {
                return Err(Attempt::Retry(e.unattended_message()))
            }
            Err(e) => return Err(Attempt::GiveUp(e.unattended_message())),
        };
        let t = &self.tunnel;
        let target_host = t.target_host.as_deref().unwrap_or_default();
        let target_port = t.target_port.unwrap_or_default();
        let forwarding = match t.kind {
            TunnelKind::Local => {
                client
                    .forward_local(&t.bind_address, t.bind_port, target_host, target_port)
                    .await
            }
            TunnelKind::Dynamic => client.forward_dynamic(&t.bind_address, t.bind_port).await,
            TunnelKind::Remote => {
                client
                    .forward_remote(&t.bind_address, t.bind_port, target_host, target_port)
                    .await
            }
        };
        match forwarding {
            Ok(f) => Ok((client, f)),
            Err(e) => {
                let _ = client.disconnect().await;
                // A port in use or a server that refuses forwarding will
                // not fix itself in a few seconds.
                Err(Attempt::GiveUp(e.to_string()))
            }
        }
    }

    /// Waits until the connection drops (true) or the tunnel is stopped (false).
    async fn watch(&mut self, client: &SshClient) -> bool {
        loop {
            if client.is_closed() {
                return true;
            }
            tokio::select! {
                _ = tokio::time::sleep(WATCH_EVERY) => {}
                changed = self.stopped.changed() => {
                    if changed.is_err() || *self.stopped.borrow() {
                        return false;
                    }
                }
            }
        }
    }

    /// Sleeps, unless stopped first (then false).
    async fn pause(&mut self, how_long: Duration) -> bool {
        tokio::select! {
            _ = tokio::time::sleep(how_long) => !*self.stopped.borrow(),
            changed = self.stopped.changed() => changed.is_ok() && !*self.stopped.borrow(),
        }
    }

    fn fail(&self, message: String) {
        self.shared.set(|s| {
            s.phase = Phase::Failed;
            s.message = Some(message);
            s.listening = None;
        });
    }

    fn audit(&self, outcome: AuditOutcome, detail: &str) {
        let user = self.host.user.as_deref().unwrap_or_default();
        let target = format!(
            "{}@{}:{}",
            if user.is_empty() { "-" } else { user },
            self.host.host.trim(),
            self.host.port
        );
        let mut what = self.tunnel.summary();
        if !detail.is_empty() {
            what = format!("{what} ({detail})");
        }
        self.extensions.audit(AuditEvent {
            action: AuditAction::TunnelOpen,
            outcome,
            target,
            detail: what,
        });
    }
}

fn backoff(attempt: u32) -> Duration {
    let secs = 2u64.saturating_pow(attempt.clamp(1, 5));
    Duration::from_secs(secs).min(MAX_BACKOFF)
}

// ---- Commands ----

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelView {
    #[serde(flatten)]
    tunnel: TunnelOut,
    status: TunnelStatus,
    /// `-L 5433:127.0.0.1:5432`, as OpenSSH writes it.
    summary: String,
    shared: bool,
}

/// A tunnel as the window sees and sends it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelOut {
    /// Empty for a new tunnel.
    #[serde(default)]
    id: String,
    name: String,
    host: String,
    kind: TunnelKind,
    bind_address: String,
    bind_port: u16,
    #[serde(default)]
    target_host: Option<String>,
    #[serde(default)]
    target_port: Option<u16>,
    #[serde(default)]
    auto_start: bool,
}

impl From<Tunnel> for TunnelOut {
    fn from(t: Tunnel) -> Self {
        TunnelOut {
            id: t.id,
            name: t.name,
            host: t.host,
            kind: t.kind,
            bind_address: t.bind_address,
            bind_port: t.bind_port,
            target_host: t.target_host,
            target_port: t.target_port,
            auto_start: t.auto_start,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelsView {
    tunnels: Vec<TunnelView>,
    /// Where the file lives.
    path: String,
    /// Set when the file could not be read (it is then never written over).
    #[serde(skip_serializing_if = "Option::is_none")]
    problem: Option<String>,
}

fn view(state: &Tunnels) -> TunnelsView {
    let path = state
        .store()
        .map(|s| s.path().display().to_string())
        .unwrap_or_default();
    match state.load() {
        Ok(list) => TunnelsView {
            tunnels: list
                .tunnels
                .into_iter()
                .map(|t| TunnelView {
                    status: state.status(&t.id),
                    summary: t.summary(),
                    shared: t.is_shared(),
                    tunnel: t.into(),
                })
                .collect(),
            path,
            problem: None,
        },
        Err(problem) => TunnelsView {
            tunnels: Vec::new(),
            path,
            problem: Some(problem),
        },
    }
}

#[tauri::command]
pub fn tunnels_list(state: State<'_, Tunnels>) -> TunnelsView {
    view(&state)
}

/// Saves a new or changed tunnel and returns its id. A running tunnel keeps
/// its old settings until it is restarted.
#[tauri::command]
pub fn tunnel_save(
    state: State<'_, Tunnels>,
    hosts: State<'_, Hosts>,
    tunnel: TunnelOut,
) -> Result<String, String> {
    save(&state, &hosts, tunnel)
}

fn save(state: &Tunnels, hosts: &Hosts, input: TunnelOut) -> Result<String, String> {
    let known = hosts.inventory()?;
    if !known.hosts.iter().any(|h| h.name == input.host) {
        return Err(format!("{:?} is not a saved host", input.host));
    }
    let mut id = String::new();
    state.change(|list| {
        id = if input.id.is_empty() {
            list.new_id(&input.name)
        } else if list.get(&input.id).is_some() {
            input.id.clone()
        } else {
            return Err("that tunnel is not saved".into());
        };
        let dynamic = input.kind == TunnelKind::Dynamic;
        list.upsert(Tunnel {
            id: id.clone(),
            name: input.name.trim().to_string(),
            host: input.host.clone(),
            kind: input.kind,
            bind_address: input.bind_address.trim().to_string(),
            bind_port: input.bind_port,
            target_host: if dynamic {
                None
            } else {
                input.target_host.as_deref().map(|h| h.trim().to_string())
            },
            target_port: if dynamic { None } else { input.target_port },
            auto_start: input.auto_start,
        })
    })?;
    Ok(id)
}

#[tauri::command]
pub fn tunnel_delete(state: State<'_, Tunnels>, id: String) -> Result<(), String> {
    state.delete(&id)
}

#[tauri::command]
pub fn tunnel_start(
    state: State<'_, Tunnels>,
    hosts: State<'_, Hosts>,
    vault: State<'_, VaultState>,
    extensions: State<'_, Extensions>,
    id: String,
) -> Result<(), String> {
    state.start(&id, &hosts, &vault, &extensions)
}

#[tauri::command]
pub fn tunnel_stop(state: State<'_, Tunnels>, id: String) {
    state.stop(&id);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tunnels(dir: &std::path::Path) -> Tunnels {
        Tunnels::with_store(Ok(TunnelStore::in_dir(dir)))
    }

    fn hosts(dir: &std::path::Path) -> Hosts {
        let store = setu_hosts::HostStore::in_dir(dir);
        let mut inv = setu_hosts::Inventory::default();
        inv.upsert(Host::new("db-01", "10.0.0.5"), None)
            .expect("add");
        store.save(&inv).expect("save");
        Hosts::with_store(store)
    }

    fn vault(dir: &std::path::Path) -> VaultState {
        VaultState::new(
            Ok(dir.join("vault.json")),
            Arc::new(setu_vault::keychain::MemoryKeyStore::default()),
        )
    }

    fn input(name: &str, port: u16) -> TunnelOut {
        TunnelOut {
            id: String::new(),
            name: name.into(),
            host: "db-01".into(),
            kind: TunnelKind::Local,
            bind_address: "127.0.0.1".into(),
            bind_port: port,
            target_host: Some(" 127.0.0.1 ".into()),
            target_port: Some(5432),
            auto_start: false,
        }
    }

    #[test]
    fn save_checks_the_host_and_makes_ids() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (state, hosts) = (tunnels(dir.path()), hosts(dir.path()));
        let id = save(&state, &hosts, input("Postgres", 5433)).expect("save");
        assert_eq!(id, "postgres");
        let mut other = input("Postgres", 5434);
        other.host = "nowhere".into();
        assert!(save(&state, &hosts, other)
            .err()
            .unwrap_or_default()
            .contains("not a saved host"));
        // Editing keeps the id; a made-up id is refused.
        let mut edit = input("Postgres (prod)", 5433);
        edit.id = id.clone();
        assert_eq!(save(&state, &hosts, edit).expect("edit"), id);
        let mut forged = input("X", 6000);
        forged.id = "../../etc".into();
        assert!(save(&state, &hosts, forged).is_err());

        let shown = view(&state);
        assert_eq!(shown.tunnels.len(), 1);
        assert_eq!(shown.tunnels[0].tunnel.name, "Postgres (prod)");
        assert_eq!(shown.tunnels[0].summary, "-L 5433:127.0.0.1:5432");
        assert_eq!(shown.tunnels[0].status.phase, Phase::Stopped);
    }

    #[test]
    fn a_socks_tunnel_drops_any_destination_sent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (state, hosts) = (tunnels(dir.path()), hosts(dir.path()));
        let mut socks = input("Browse via db", 1080);
        socks.kind = TunnelKind::Dynamic;
        let id = save(&state, &hosts, socks).expect("save");
        let list = state.load().expect("load");
        assert_eq!(list.get(&id).and_then(|t| t.target_host.clone()), None);
    }

    #[test]
    fn a_tunnel_on_a_missing_host_fails_with_a_reason() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (state, hosts) = (tunnels(dir.path()), hosts(dir.path()));
        let id = save(&state, &hosts, input("Postgres", 5433)).expect("save");
        // The host is renamed after the tunnel was saved.
        setu_hosts::HostStore::in_dir(dir.path())
            .save(&setu_hosts::Inventory::default())
            .expect("save");
        let err = state
            .start(&id, &hosts, &vault(dir.path()), &Extensions::default())
            .err()
            .unwrap_or_default();
        assert!(err.contains("is gone"), "{err}");
        state.failed(&id, err);
        assert_eq!(state.status(&id).phase, Phase::Failed);
        assert!(!state.is_live(&id));
        state.delete(&id).expect("delete");
        assert!(view(&state).tunnels.is_empty());
        assert!(state.delete(&id).is_err());
    }

    #[test]
    fn backoff_grows_then_caps() {
        assert_eq!(backoff(0), Duration::from_secs(2));
        assert_eq!(backoff(1), Duration::from_secs(2));
        assert_eq!(backoff(2), Duration::from_secs(4));
        assert_eq!(backoff(4), Duration::from_secs(16));
        assert_eq!(backoff(9), MAX_BACKOFF);
    }
}
