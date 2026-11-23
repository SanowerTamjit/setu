//! Runs a workflow: starts every step the [`Plan`] allows, side by side,
//! reports progress as [`Event`]s and returns a [`RunRecord`] for history.
//!
//! The engine does not know about SSH. A [`Runner`] does the work: the
//! desktop app's runner opens SSH connections to saved hosts and asks the
//! person to approve; tests use a fake.

use crate::model::{Kind, Step, Workflow};
use crate::plan::{Decision, Plan, State};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;

/// Output kept per host, from the end: enough to see what went wrong.
pub const MAX_OUTPUT: usize = 64 * 1024;

/// What a command printed (stdout and stderr together) and how it ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecOutput {
    pub output: String,
    /// `None` when the server closed the channel without an exit status.
    pub code: Option<u32>,
}

/// Does the real work for the engine.
pub trait Runner: Send + Sync + 'static {
    /// Runs `command` on the saved host `host`. `Err` is for when it could
    /// not run at all (no connection, unknown host key), shown to the person.
    fn exec(
        &self,
        host: &str,
        command: &str,
    ) -> impl Future<Output = Result<ExecOutput, String>> + Send;

    /// Asks the person to approve step `step`; `true` goes on.
    fn approve(&self, step: &str, message: &str) -> impl Future<Output = bool> + Send;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostRecord {
    pub host: String,
    pub state: State,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<u32>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub output: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepRecord {
    pub step: String,
    pub name: String,
    pub state: State,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hosts: Vec<HostRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub id: String,
    pub workflow: String,
    pub workflow_name: String,
    pub status: RunStatus,
    pub started_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_ms: Option<u64>,
    pub steps: Vec<StepRecord>,
}

/// Progress, sent to the UI as it happens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum Event {
    #[serde(rename_all = "camelCase")]
    Step {
        step: String,
        state: State,
        #[serde(skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    Host {
        step: String,
        #[serde(flatten)]
        record: HostRecord,
    },
    Finished {
        status: RunStatus,
    },
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Keeps the last `max` bytes of `s`, on a character boundary.
pub fn tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &s[start..])
}

/// What the run needs besides the workflow: the hosts each command step
/// runs on, already expanded from names and tags.
#[derive(Debug, Clone, Default)]
pub struct Targets(pub HashMap<String, Vec<String>>);

enum Msg {
    Event(Event),
}

struct StepDone {
    id: String,
    state: State,
    note: Option<String>,
    hosts: Vec<HostRecord>,
}

/// Runs `workflow` to the end, or until `cancel` turns `true`.
pub async fn run<R: Runner>(
    run_id: String,
    workflow: Workflow,
    targets: Targets,
    runner: Arc<R>,
    mut cancel: watch::Receiver<bool>,
    mut on_event: impl FnMut(&Event) + Send,
) -> RunRecord {
    let plan = Plan::new(&workflow);
    let mut record = RunRecord {
        id: run_id,
        workflow: workflow.id.clone(),
        workflow_name: workflow.name.clone(),
        status: RunStatus::Running,
        started_ms: now_ms(),
        finished_ms: None,
        steps: workflow
            .steps
            .iter()
            .map(|s| StepRecord {
                step: s.id.clone(),
                name: s.name.clone(),
                state: State::Pending,
                started_ms: None,
                finished_ms: None,
                note: None,
                hosts: Vec::new(),
            })
            .collect(),
    };
    let mut states: HashMap<String, State> = HashMap::new();
    let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();
    let mut tasks: JoinSet<StepDone> = JoinSet::new();
    let steps: HashMap<String, Step> = workflow
        .steps
        .iter()
        .map(|s| (s.id.clone(), s.clone()))
        .collect();
    let mut cancelled = *cancel.borrow();

    let mut emit = |record: &mut RunRecord, e: Event| {
        apply(record, &e);
        on_event(&e);
    };

    loop {
        if !cancelled {
            // Apply decisions until nothing changes: a skip can unblock more.
            loop {
                let next = plan.next(&states);
                if next.is_empty() {
                    break;
                }
                for d in next {
                    match d {
                        Decision::Skip(id) => {
                            states.insert(id.clone(), State::Skipped);
                            emit(&mut record, step_event(&id, State::Skipped, None));
                        }
                        Decision::Run(id) => {
                            let Some(step) = steps.get(&id).cloned() else {
                                continue;
                            };
                            let state = if step.kind == Kind::Approval {
                                State::Waiting
                            } else {
                                State::Running
                            };
                            states.insert(id.clone(), state);
                            emit(&mut record, step_event(&id, state, None));
                            let hosts = targets.0.get(&id).cloned().unwrap_or_default();
                            tasks.spawn(run_step(step, hosts, Arc::clone(&runner), tx.clone()));
                        }
                    }
                }
            }
        }
        if tasks.is_empty() {
            break;
        }
        tokio::select! {
            Some(Msg::Event(e)) = rx.recv() => emit(&mut record, e),
            Some(done) = tasks.join_next() => {
                // Events sent before the step finished come first.
                while let Ok(Msg::Event(e)) = rx.try_recv() {
                    emit(&mut record, e);
                }
                if let Ok(done) = done {
                    states.insert(done.id.clone(), done.state);
                    if let Some(s) = record.steps.iter_mut().find(|s| s.step == done.id) {
                        s.hosts = done.hosts;
                    }
                    emit(&mut record, step_event(&done.id, done.state, done.note));
                }
            }
            changed = cancel.changed(), if !cancelled => {
                if changed.is_err() || *cancel.borrow() {
                    cancelled = true;
                    tasks.abort_all();
                }
            }
        }
    }

    if cancelled {
        for s in &workflow.steps {
            let state = states.get(&s.id).copied().unwrap_or(State::Pending);
            if !state.is_done() {
                states.insert(s.id.clone(), State::Cancelled);
                emit(&mut record, step_event(&s.id, State::Cancelled, None));
            }
        }
    }
    let failed = workflow
        .steps
        .iter()
        .any(|s| states.get(&s.id) == Some(&State::Failed) && !plan.handles_failure(&s.id));
    let status = if cancelled {
        RunStatus::Cancelled
    } else if failed {
        RunStatus::Failed
    } else {
        RunStatus::Succeeded
    };
    record.status = status;
    record.finished_ms = Some(now_ms());
    on_event(&Event::Finished { status });
    record
}

fn step_event(id: &str, state: State, note: Option<String>) -> Event {
    Event::Step {
        step: id.to_string(),
        state,
        note,
    }
}

/// Keeps the record in step with the events the UI sees.
fn apply(record: &mut RunRecord, e: &Event) {
    match e {
        Event::Step { step, state, note } => {
            if let Some(s) = record.steps.iter_mut().find(|s| &s.step == step) {
                if matches!(state, State::Running | State::Waiting) {
                    s.started_ms = Some(now_ms());
                } else if state.is_done() && s.started_ms.is_some() {
                    s.finished_ms = Some(now_ms());
                }
                s.state = *state;
                if note.is_some() {
                    s.note = note.clone();
                }
            }
        }
        Event::Host { step, record: h } => {
            if let Some(s) = record.steps.iter_mut().find(|s| &s.step == step) {
                match s.hosts.iter_mut().find(|x| x.host == h.host) {
                    Some(x) => *x = h.clone(),
                    None => s.hosts.push(h.clone()),
                }
            }
        }
        Event::Finished { .. } => {}
    }
}

async fn run_step<R: Runner>(
    step: Step,
    hosts: Vec<String>,
    runner: Arc<R>,
    tx: mpsc::UnboundedSender<Msg>,
) -> StepDone {
    let done = |state, note: Option<String>, hosts| StepDone {
        id: step.id.clone(),
        state,
        note,
        hosts,
    };
    match step.kind {
        Kind::Wait => {
            tokio::time::sleep(Duration::from_secs(u64::from(step.seconds))).await;
            done(State::Succeeded, None, Vec::new())
        }
        Kind::Approval => {
            if runner.approve(&step.id, &step.message).await {
                done(State::Succeeded, Some("Approved".into()), Vec::new())
            } else {
                done(State::Failed, Some("Rejected".into()), Vec::new())
            }
        }
        Kind::Command => {
            if hosts.is_empty() {
                return done(State::Failed, Some("No hosts to run on".into()), Vec::new());
            }
            let timeout = step
                .timeout_seconds
                .map(|s| Duration::from_secs(u64::from(s)));
            let mut results: Vec<HostRecord> = hosts
                .iter()
                .map(|h| HostRecord {
                    host: h.clone(),
                    state: State::Pending,
                    code: None,
                    output: String::new(),
                    error: None,
                })
                .collect();
            if step.one_at_a_time {
                let mut failed = false;
                for r in results.iter_mut() {
                    if failed {
                        r.state = State::Skipped;
                        let _ = tx.send(host_event(&step.id, r));
                        continue;
                    }
                    *r = run_host(&step, &r.host, timeout, &runner, &tx).await;
                    failed = r.state == State::Failed;
                }
            } else {
                let mut set = JoinSet::new();
                for (i, h) in hosts.iter().enumerate() {
                    let (step, h, runner, tx) =
                        (step.clone(), h.clone(), Arc::clone(&runner), tx.clone());
                    set.spawn(async move { (i, run_host(&step, &h, timeout, &runner, &tx).await) });
                }
                while let Some(joined) = set.join_next().await {
                    if let Ok((i, r)) = joined {
                        results[i] = r;
                    }
                }
            }
            let failed = results
                .iter()
                .filter(|r| r.state != State::Succeeded)
                .count();
            let note = (hosts.len() > 1 || failed > 0).then(|| {
                if failed == 0 {
                    format!("{} hosts OK", hosts.len())
                } else {
                    format!("{failed} of {} hosts failed", hosts.len())
                }
            });
            let state = if failed == 0 {
                State::Succeeded
            } else {
                State::Failed
            };
            done(state, note, results)
        }
    }
}

fn host_event(step: &str, r: &HostRecord) -> Msg {
    Msg::Event(Event::Host {
        step: step.to_string(),
        record: r.clone(),
    })
}

async fn run_host<R: Runner>(
    step: &Step,
    host: &str,
    timeout: Option<Duration>,
    runner: &Arc<R>,
    tx: &mpsc::UnboundedSender<Msg>,
) -> HostRecord {
    let mut r = HostRecord {
        host: host.to_string(),
        state: State::Running,
        code: None,
        output: String::new(),
        error: None,
    };
    let _ = tx.send(host_event(&step.id, &r));
    let exec = runner.exec(host, &step.command);
    let result = match timeout {
        Some(t) => match tokio::time::timeout(t, exec).await {
            Ok(r) => r,
            Err(_) => Err(format!("timed out after {} s", t.as_secs())),
        },
        None => exec.await,
    };
    match result {
        Ok(out) => {
            r.code = out.code;
            r.output = tail(&out.output, MAX_OUTPUT);
            r.state = if out.code == Some(0) {
                State::Succeeded
            } else {
                State::Failed
            };
            if out.code.is_none() {
                r.error = Some("the server closed the command without an exit status".into());
            }
        }
        Err(e) => {
            r.state = State::Failed;
            r.error = Some(e);
        }
    }
    let _ = tx.send(host_event(&step.id, &r));
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Link, When};
    use std::sync::Mutex;

    /// Commands `fail` exit 1, `slow` takes 10 s, `offline` hosts cannot connect.
    #[derive(Default)]
    struct Fake {
        log: Mutex<Vec<String>>,
        approve: bool,
    }

    impl Runner for Fake {
        async fn exec(&self, host: &str, command: &str) -> Result<ExecOutput, String> {
            self.log
                .lock()
                .expect("lock")
                .push(format!("{host}:{command}"));
            if host == "offline" {
                return Err("could not connect".into());
            }
            if command == "slow" {
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
            let code = if command == "fail" { 1 } else { 0 };
            Ok(ExecOutput {
                output: format!("{command} on {host}\n"),
                code: Some(code),
            })
        }

        async fn approve(&self, step: &str, _message: &str) -> bool {
            self.log
                .lock()
                .expect("lock")
                .push(format!("approve:{step}"));
            self.approve
        }
    }

    fn workflow(steps: Vec<Step>, links: Vec<Link>) -> (Workflow, Targets) {
        let mut w = Workflow::new("w", "W");
        let mut t = HashMap::new();
        for s in &steps {
            if s.kind == Kind::Command {
                t.insert(s.id.clone(), s.targets.clone());
            }
        }
        w.steps = steps;
        w.links = links;
        w.validate().expect("valid");
        (w, Targets(t))
    }

    async fn go(w: Workflow, t: Targets, fake: Fake) -> (RunRecord, Vec<Event>, Arc<Fake>) {
        let fake = Arc::new(fake);
        let (_tx, rx) = watch::channel(false);
        let mut events = Vec::new();
        let rec = run("r1".into(), w, t, Arc::clone(&fake), rx, |e| {
            events.push(e.clone())
        })
        .await;
        (rec, events, fake)
    }

    fn state(rec: &RunRecord, id: &str) -> State {
        rec.steps.iter().find(|s| s.step == id).expect("step").state
    }

    #[tokio::test]
    async fn runs_in_order_and_records_output() {
        let (w, t) = workflow(
            vec![
                Step::command("a", "A", "echo a", &["h1", "h2"]),
                Step::command("b", "B", "echo b", &["h1"]),
            ],
            vec![Link::new("a", "b", When::Success)],
        );
        let (rec, events, fake) = go(w, t, Fake::default()).await;
        assert_eq!(rec.status, RunStatus::Succeeded);
        let a = &rec.steps[0];
        assert_eq!(a.hosts.len(), 2);
        assert_eq!(a.hosts[1].output, "echo a on h2\n");
        assert_eq!(a.note.as_deref(), Some("2 hosts OK"));
        let log = fake.log.lock().expect("lock").clone();
        assert_eq!(log.last().map(String::as_str), Some("h1:echo b"));
        assert!(matches!(
            events.last(),
            Some(Event::Finished {
                status: RunStatus::Succeeded
            })
        ));
    }

    #[tokio::test]
    async fn failure_runs_the_failure_branch_and_the_run_still_passes() {
        let (w, t) = workflow(
            vec![
                Step::command("a", "A", "fail", &["h1"]),
                Step::command("ok", "OK", "echo ok", &["h1"]),
                Step::command("rollback", "Rollback", "echo undo", &["h1"]),
            ],
            vec![
                Link::new("a", "ok", When::Success),
                Link::new("a", "rollback", When::Failure),
            ],
        );
        let (rec, _, _) = go(w, t, Fake::default()).await;
        assert_eq!(state(&rec, "a"), State::Failed);
        assert_eq!(state(&rec, "ok"), State::Skipped);
        assert_eq!(state(&rec, "rollback"), State::Succeeded);
        assert_eq!(rec.status, RunStatus::Succeeded, "the failure was handled");
    }

    #[tokio::test]
    async fn an_unhandled_failure_fails_the_run() {
        let (w, t) = workflow(
            vec![
                Step::command("a", "A", "echo", &["h1", "offline"]),
                Step::command("b", "B", "echo", &["h1"]),
            ],
            vec![Link::new("a", "b", When::Success)],
        );
        let (rec, _, _) = go(w, t, Fake::default()).await;
        assert_eq!(rec.status, RunStatus::Failed);
        assert_eq!(rec.steps[0].note.as_deref(), Some("1 of 2 hosts failed"));
        assert_eq!(
            rec.steps[0].hosts[1].error.as_deref(),
            Some("could not connect")
        );
        assert_eq!(state(&rec, "b"), State::Skipped);
    }

    #[tokio::test]
    async fn one_at_a_time_stops_at_the_first_failure() {
        let mut s = Step::command("a", "A", "echo", &["h1", "offline", "h3"]);
        s.one_at_a_time = true;
        let (w, t) = workflow(vec![s], vec![]);
        let (rec, _, fake) = go(w, t, Fake::default()).await;
        let hosts: Vec<_> = rec.steps[0].hosts.iter().map(|h| h.state).collect();
        assert_eq!(hosts, [State::Succeeded, State::Failed, State::Skipped]);
        assert_eq!(fake.log.lock().expect("lock").len(), 2, "h3 never ran");
    }

    #[tokio::test]
    async fn rejecting_an_approval_fails_the_step() {
        let (w, t) = workflow(
            vec![
                Step::approval("ok", "OK?", "check the graphs"),
                Step::command("go", "Go", "echo", &["h1"]),
            ],
            vec![Link::new("ok", "go", When::Success)],
        );
        let (rec, events, _) = go(w, t, Fake::default()).await;
        assert_eq!(state(&rec, "ok"), State::Failed);
        assert_eq!(rec.steps[0].note.as_deref(), Some("Rejected"));
        assert_eq!(state(&rec, "go"), State::Skipped);
        assert!(events.contains(&step_event("ok", State::Waiting, None)));

        let (w, t) = workflow(
            vec![
                Step::approval("ok", "OK?", ""),
                Step::command("go", "Go", "echo", &["h1"]),
            ],
            vec![Link::new("ok", "go", When::Success)],
        );
        let fake = Fake {
            approve: true,
            ..Fake::default()
        };
        let (rec, _, _) = go(w, t, fake).await;
        assert_eq!(state(&rec, "go"), State::Succeeded);
    }

    #[tokio::test(start_paused = true)]
    async fn timeouts_fail_the_host() {
        let mut s = Step::command("a", "A", "slow", &["h1"]);
        s.timeout_seconds = Some(2);
        let (w, t) = workflow(vec![s], vec![]);
        let (rec, _, _) = go(w, t, Fake::default()).await;
        assert_eq!(
            rec.steps[0].hosts[0].error.as_deref(),
            Some("timed out after 2 s")
        );
        assert_eq!(rec.status, RunStatus::Failed);
    }

    #[tokio::test(start_paused = true)]
    async fn cancelling_stops_running_and_pending_steps() {
        let (w, t) = workflow(
            vec![
                Step::command("a", "A", "slow", &["h1"]),
                Step::wait("b", "B", 60),
                Step::command("c", "C", "echo", &["h1"]),
            ],
            vec![Link::new("a", "c", When::Success)],
        );
        let fake = Arc::new(Fake::default());
        let (tx, rx) = watch::channel(false);
        let handle = tokio::spawn(run("r".into(), w, t, Arc::clone(&fake), rx, |_| {}));
        tokio::time::sleep(Duration::from_secs(1)).await;
        tx.send(true).expect("send");
        let rec = handle.await.expect("join");
        assert_eq!(rec.status, RunStatus::Cancelled);
        assert!(rec.steps.iter().all(|s| s.state == State::Cancelled));
        assert_eq!(fake.log.lock().expect("lock").len(), 1);
    }

    #[test]
    fn tail_keeps_the_end_on_a_char_boundary() {
        assert_eq!(tail("abc", 5), "abc");
        assert_eq!(tail("abcdef", 3), "…def");
        let t = tail("ééé", 3);
        assert_eq!(t, "…é");
    }
}
