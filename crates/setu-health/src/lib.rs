//! Problems and health for Setu: one list of what needs attention across
//! saved hosts, like the problems list of a code editor.
//!
//! A [`Finding`] comes from one of three places:
//!
//! - **Health checks** ([`check`]): one fixed, read-only command run over
//!   SSH that reports failed systemd units, nearly full disks (space and
//!   inodes), a restart needed after updates and a clock that is not
//!   synchronised. It takes no input, so nothing typed in Setu or read from
//!   a server ever reaches a shell.
//! - **Monitoring** ([`sources::monitor`]): alerts that have lasted their
//!   hold time on hosts being watched.
//! - **Workflows** ([`sources::workflows`]): the latest run of a workflow,
//!   when it failed.
//!
//! Each finding has a stable [`Finding::id`], so it can be snoozed
//! ([`snooze`]) and so Setu knows how long it has been there
//! ([`Seen`]). Everything a server sent is cleaned (no control characters
//! or terminal escapes) and shortened before it becomes a finding.

pub mod check;
pub mod snooze;
pub mod sources;

use serde::Serialize;
use std::collections::HashMap;

/// How much a finding matters. Ordered: errors first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    /// Something is broken: a failed service, a disk about to fill, a host
    /// that could not be checked, a failed workflow.
    Error,
    /// Something will break if nobody looks: a disk filling up, a clock
    /// drifting, a monitoring alert.
    Warning,
    /// Worth knowing: a restart is needed to finish updates.
    Info,
}

/// Where a finding came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    Check,
    Monitor,
    Workflow,
}

/// What the window can open to look into a finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Action {
    /// A service in the Server view.
    Service { host: String, unit: String },
    /// The Disk tab of the Server view.
    Disk { host: String },
    /// The host's panel in Monitoring.
    Monitor { host: String },
    /// A workflow (and the run that failed) in Workflows.
    Workflow { workflow: String, run: String },
    /// The host's login, to see why it could not be checked.
    Connect { host: String },
}

/// One problem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// Stable for the same problem from one check to the next, e.g.
    /// `unit:web1:nginx.service` or `disk:db1:/var`.
    pub id: String,
    pub severity: Severity,
    pub source: Source,
    /// The saved host it is about, when it is about one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// One line, e.g. "nginx.service failed" or "/var is 96% full".
    pub title: String,
    /// A sentence or two with the figures and what it means.
    pub detail: String,
    /// When Setu first saw it (Unix ms); filled in by [`Seen::stamp`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Action>,
}

/// Puts errors first, then warnings, then the rest; within each, by host
/// and title, so the list does not jump around between checks.
pub fn sort(findings: &mut [Finding]) {
    findings.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.host.cmp(&b.host))
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.id.cmp(&b.id))
    });
}

/// How many findings of each severity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
}

pub fn counts(findings: &[Finding]) -> Counts {
    let mut c = Counts::default();
    for f in findings {
        match f.severity {
            Severity::Error => c.errors += 1,
            Severity::Warning => c.warnings += 1,
            Severity::Info => c.infos += 1,
        }
    }
    c
}

/// Remembers when each finding was first seen, so the list can say "for 2
/// hours". A finding that goes away and comes back starts again.
#[derive(Debug, Default, Clone)]
pub struct Seen {
    first: HashMap<String, u64>,
}

impl Seen {
    /// Sets `since_ms` on each finding (now for new ones) and forgets the
    /// ones no longer in `findings`.
    pub fn stamp(&mut self, findings: &mut [Finding], now_ms: u64) {
        let mut next = HashMap::with_capacity(findings.len());
        for f in findings.iter_mut() {
            let first = *self.first.get(&f.id).unwrap_or(&now_ms);
            f.since_ms = Some(first);
            next.insert(f.id.clone(), first);
        }
        self.first = next;
    }
}

#[cfg(test)]
pub(crate) fn finding(id: &str, severity: Severity, host: Option<&str>, title: &str) -> Finding {
    Finding {
        id: id.into(),
        severity,
        source: Source::Check,
        host: host.map(Into::into),
        title: title.into(),
        detail: String::new(),
        since_ms: None,
        action: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_come_first_then_host_and_title() {
        let mut list = vec![
            finding("a", Severity::Info, Some("web1"), "Restart needed"),
            finding("b", Severity::Warning, Some("db1"), "Clock"),
            finding("c", Severity::Error, Some("web1"), "nginx failed"),
            finding("d", Severity::Error, Some("db1"), "Could not check"),
        ];
        sort(&mut list);
        let ids: Vec<_> = list.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["d", "c", "b", "a"]);
        assert_eq!(
            counts(&list),
            Counts {
                errors: 2,
                warnings: 1,
                infos: 1
            }
        );
    }

    #[test]
    fn seen_keeps_the_first_time_and_forgets_what_went_away() {
        let mut seen = Seen::default();
        let mut one = vec![finding("x", Severity::Error, None, "x")];
        seen.stamp(&mut one, 1000);
        assert_eq!(one[0].since_ms, Some(1000));
        let mut two = vec![
            finding("x", Severity::Error, None, "x"),
            finding("y", Severity::Error, None, "y"),
        ];
        seen.stamp(&mut two, 5000);
        assert_eq!(two[0].since_ms, Some(1000));
        assert_eq!(two[1].since_ms, Some(5000));
        // x went away, then came back: it starts again.
        seen.stamp(&mut [], 6000);
        let mut three = vec![finding("x", Severity::Error, None, "x")];
        seen.stamp(&mut three, 7000);
        assert_eq!(three[0].since_ms, Some(7000));
    }

    #[test]
    fn findings_serialize_for_the_window() {
        let mut f = finding(
            "unit:web1:nginx.service",
            Severity::Error,
            Some("web1"),
            "nginx.service failed",
        );
        f.action = Some(Action::Service {
            host: "web1".into(),
            unit: "nginx.service".into(),
        });
        let v = serde_json::to_value(&f).unwrap_or_default();
        assert_eq!(v["severity"], "error");
        assert_eq!(v["source"], "check");
        assert_eq!(v["action"]["kind"], "service");
        assert_eq!(v["action"]["unit"], "nginx.service");
        assert!(v.get("sinceMs").is_none());
    }
}
