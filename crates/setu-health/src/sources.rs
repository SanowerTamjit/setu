//! Findings from the rest of Setu: monitoring alerts and workflow runs.

use crate::{Action, Finding, Severity, Source};
use setu_monitor::alerts::ActiveAlert;
use setu_workflow::engine::{RunRecord, RunStatus};
use setu_workflow::plan::State;
use std::collections::HashSet;

/// "3 min", "2 h 5 min".
fn minutes(secs: u64) -> String {
    let m = secs / 60;
    match m {
        0 => format!("{secs} s"),
        1..=59 => format!("{m} min"),
        _ if m.is_multiple_of(60) => format!("{} h", m / 60),
        _ => format!("{} h {} min", m / 60, m % 60),
    }
}

/// Monitoring alerts on `host` that have lasted their hold time. Pending
/// ones (over the limit, but not for long enough yet) are left out, the
/// same way they do not notify.
pub fn monitor(host: &str, alerts: &[ActiveAlert]) -> Vec<Finding> {
    alerts
        .iter()
        .filter(|a| a.firing)
        .map(|a| Finding {
            id: format!("monitor:{host}:{}", a.alert.key()),
            severity: Severity::Warning,
            source: Source::Monitor,
            host: Some(host.to_string()),
            title: a.alert.text(),
            detail: format!(
                "Over the limit set in Monitoring for {}. It clears by itself once the reading falls back under the limit.",
                minutes(a.secs)
            ),
            since_ms: None,
            action: Some(Action::Monitor {
                host: host.to_string(),
            }),
        })
        .collect()
}

/// The latest finished run of each workflow, when it failed. A failed run
/// followed by one that succeeded is fixed and not a problem any more. The
/// run id is part of the finding's id, so a new failure shows again even
/// if the last one was snoozed.
///
/// `runs` is the run history, newest first or in any order.
pub fn workflows(runs: &[RunRecord]) -> Vec<Finding> {
    let mut latest: Vec<&RunRecord> = runs
        .iter()
        .filter(|r| r.status != RunStatus::Running)
        .collect();
    latest.sort_by_key(|r| std::cmp::Reverse(r.started_ms));
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for run in latest {
        if !seen.insert(run.workflow.as_str()) || run.status != RunStatus::Failed {
            continue;
        }
        let step = run.steps.iter().find(|s| s.state == State::Failed);
        let failed_hosts: Vec<&str> = step
            .map(|s| {
                s.hosts
                    .iter()
                    .filter(|h| h.state == State::Failed)
                    .map(|h| h.host.as_str())
                    .collect()
            })
            .unwrap_or_default();
        let detail = match (step, failed_hosts.as_slice()) {
            (Some(s), []) => format!("The step “{}” failed.", s.name),
            (Some(s), [one]) => {
                let why = s
                    .hosts
                    .iter()
                    .find(|h| h.host == *one)
                    .and_then(|h| {
                        h.error
                            .clone()
                            .or_else(|| h.code.map(|c| format!("exit code {c}")))
                    })
                    .map(|w| format!(": {}", setu_server::field(&w, 200)))
                    .unwrap_or_default();
                format!("The step “{}” failed on {one}{why}.", s.name)
            }
            (Some(s), many) => format!(
                "The step “{}” failed on {} hosts: {}.",
                s.name,
                many.len(),
                many.join(", ")
            ),
            (None, _) => "The run failed.".to_string(),
        };
        out.push(Finding {
            id: format!("workflow:{}:{}", run.workflow, run.id),
            severity: Severity::Error,
            source: Source::Workflow,
            host: match failed_hosts.as_slice() {
                [one] => Some((*one).to_string()),
                _ => None,
            },
            title: format!("Workflow “{}” failed", run.workflow_name),
            detail: format!("{detail} Open it to see the output and run it again."),
            since_ms: run.finished_ms.or(Some(run.started_ms)),
            action: Some(Action::Workflow {
                workflow: run.workflow.clone(),
                run: run.id.clone(),
            }),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use setu_monitor::Alert;
    use setu_workflow::engine::{HostRecord, StepRecord};

    #[test]
    fn only_firing_alerts_count() {
        let alerts = [
            ActiveAlert {
                alert: Alert::Cpu { percent: 97.4 },
                secs: 185,
                firing: true,
            },
            ActiveAlert {
                alert: Alert::Memory { percent: 92.0 },
                secs: 10,
                firing: false,
            },
            ActiveAlert {
                alert: Alert::Disk {
                    mount: "/var".into(),
                    percent: 93.0,
                },
                secs: 7500,
                firing: true,
            },
        ];
        let f = monitor("web1", &alerts);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].id, "monitor:web1:cpu");
        assert_eq!(f[0].title, "CPU at 97%");
        assert!(f[0].detail.contains("for 3 min."));
        assert_eq!(f[1].id, "monitor:web1:disk:/var");
        assert!(f[1].detail.contains("for 2 h 5 min."));
        assert_eq!(
            f[1].action,
            Some(Action::Monitor {
                host: "web1".into()
            })
        );
    }

    fn run(id: &str, wf: &str, status: RunStatus, at: u64, steps: Vec<StepRecord>) -> RunRecord {
        RunRecord {
            id: id.into(),
            workflow: wf.into(),
            workflow_name: format!("Name of {wf}"),
            status,
            started_ms: at,
            finished_ms: Some(at + 10),
            steps,
        }
    }

    fn step(name: &str, state: State, hosts: &[(&str, State, Option<u32>)]) -> StepRecord {
        StepRecord {
            step: name.to_lowercase(),
            name: name.into(),
            state,
            started_ms: None,
            finished_ms: None,
            note: None,
            hosts: hosts
                .iter()
                .map(|(h, s, code)| HostRecord {
                    host: (*h).into(),
                    state: *s,
                    code: *code,
                    output: "secret output stays out".into(),
                    error: None,
                })
                .collect(),
        }
    }

    #[test]
    fn the_latest_failed_run_of_each_workflow_is_a_problem() {
        let runs = vec![
            run("r1", "deploy", RunStatus::Failed, 100, vec![]),
            run(
                "r2",
                "deploy",
                RunStatus::Failed,
                200,
                vec![
                    step(
                        "Check",
                        State::Succeeded,
                        &[("web1", State::Succeeded, Some(0))],
                    ),
                    step(
                        "Restart",
                        State::Failed,
                        &[
                            ("web1", State::Succeeded, Some(0)),
                            ("web2", State::Failed, Some(3)),
                        ],
                    ),
                ],
            ),
            run("r3", "backup", RunStatus::Failed, 150, vec![]),
            run("r4", "backup", RunStatus::Succeeded, 300, vec![]),
            run("r5", "patch", RunStatus::Running, 400, vec![]),
            run("r6", "patch", RunStatus::Cancelled, 350, vec![]),
        ];
        let f = workflows(&runs);
        assert_eq!(f.len(), 1, "backup was fixed, patch never failed");
        assert_eq!(f[0].id, "workflow:deploy:r2");
        assert_eq!(f[0].title, "Workflow “Name of deploy” failed");
        assert_eq!(f[0].host.as_deref(), Some("web2"));
        assert!(f[0]
            .detail
            .starts_with("The step “Restart” failed on web2: exit code 3."));
        assert!(
            !f[0].detail.contains("secret"),
            "output never goes in a finding"
        );
        assert_eq!(f[0].since_ms, Some(210));
    }

    #[test]
    fn several_failed_hosts_are_named() {
        let runs = vec![run(
            "r1",
            "deploy",
            RunStatus::Failed,
            1,
            vec![step(
                "Restart",
                State::Failed,
                &[
                    ("web1", State::Failed, Some(1)),
                    ("web2", State::Failed, None),
                ],
            )],
        )];
        let f = workflows(&runs);
        assert_eq!(f[0].host, None);
        assert!(f[0]
            .detail
            .starts_with("The step “Restart” failed on 2 hosts: web1, web2."));
    }
}
