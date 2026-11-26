//! Alerts that last: a reading over a limit becomes an alert only once it
//! has stayed over for a while (`hold`), so a short spike does not wake
//! anyone. [`AlertTracker`] follows each alert from one sample to the next
//! and says when one starts (has lasted `hold`) and when one that started
//! has cleared, which is when Setu sends a desktop notification.
//!
//! Time is passed in, in seconds from any fixed start, so this is tested
//! without a clock.

use crate::Alert;
use serde::Serialize;
use std::collections::HashMap;

/// The longest hold allowed, in seconds.
pub const MAX_HOLD_SECS: u64 = 3600;

impl Alert {
    /// Names what the alert is about, so it is the same alert in the next
    /// sample even when the figure has changed.
    pub fn key(&self) -> String {
        match self {
            Alert::Cpu { .. } => "cpu".to_string(),
            Alert::Memory { .. } => "memory".to_string(),
            Alert::Disk { mount, .. } => format!("disk:{mount}"),
        }
    }

    /// "CPU at 97%", "Disk /var is 93% full".
    pub fn text(&self) -> String {
        match self {
            Alert::Cpu { percent } => format!("CPU at {percent:.0}%"),
            Alert::Memory { percent } => format!("Memory at {percent:.0}%"),
            Alert::Disk { mount, percent } => format!("Disk {mount} is {percent:.0}% full"),
        }
    }

    /// What it was about, for "… is back under the limit".
    fn subject(&self) -> String {
        match self {
            Alert::Cpu { .. } => "CPU".to_string(),
            Alert::Memory { .. } => "Memory".to_string(),
            Alert::Disk { mount, .. } => format!("Disk {mount}"),
        }
    }
}

/// A reading over its limit, and for how long.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveAlert {
    #[serde(flatten)]
    pub alert: Alert,
    /// Seconds over the limit so far.
    pub secs: u64,
    /// True once it has lasted the hold time; before that it is pending.
    pub firing: bool,
}

/// What changed with one sample.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct AlertUpdate {
    /// Every reading over its limit, oldest first.
    pub active: Vec<ActiveAlert>,
    /// Alerts that have just lasted the hold time.
    pub started: Vec<ActiveAlert>,
    /// Alerts that had started and are now back under the limit (with the
    /// last figure seen).
    pub cleared: Vec<Alert>,
}

#[derive(Debug)]
struct Tracked {
    since: u64,
    firing: bool,
    last: Alert,
}

#[derive(Debug)]
pub struct AlertTracker {
    hold: u64,
    tracked: HashMap<String, Tracked>,
}

impl AlertTracker {
    /// `hold_secs` (clamped to [`MAX_HOLD_SECS`]) is how long a reading must
    /// stay over its limit; 0 alerts at once.
    pub fn new(hold_secs: u64) -> Self {
        Self {
            hold: hold_secs.min(MAX_HOLD_SECS),
            tracked: HashMap::new(),
        }
    }

    /// Takes the alerts of a sample taken at `now` (seconds). A reading that
    /// drops under its limit for one sample starts its wait again.
    pub fn update(&mut self, now: u64, alerts: &[Alert]) -> AlertUpdate {
        let mut update = AlertUpdate::default();
        let keys: Vec<String> = alerts.iter().map(Alert::key).collect();
        let gone: Vec<String> = self
            .tracked
            .keys()
            .filter(|k| !keys.contains(k))
            .cloned()
            .collect();
        for key in gone {
            if let Some(t) = self.tracked.remove(&key) {
                if t.firing {
                    update.cleared.push(t.last);
                }
            }
        }
        for (alert, key) in alerts.iter().zip(keys) {
            let t = self.tracked.entry(key).or_insert_with(|| Tracked {
                since: now,
                firing: false,
                last: alert.clone(),
            });
            t.last = alert.clone();
            let secs = now.saturating_sub(t.since);
            let active = ActiveAlert {
                alert: alert.clone(),
                secs,
                firing: secs >= self.hold,
            };
            if active.firing && !t.firing {
                t.firing = true;
                update.started.push(active.clone());
            }
            update.active.push(active);
        }
        update.active.sort_by_key(|a| std::cmp::Reverse(a.secs));
        update
    }
}

/// "2 min", "45 s", "1 h 5 min".
pub fn duration_words(secs: u64) -> String {
    if secs < 60 {
        return format!("{secs} s");
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins} min");
    }
    match mins % 60 {
        0 => format!("{} h", mins / 60),
        m => format!("{} h {m} min", mins / 60),
    }
}

/// Title and body of the desktop notification for an alert that started.
/// Figures and names come from the server, so characters some desktops read
/// as markup are dropped.
pub fn started_notice(host: &str, a: &ActiveAlert) -> (String, String) {
    let body = if a.secs == 0 {
        "Over the limit you set in Monitoring.".to_string()
    } else {
        format!(
            "Over the limit you set in Monitoring for {}.",
            duration_words(a.secs)
        )
    };
    (plain(&format!("{host}: {}", a.alert.text())), body)
}

/// Title and body for an alert that has cleared.
pub fn cleared_notice(host: &str, a: &Alert) -> (String, String) {
    (
        plain(&format!("{host}: {} is back under the limit", a.subject())),
        plain(&format!("Last reading over it: {}.", a.text())),
    )
}

fn plain(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() && !matches!(c, '<' | '>' | '&'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpu(p: f64) -> Alert {
        Alert::Cpu { percent: p }
    }

    fn disk(m: &str, p: f64) -> Alert {
        Alert::Disk {
            mount: m.to_string(),
            percent: p,
        }
    }

    #[test]
    fn an_alert_starts_only_after_the_hold() {
        let mut t = AlertTracker::new(120);
        let u = t.update(0, &[cpu(95.0)]);
        assert_eq!(u.active.len(), 1);
        assert!(!u.active[0].firing);
        assert!(u.started.is_empty());

        let u = t.update(60, &[cpu(97.0)]);
        assert_eq!(u.active[0].secs, 60);
        assert!(!u.active[0].firing);

        let u = t.update(120, &[cpu(99.0)]);
        assert!(u.active[0].firing);
        assert_eq!(u.started.len(), 1);
        assert_eq!(u.started[0].alert, cpu(99.0));

        // Started once, not again on every sample.
        let u = t.update(122, &[cpu(99.0)]);
        assert!(u.active[0].firing);
        assert!(u.started.is_empty());
    }

    #[test]
    fn a_dip_under_the_limit_starts_the_wait_again() {
        let mut t = AlertTracker::new(120);
        t.update(0, &[cpu(95.0)]);
        let u = t.update(100, &[]);
        assert!(u.cleared.is_empty(), "it never started, so nothing clears");
        t.update(110, &[cpu(95.0)]);
        let u = t.update(200, &[cpu(95.0)]);
        assert!(!u.active[0].firing);
        assert_eq!(u.active[0].secs, 90);
    }

    #[test]
    fn a_started_alert_clears_with_its_last_figure() {
        let mut t = AlertTracker::new(0);
        let u = t.update(0, &[disk("/var", 93.0), cpu(91.0)]);
        assert_eq!(u.started.len(), 2);
        let u = t.update(2, &[disk("/var", 94.0)]);
        assert_eq!(u.cleared, vec![cpu(91.0)]);
        assert_eq!(u.active.len(), 1);
        let u = t.update(4, &[]);
        assert_eq!(u.cleared, vec![disk("/var", 94.0)]);
    }

    #[test]
    fn disks_are_followed_one_by_one_and_longest_comes_first() {
        let mut t = AlertTracker::new(60);
        t.update(0, &[disk("/", 95.0)]);
        let u = t.update(30, &[disk("/data", 99.0), disk("/", 95.0)]);
        assert_eq!(u.active[0].alert, disk("/", 95.0));
        assert_eq!(u.active[0].secs, 30);
        assert_eq!(u.active[1].secs, 0);
        let u = t.update(60, &[disk("/data", 99.0), disk("/", 95.0)]);
        assert_eq!(u.started.len(), 1);
        assert_eq!(u.started[0].alert, disk("/", 95.0));
    }

    #[test]
    fn hold_is_capped() {
        let mut t = AlertTracker::new(u64::MAX);
        t.update(0, &[cpu(95.0)]);
        assert_eq!(t.update(MAX_HOLD_SECS, &[cpu(95.0)]).started.len(), 1);
    }

    #[test]
    fn notices_read_well_and_drop_markup() {
        let a = ActiveAlert {
            alert: disk("/srv/<b>x</b>&", 93.4),
            secs: 125,
            firing: true,
        };
        let (title, body) = started_notice("db-01", &a);
        assert_eq!(title, "db-01: Disk /srv/bx/b is 93% full");
        assert_eq!(body, "Over the limit you set in Monitoring for 2 min.");
        let (title, body) = cleared_notice("db-01", &cpu(96.6));
        assert_eq!(title, "db-01: CPU is back under the limit");
        assert_eq!(body, "Last reading over it: CPU at 97%.");
        let now = ActiveAlert {
            alert: cpu(91.0),
            secs: 0,
            firing: true,
        };
        assert_eq!(
            started_notice("web", &now).1,
            "Over the limit you set in Monitoring."
        );
    }

    #[test]
    fn durations() {
        assert_eq!(duration_words(5), "5 s");
        assert_eq!(duration_words(60), "1 min");
        assert_eq!(duration_words(3600), "1 h");
        assert_eq!(duration_words(3900), "1 h 5 min");
    }

    #[test]
    fn active_alerts_serialize_flat() -> Result<(), serde_json::Error> {
        let a = ActiveAlert {
            alert: disk("/", 91.0),
            secs: 3,
            firing: false,
        };
        let v = serde_json::to_value(&a)?;
        assert_eq!(v["kind"], "disk");
        assert_eq!(v["mount"], "/");
        assert_eq!(v["secs"], 3);
        assert_eq!(v["firing"], false);
        Ok(())
    }
}
