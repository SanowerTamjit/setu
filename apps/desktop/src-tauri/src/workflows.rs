//! Workflows: save and list them, run one on saved hosts over SSH, answer
//! its approval steps, stop it, and keep its history.
//!
//! The web view sends only a workflow id to run: the steps are read back
//! from disk and checked again here, and every target must be a saved host.
//! Connections never prompt: host keys must already be trusted, and
//! passwords or passphrases come from the vault only.

use crate::hosts::Hosts;
use crate::ssh::{connect_client, ConnectRequest};
use crate::vault::VaultState;
use serde::Serialize;
use setu_core::paths::{self, Os, ProcessEnv};
use setu_hosts::Host;
use setu_ssh::SshClient;
use setu_workflow::engine::{self, Event, ExecOutput, RunRecord, Runner, Targets, MAX_OUTPUT};
use setu_workflow::store::{Broken, WorkflowStore};
use setu_workflow::{resolve_targets, templates, Kind, Workflow};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tauri::ipc::Channel;
use tauri::State;
use tokio::sync::{oneshot, watch, OnceCell};

/// Runs going at once; more is almost certainly a mistake.
const MAX_ACTIVE: usize = 8;

type Approvals = Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>;

struct ActiveRun {
    cancel: watch::Sender<bool>,
    approvals: Approvals,
}

pub struct Workflows {
    store: Result<WorkflowStore, String>,
    active: Arc<Mutex<HashMap<String, ActiveRun>>>,
    counter: AtomicU32,
}

impl std::fmt::Debug for Workflows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workflows").finish_non_exhaustive()
    }
}

impl Workflows {
    pub fn from_env() -> Self {
        let store = paths::config_dir(&ProcessEnv, Os::current())
            .map(|dir| WorkflowStore::in_dir(&dir))
            .map_err(|e| e.to_string());
        Workflows {
            store,
            active: Arc::default(),
            counter: AtomicU32::new(0),
        }
    }

    fn store(&self) -> Result<&WorkflowStore, String> {
        self.store.as_ref().map_err(Clone::clone)
    }
}

#[derive(Debug, Serialize)]
pub struct WorkflowsView {
    workflows: Vec<Workflow>,
    broken: Vec<Broken>,
    templates: Vec<Workflow>,
    /// The folder, shown so people can back it up or keep it in Git.
    dir: String,
}

fn view(store: &WorkflowStore) -> Result<WorkflowsView, String> {
    let (workflows, broken) = store.list().map_err(|e| e.to_string())?;
    Ok(WorkflowsView {
        workflows,
        broken,
        templates: templates::all(),
        dir: store.dir().display().to_string(),
    })
}

#[tauri::command]
pub fn workflows_list(state: State<'_, Workflows>) -> Result<WorkflowsView, String> {
    view(state.store()?)
}

#[tauri::command]
pub fn workflow_save(
    state: State<'_, Workflows>,
    workflow: Workflow,
    previous: Option<String>,
) -> Result<WorkflowsView, String> {
    let store = state.store()?;
    store
        .save(&workflow, previous.as_deref())
        .map_err(|e| e.to_string())?;
    view(store)
}

#[tauri::command]
pub fn workflow_delete(state: State<'_, Workflows>, id: String) -> Result<WorkflowsView, String> {
    let store = state.store()?;
    store.delete(&id).map_err(|e| e.to_string())?;
    view(store)
}

#[tauri::command]
pub fn workflow_history(state: State<'_, Workflows>) -> Result<Vec<RunRecord>, String> {
    Ok(state.store()?.history())
}

#[tauri::command]
pub fn workflow_history_clear(state: State<'_, Workflows>) -> Result<(), String> {
    state.store()?.clear_history().map_err(|e| e.to_string())
}

/// Expands every command step's targets against the saved hosts.
fn targets_for(workflow: &Workflow, hosts: &[Host]) -> Result<Targets, String> {
    let index: Vec<(String, Vec<String>)> = hosts
        .iter()
        .map(|h| (h.name.clone(), h.tags.clone()))
        .collect();
    let mut out = HashMap::new();
    for step in workflow.steps.iter().filter(|s| s.kind == Kind::Command) {
        let resolved =
            resolve_targets(&step.targets, &index).map_err(|e| format!("{}: {e}", step.name))?;
        out.insert(step.id.clone(), resolved);
    }
    Ok(Targets(out))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartedRun {
    run_id: String,
    /// The hosts each command step will run on, for the run view.
    targets: HashMap<String, Vec<String>>,
}

/// Starts workflow `id` as saved on disk. Progress goes to `on_event`; the
/// finished run is added to history.
#[tauri::command]
pub fn workflow_run(
    state: State<'_, Workflows>,
    hosts: State<'_, Hosts>,
    vault: State<'_, VaultState>,
    id: String,
    on_event: Channel<Event>,
) -> Result<StartedRun, String> {
    let store = state.store()?.clone();
    let workflow = store.load(&id).map_err(|e| e.to_string())?;
    workflow.validate().map_err(|e| e.to_string())?;
    if workflow.steps.is_empty() {
        return Err("add a step before running the workflow".into());
    }
    let inventory = hosts.inventory()?;
    let targets = targets_for(&workflow, &inventory.hosts)?;

    let n = state.counter.fetch_add(1, Ordering::Relaxed);
    let run_id = format!("{}-{n}", engine::now_ms());
    let (cancel, cancel_rx) = watch::channel(false);
    let approvals: Approvals = Arc::default();
    {
        let mut active = state.active.lock().map_err(|_| "runs lock poisoned")?;
        if active.len() >= MAX_ACTIVE {
            return Err(format!(
                "{MAX_ACTIVE} workflows are already running; stop one first"
            ));
        }
        active.insert(
            run_id.clone(),
            ActiveRun {
                cancel,
                approvals: Arc::clone(&approvals),
            },
        );
    }
    let runner = Arc::new(SshRunner {
        hosts: inventory
            .hosts
            .into_iter()
            .map(|h| (h.name.clone(), h))
            .collect(),
        vault: vault.inner().clone(),
        connections: Mutex::default(),
        approvals,
    });
    let started = StartedRun {
        run_id: run_id.clone(),
        targets: targets.0.clone(),
    };
    let active = Arc::clone(&state.active);
    tauri::async_runtime::spawn(async move {
        let record = engine::run(
            run_id.clone(),
            workflow,
            targets,
            Arc::clone(&runner),
            cancel_rx,
            |e| {
                let _ = on_event.send(e.clone());
            },
        )
        .await;
        runner.disconnect_all().await;
        // History is best effort: the person saw the run live.
        let _ = store.record(&record);
        if let Ok(mut active) = active.lock() {
            active.remove(&run_id);
        }
    });
    Ok(started)
}

/// Approves or rejects the approval step `step` of a running workflow.
#[tauri::command]
pub fn workflow_answer(
    state: State<'_, Workflows>,
    run_id: String,
    step: String,
    approve: bool,
) -> Result<(), String> {
    let active = state.active.lock().map_err(|_| "runs lock poisoned")?;
    let run = active.get(&run_id).ok_or("that run has finished")?;
    let sender = run
        .approvals
        .lock()
        .map_err(|_| "approvals lock poisoned")?
        .remove(&step)
        .ok_or("that step is not waiting for approval")?;
    let _ = sender.send(approve);
    Ok(())
}

#[tauri::command]
pub fn workflow_cancel(state: State<'_, Workflows>, run_id: String) -> Result<(), String> {
    let active = state.active.lock().map_err(|_| "runs lock poisoned")?;
    if let Some(run) = active.get(&run_id) {
        let _ = run.cancel.send(true);
    }
    Ok(())
}

type Connection = Arc<OnceCell<Result<Arc<SshClient>, String>>>;

/// Runs steps over SSH, one connection per host for the whole run.
struct SshRunner {
    hosts: HashMap<String, Host>,
    vault: VaultState,
    connections: Mutex<HashMap<String, Connection>>,
    approvals: Approvals,
}

impl SshRunner {
    async fn client(&self, name: &str) -> Result<Arc<SshClient>, String> {
        let host = self
            .hosts
            .get(name)
            .ok_or_else(|| format!("there is no saved host called `{name}`"))?;
        let cell = {
            let mut map = self.connections.lock().map_err(|_| "lock poisoned")?;
            Arc::clone(map.entry(name.to_string()).or_default())
        };
        cell.get_or_init(|| async {
            connect_client(ConnectRequest::for_host(host), &self.vault)
                .await
                .map(|(client, _)| Arc::new(client))
                .map_err(|e| e.unattended_message())
        })
        .await
        .clone()
    }

    async fn disconnect_all(&self) {
        let cells: Vec<Connection> = match self.connections.lock() {
            Ok(mut map) => map.drain().map(|(_, c)| c).collect(),
            Err(_) => return,
        };
        for cell in cells {
            if let Some(Ok(client)) = cell.get() {
                let _ = client.disconnect().await;
            }
        }
    }
}

impl Runner for SshRunner {
    async fn exec(&self, host: &str, command: &str) -> Result<ExecOutput, String> {
        let client = self.client(host).await?;
        let (out, code) = client
            .run_command(command, MAX_OUTPUT)
            .await
            .map_err(|e| e.to_string())?;
        Ok(ExecOutput {
            output: plain_text(&String::from_utf8_lossy(&out)),
            code,
        })
    }

    async fn approve(&self, step: &str, _message: &str) -> bool {
        let (tx, rx) = oneshot::channel();
        match self.approvals.lock() {
            Ok(mut map) => {
                map.insert(step.to_string(), tx);
            }
            Err(_) => return false,
        }
        // A dropped sender (the run was stopped) counts as "no".
        rx.await.unwrap_or(false)
    }
}

/// Server output is shown as text in the run log: drop terminal escape
/// sequences and control characters other than newlines and tabs.
fn plain_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => {
                // CSI: ESC [ ... final byte in @..~ ; OSC: ESC ] ... BEL or ESC \
                match chars.peek() {
                    Some('[') => {
                        chars.next();
                        for c in chars.by_ref() {
                            if ('@'..='~').contains(&c) {
                                break;
                            }
                        }
                    }
                    Some(']') => {
                        chars.next();
                        while let Some(c) = chars.next() {
                            if c == '\u{7}' {
                                break;
                            }
                            if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                                chars.next();
                                break;
                            }
                        }
                    }
                    _ => {
                        chars.next();
                    }
                }
            }
            '\r' => {
                if chars.peek() != Some(&'\n') {
                    out.push('\n');
                }
            }
            '\n' | '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use setu_workflow::Step;

    #[test]
    fn plain_text_drops_escapes_and_controls() {
        assert_eq!(
            plain_text("\u{1b}[1;31mred\u{1b}[0m ok\r\n\u{1b}]0;title\u{7}x\u{0}y\u{8}"),
            "red ok\nxy"
        );
        assert_eq!(plain_text("50%\r100%\n"), "50%\n100%\n");
    }

    #[test]
    fn targets_must_be_saved_hosts() {
        let mut w = Workflow::new("w", "W");
        w.steps = vec![
            Step::command("a", "A", "uptime", &["tag:web"]),
            Step::approval("b", "B", ""),
        ];
        let mut web = Host::new("web-01", "10.0.0.1");
        web.tags = vec!["web".into()];
        let t = targets_for(&w, &[web, Host::new("db", "10.0.0.2")]).expect("targets");
        assert_eq!(t.0["a"], ["web-01"]);
        assert!(!t.0.contains_key("b"));
        w.steps[0].targets = vec!["evil; rm -rf /".into()];
        let err = targets_for(&w, &[]).expect_err("unknown host");
        assert!(err.contains("no saved host"), "{err}");
    }
}
