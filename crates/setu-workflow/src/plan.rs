//! Which steps can run next.
//!
//! A step with no incoming links starts right away. Any other step waits
//! until every step linking to it has finished, then:
//!
//! - links from **skipped** steps are ignored (so branches can join again
//!   after an if/else);
//! - if every remaining link *fires* (success link from a step that
//!   succeeded, failure link from one that failed, always link from either)
//!   and at least one does, the step runs;
//! - otherwise it is skipped, and that can skip the steps after it.

use crate::model::{When, Workflow};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    Pending,
    Running,
    /// An approval step waiting for a person.
    Waiting,
    Succeeded,
    Failed,
    Skipped,
    /// The run was stopped before or while the step ran.
    Cancelled,
}

impl State {
    pub fn is_done(self) -> bool {
        matches!(
            self,
            State::Succeeded | State::Failed | State::Skipped | State::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Run(String),
    Skip(String),
}

#[derive(Debug)]
pub struct Plan {
    order: Vec<String>,
    incoming: HashMap<String, Vec<(String, When)>>,
    outgoing: HashMap<String, Vec<When>>,
}

fn fires(from: State, when: When) -> bool {
    matches!(
        (from, when),
        (State::Succeeded, When::Success | When::Always)
            | (State::Failed, When::Failure | When::Always)
    )
}

impl Plan {
    /// Builds the plan for a workflow that passed [`Workflow::validate`].
    pub fn new(workflow: &Workflow) -> Self {
        let mut incoming: HashMap<String, Vec<(String, When)>> = HashMap::new();
        let mut outgoing: HashMap<String, Vec<When>> = HashMap::new();
        for l in &workflow.links {
            incoming
                .entry(l.to.clone())
                .or_default()
                .push((l.from.clone(), l.when));
            outgoing.entry(l.from.clone()).or_default().push(l.when);
        }
        Plan {
            order: workflow.steps.iter().map(|s| s.id.clone()).collect(),
            incoming,
            outgoing,
        }
    }

    /// The steps to start or skip now, given every step's state. Call it
    /// again after applying the decisions: a skip can unblock more steps.
    pub fn next(&self, states: &HashMap<String, State>) -> Vec<Decision> {
        let state = |id: &str| states.get(id).copied().unwrap_or(State::Pending);
        let mut out = Vec::new();
        for id in &self.order {
            if state(id) != State::Pending {
                continue;
            }
            let links = self.incoming.get(id).map(Vec::as_slice).unwrap_or(&[]);
            if links.is_empty() {
                out.push(Decision::Run(id.clone()));
                continue;
            }
            if links.iter().any(|(from, _)| !state(from).is_done()) {
                continue;
            }
            let live: Vec<_> = links
                .iter()
                .filter(|(from, _)| state(from) != State::Skipped)
                .collect();
            let run = !live.is_empty() && live.iter().all(|(from, when)| fires(state(from), *when));
            out.push(if run {
                Decision::Run(id.clone())
            } else {
                Decision::Skip(id.clone())
            });
        }
        out
    }

    /// Whether a failure of `step` is handled by a failure or always link,
    /// so it does not fail the whole run.
    pub fn handles_failure(&self, step: &str) -> bool {
        self.outgoing
            .get(step)
            .is_some_and(|w| w.iter().any(|w| matches!(w, When::Failure | When::Always)))
    }

    /// Whether the run is over: nothing pending, running or waiting.
    pub fn finished(&self, states: &HashMap<String, State>) -> bool {
        self.order
            .iter()
            .all(|id| states.get(id).copied().unwrap_or(State::Pending).is_done())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Link, Step};

    /// Runs the plan to the end, with `fail` listing the steps that fail.
    fn simulate(w: &Workflow, fail: &[&str]) -> HashMap<String, State> {
        let plan = Plan::new(w);
        let mut states = HashMap::new();
        loop {
            let next = plan.next(&states);
            if next.is_empty() {
                break;
            }
            for d in next {
                match d {
                    Decision::Run(id) => {
                        let s = if fail.contains(&id.as_str()) {
                            State::Failed
                        } else {
                            State::Succeeded
                        };
                        states.insert(id, s);
                    }
                    Decision::Skip(id) => {
                        states.insert(id, State::Skipped);
                    }
                }
            }
        }
        assert!(plan.finished(&states));
        states
    }

    fn cmd(id: &str) -> Step {
        Step::command(id, id, "true", &["h"])
    }

    fn branchy() -> Workflow {
        // check -> deploy (success) -> verify -> done
        //       -> alert  (failure) ----------^
        let mut w = Workflow::new("w", "W");
        w.steps = ["check", "deploy", "alert", "verify", "done"]
            .iter()
            .map(|s| cmd(s))
            .collect();
        w.links = vec![
            Link::new("check", "deploy", When::Success),
            Link::new("check", "alert", When::Failure),
            Link::new("deploy", "verify", When::Success),
            Link::new("verify", "done", When::Always),
            Link::new("alert", "done", When::Always),
        ];
        w.validate().expect("valid");
        w
    }

    #[test]
    fn success_path_skips_the_failure_branch_and_joins() {
        let s = simulate(&branchy(), &[]);
        assert_eq!(s["deploy"], State::Succeeded);
        assert_eq!(s["alert"], State::Skipped);
        assert_eq!(s["done"], State::Succeeded);
    }

    #[test]
    fn failure_path_runs_the_alert() {
        let s = simulate(&branchy(), &["check"]);
        assert_eq!(s["deploy"], State::Skipped);
        assert_eq!(s["verify"], State::Skipped);
        assert_eq!(s["alert"], State::Succeeded);
        assert_eq!(s["done"], State::Succeeded);
    }

    #[test]
    fn a_failure_without_a_handler_skips_what_follows() {
        let s = simulate(&branchy(), &["deploy"]);
        assert_eq!(s["verify"], State::Skipped);
        // `done` only has links from skipped steps now (verify, alert).
        assert_eq!(s["done"], State::Skipped);
    }

    #[test]
    fn parallel_steps_start_together_and_join_needs_all() {
        let mut w = Workflow::new("w", "W");
        w.steps = ["a", "b", "c", "d"].iter().map(|s| cmd(s)).collect();
        w.links = vec![
            Link::new("a", "b", When::Success),
            Link::new("a", "c", When::Success),
            Link::new("b", "d", When::Success),
            Link::new("c", "d", When::Success),
        ];
        let plan = Plan::new(&w);
        let mut states = HashMap::new();
        assert_eq!(plan.next(&states), vec![Decision::Run("a".into())]);
        states.insert("a".into(), State::Succeeded);
        assert_eq!(
            plan.next(&states),
            vec![Decision::Run("b".into()), Decision::Run("c".into())]
        );
        states.insert("b".into(), State::Succeeded);
        states.insert("c".into(), State::Running);
        assert!(plan.next(&states).is_empty(), "d waits for c");
        states.insert("c".into(), State::Failed);
        assert_eq!(plan.next(&states), vec![Decision::Skip("d".into())]);
    }

    #[test]
    fn failure_handlers_are_known() {
        let plan = Plan::new(&branchy());
        assert!(plan.handles_failure("check"));
        assert!(!plan.handles_failure("deploy"));
        assert!(plan.handles_failure("verify"));
    }
}
