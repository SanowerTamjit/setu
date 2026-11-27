//! The health check: one fixed, read-only command run on a Linux server
//! over SSH, and the findings made from its output.
//!
//! [`COMMAND`] takes no input. It asks systemd for failed units, `df` for
//! space and inodes, looks for `/var/run/reboot-required` (Debian and
//! Ubuntu write it when an update needs a restart) and asks `timedatectl`
//! whether the clock is synchronised. Each part prints under a `@@` mark;
//! a part the server cannot answer (no systemd, no timedatectl) is left
//! empty and finds nothing, rather than failing the whole check.
//!
//! The output is **untrusted**: unit names are checked the way systemd
//! checks them, every other field is cleaned and shortened
//! ([`setu_server::field`]) and every list is capped.

use crate::{Action, Finding, Severity, Source};
use setu_server::disk::{parse_df, Filesystem};
use setu_server::field;

/// Most output read from the server.
pub const MAX_OUTPUT: usize = 256 * 1024;
/// Most failed units reported for one host.
pub const MAX_FAILED_UNITS: usize = 100;
/// Most package names shown for a restart.
const MAX_PACKAGES: usize = 6;

/// The check. No input: every host gets exactly this text.
pub const COMMAND: &str = "export LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_PAGER=; \
echo @@units; if command -v systemctl >/dev/null 2>&1; then systemctl list-units --state=failed --all --no-legend --no-pager --plain 2>/dev/null; else echo @@nosystemd; fi; \
echo @@df; df -P -k -T 2>/dev/null; \
echo @@inodes; df -P -i -T 2>/dev/null; \
echo @@reboot; if [ -f /var/run/reboot-required ]; then echo required; head -n 50 /var/run/reboot-required.pkgs 2>/dev/null; fi; \
echo @@clock; timedatectl show -p NTPSynchronized --value 2>/dev/null; \
echo @@end";

/// When a disk (or its inodes) counts as a problem, in percent used.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    pub disk_warning: f64,
    pub disk_error: f64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            disk_warning: 90.0,
            disk_error: 95.0,
        }
    }
}

/// A failed systemd unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedUnit {
    pub name: String,
    pub description: String,
    /// systemd's sub-state: `failed`, `auto-restart`…
    pub sub: String,
}

/// What the check found out about one host.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Health {
    /// `None` when the server has no systemd.
    pub failed: Option<Vec<FailedUnit>>,
    /// More failed units than [`MAX_FAILED_UNITS`].
    pub failed_truncated: bool,
    pub disks: Vec<Filesystem>,
    /// The same file systems, counted in inodes instead of kilobytes.
    pub inodes: Vec<Filesystem>,
    /// `Some` (with the packages that asked, if listed) when a restart is
    /// needed to finish updates.
    pub reboot: Option<Vec<String>>,
    /// `None` when the server did not say (no timedatectl).
    pub clock_synced: Option<bool>,
}

/// Unit types systemd knows.
const UNIT_TYPES: &[&str] = &[
    "service",
    "socket",
    "target",
    "device",
    "mount",
    "automount",
    "swap",
    "timer",
    "path",
    "slice",
    "scope",
];

/// Checks a unit name the way systemd does: letters, digits and
/// `: _ . \ - @`, at most 255 bytes, ending in a known unit type. Anything
/// else (spaces, quotes, escapes) is not a unit and is skipped.
pub fn is_unit_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 255 || name.starts_with(['-', '.', '@']) {
        return false;
    }
    let Some((stem, kind)) = name.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && UNIT_TYPES.contains(&kind)
        && name.matches('@').count() <= 1
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '.' | '\\' | '-' | '@'))
}

/// Reads the output of [`COMMAND`]. An error means the output was not a
/// health report at all (not a Linux server, or cut short).
pub fn parse(output: &str) -> Result<Health, String> {
    let mut health = Health::default();
    let mut section = "";
    let mut ended = false;
    let mut failed: Vec<FailedUnit> = Vec::new();
    let mut systemd = true;
    let mut df = String::new();
    let mut inodes = String::new();
    let mut packages: Vec<String> = Vec::new();
    let mut seen_mark = false;
    for line in output.lines() {
        let line = line.trim_end();
        if let Some(mark) = line.strip_prefix("@@") {
            seen_mark = true;
            match mark.trim() {
                "nosystemd" => systemd = false,
                "end" => {
                    ended = true;
                    section = "";
                }
                m @ ("units" | "df" | "inodes" | "reboot" | "clock") => section = m,
                _ => section = "",
            }
            continue;
        }
        match section {
            "units" => {
                let rest = line
                    .trim_start()
                    .trim_start_matches(['●', '*', '×'])
                    .trim_start();
                let mut words = rest.split_whitespace();
                let (Some(name), Some(_load), Some(_active), Some(sub)) =
                    (words.next(), words.next(), words.next(), words.next())
                else {
                    continue;
                };
                if !is_unit_name(name) || failed.iter().any(|u| u.name == name) {
                    continue;
                }
                if failed.len() >= MAX_FAILED_UNITS {
                    health.failed_truncated = true;
                    continue;
                }
                failed.push(FailedUnit {
                    name: name.to_string(),
                    description: field(&words.collect::<Vec<_>>().join(" "), 160),
                    sub: field(sub, 32),
                });
            }
            "df" => {
                df.push_str(line);
                df.push('\n');
            }
            "inodes" => {
                inodes.push_str(line);
                inodes.push('\n');
            }
            "reboot" => {
                let t = line.trim();
                if t == "required" {
                    health.reboot = Some(Vec::new());
                } else if !t.is_empty() && packages.len() < 50 {
                    let name = field(t, 60);
                    if !packages.contains(&name) {
                        packages.push(name);
                    }
                }
            }
            "clock" => match line.trim() {
                "yes" => health.clock_synced = Some(true),
                "no" => health.clock_synced = Some(false),
                _ => {}
            },
            _ => {}
        }
    }
    if !seen_mark {
        return Err("the server did not answer the health check (checks need a Linux server with a POSIX shell)".into());
    }
    if !ended {
        return Err("the health check was cut short".into());
    }
    health.failed = systemd.then_some(failed);
    // `parse_df` skips the first line, which is df's heading.
    health.disks = parse_df(&df);
    health.inodes = parse_df(&inodes);
    if let Some(list) = health.reboot.as_mut() {
        *list = packages;
    }
    Ok(health)
}

/// Percent used the way `df` shows it: used out of what a user can use,
/// rounded up.
fn percent_used(f: &Filesystem) -> f64 {
    let usable = f.used_kb.saturating_add(f.available_kb);
    if usable == 0 {
        return 0.0;
    }
    (f.used_kb as f64 * 100.0 / usable as f64).ceil()
}

/// "1.5 GB", "800 MB", for kilobytes.
pub fn size(kb: u64) -> String {
    let units = ["KB", "MB", "GB", "TB", "PB"];
    let mut v = kb as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if v >= 10.0 || i == 0 {
        format!("{v:.0} {}", units[i])
    } else {
        format!("{v:.1} {}", units[i])
    }
}

/// "1.2 million", "45,000"-ish counts of inodes, short.
fn count(n: u64) -> String {
    match n {
        0..=9_999 => n.to_string(),
        10_000..=999_999 => format!("{}k", n / 1000),
        _ => format!("{:.1} million", n as f64 / 1_000_000.0),
    }
}

/// Turns a host's health into findings.
pub fn findings(host: &str, health: &Health, limits: Limits) -> Vec<Finding> {
    let mut out = Vec::new();
    let finding =
        |id: String, severity: Severity, title: String, detail: String, action: Option<Action>| {
            Finding {
                id,
                severity,
                source: Source::Check,
                host: Some(host.to_string()),
                title,
                detail,
                since_ms: None,
                action,
            }
        };
    if let Some(units) = &health.failed {
        for u in units {
            let what = if u.description.is_empty() {
                String::new()
            } else {
                format!("{}. ", u.description)
            };
            let action = u.name.ends_with(".service").then(|| Action::Service {
                host: host.to_string(),
                unit: u.name.clone(),
            });
            let hint = if action.is_some() {
                "Open it to read its logs and start it again."
            } else {
                "See `systemctl status` on the server for why."
            };
            out.push(finding(
                format!("unit:{host}:{}", u.name),
                Severity::Error,
                format!("{} failed", u.name),
                if u.sub == "failed" || u.sub.is_empty() {
                    format!("{what}systemd reports it as failed. {hint}")
                } else {
                    format!("{what}systemd reports it as failed ({}). {hint}", u.sub)
                },
                action,
            ));
        }
        if health.failed_truncated {
            out.push(finding(
                format!("units-more:{host}"),
                Severity::Error,
                "More failed units".into(),
                format!("More than {MAX_FAILED_UNITS} units have failed; only the first {MAX_FAILED_UNITS} are listed."),
                None,
            ));
        }
    }
    for d in &health.disks {
        let p = percent_used(d);
        if p < limits.disk_warning {
            continue;
        }
        let severity = if p >= limits.disk_error {
            Severity::Error
        } else {
            Severity::Warning
        };
        out.push(finding(
            format!("disk:{host}:{}", d.mount),
            severity,
            format!("{} is {p:.0}% full", d.mount),
            format!(
                "{} free of {} on {} ({}). Open the Disk tab to see what fills it.",
                size(d.available_kb),
                size(d.total_kb),
                d.device,
                d.kind
            ),
            Some(Action::Disk {
                host: host.to_string(),
            }),
        ));
    }
    for d in &health.inodes {
        let p = percent_used(d);
        if p < limits.disk_warning {
            continue;
        }
        let severity = if p >= limits.disk_error {
            Severity::Error
        } else {
            Severity::Warning
        };
        out.push(finding(
            format!("inodes:{host}:{}", d.mount),
            severity,
            format!("{} has used {p:.0}% of its inodes", d.mount),
            format!(
                "{} of {} file entries left on {}. New files fail when they run out, even with space free; it is usually many small files (caches, sessions, mail).",
                count(d.available_kb),
                count(d.total_kb),
                d.device
            ),
            None,
        ));
    }
    if let Some(packages) = &health.reboot {
        let because = match packages.len() {
            0 => String::new(),
            n if n <= MAX_PACKAGES => format!(" Asked by: {}.", packages.join(", ")),
            n => format!(
                " Asked by: {} and {} more.",
                packages[..MAX_PACKAGES].join(", "),
                n - MAX_PACKAGES
            ),
        };
        out.push(finding(
            format!("reboot:{host}"),
            Severity::Info,
            "Restart needed to finish updates".into(),
            format!("Updates were installed that only take effect after a restart.{because}"),
            None,
        ));
    }
    if health.clock_synced == Some(false) {
        out.push(finding(
            format!("clock:{host}"),
            Severity::Warning,
            "Clock is not synchronised".into(),
            "timedatectl reports no time synchronisation (NTP). A drifting clock breaks certificate checks, logins with time-based codes and log timelines.".into(),
            None,
        ));
    }
    out
}

/// The finding for a host that could not be checked at all.
pub fn unreachable(host: &str, reason: &str) -> Finding {
    Finding {
        id: format!("check:{host}"),
        severity: Severity::Error,
        source: Source::Check,
        host: Some(host.to_string()),
        title: "Could not check this host".into(),
        detail: field(reason, 300),
        since_ms: None,
        action: Some(Action::Connect {
            host: host.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UBUNTU: &str = "@@units
nginx.service      loaded failed failed A high performance web server
\u{25cf} backup.timer  loaded failed failed Nightly backup
bad;name.service loaded failed failed Not a unit
@@df
Filesystem     Type  1024-blocks     Used Available Capacity Mounted on
/dev/sda1      ext4     51475068 48900000   1000000      98% /
/dev/sdb1      xfs     104857600 94371840  10485760      90% /var
/dev/sdc1      xfs     104857600 10485760  94371840      10% /srv
tmpfs          tmpfs      812345        0    812345       0% /run
@@inodes
Filesystem     Type    Inodes   IUsed   IFree IUse% Mounted on
/dev/sda1      ext4   3276800  200000 3076800    7% /
/dev/sdb1      xfs    1000000  960000   40000   96% /var
@@reboot
required
linux-image-6.8.0-49-generic
libc6
@@clock
no
@@end
";

    #[test]
    fn reads_a_full_report() {
        let h = parse(UBUNTU).unwrap_or_default();
        let failed = h.failed.clone().unwrap_or_default();
        assert_eq!(failed.len(), 2, "the bad name is skipped");
        assert_eq!(failed[0].name, "nginx.service");
        assert_eq!(failed[0].description, "A high performance web server");
        assert_eq!(failed[1].name, "backup.timer");
        assert_eq!(h.disks.len(), 3, "tmpfs is not a disk");
        assert_eq!(h.inodes.len(), 2);
        assert_eq!(
            h.reboot,
            Some(vec!["linux-image-6.8.0-49-generic".into(), "libc6".into()])
        );
        assert_eq!(h.clock_synced, Some(false));
    }

    #[test]
    fn turns_a_report_into_findings() {
        let h = parse(UBUNTU).unwrap_or_default();
        let f = findings("web1", &h, Limits::default());
        let ids: Vec<_> = f.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "unit:web1:nginx.service",
                "unit:web1:backup.timer",
                "disk:web1:/",
                "disk:web1:/var",
                "inodes:web1:/var",
                "reboot:web1",
                "clock:web1"
            ]
        );
        assert_eq!(f[0].severity, Severity::Error);
        assert_eq!(
            f[0].action,
            Some(Action::Service {
                host: "web1".into(),
                unit: "nginx.service".into()
            })
        );
        assert_eq!(f[1].action, None, "only services open in the Server view");
        assert_eq!(
            f[0].detail,
            "A high performance web server. systemd reports it as failed. Open it to read its logs and start it again."
        );
        assert_eq!(f[2].title, "/ is 98% full");
        assert_eq!(f[2].severity, Severity::Error);
        assert_eq!(f[3].title, "/var is 90% full");
        assert_eq!(f[3].severity, Severity::Warning);
        assert!(f[3]
            .detail
            .starts_with("10 GB free of 100 GB on /dev/sdb1 (xfs)"));
        assert_eq!(f[4].severity, Severity::Error);
        assert!(f[4].detail.starts_with("40k of 1.0 million"));
        assert_eq!(f[5].severity, Severity::Info);
        assert!(f[5]
            .detail
            .ends_with("Asked by: linux-image-6.8.0-49-generic, libc6."));
        assert_eq!(f[6].severity, Severity::Warning);
    }

    #[test]
    fn a_healthy_server_finds_nothing() {
        let out = "@@units\n@@df\nFilesystem Type 1024-blocks Used Available Capacity Mounted on\n/dev/sda1 ext4 1000 100 900 10% /\n@@inodes\n@@reboot\n@@clock\nyes\n@@end\n";
        let h = parse(out).unwrap_or_default();
        assert_eq!(h.failed, Some(vec![]));
        assert_eq!(h.clock_synced, Some(true));
        assert!(findings("db1", &h, Limits::default()).is_empty());
    }

    #[test]
    fn missing_parts_find_nothing_rather_than_fail() {
        let out = "@@units\n@@nosystemd\n@@df\n@@inodes\n@@reboot\n@@clock\n@@end\n";
        let h = parse(out).unwrap_or_default();
        assert_eq!(h.failed, None);
        assert_eq!(h.clock_synced, None);
        assert!(findings("alpine", &h, Limits::default()).is_empty());
    }

    #[test]
    fn output_that_is_not_a_report_is_an_error() {
        assert!(parse("").is_err());
        assert!(parse("Welcome to RouterOS\n> ").is_err());
        assert!(
            parse("@@units\nnginx.service loaded failed failed x\n@@df\n")
                .is_err_and(|e| e.contains("cut short"))
        );
    }

    #[test]
    fn server_text_is_cleaned_and_capped() {
        let mut out = String::from("@@units\n");
        out.push_str(
            "evil.service loaded failed failed \u{1b}]0;title\u{7}Bad\u{1b}[31m red\u{202e}x\n",
        );
        for i in 0..150 {
            out.push_str(&format!("u{i}.service loaded failed failed d\n"));
        }
        out.push_str("@@reboot\nrequired\n\u{1b}[2Jpkg\n@@end\n");
        let h = parse(&out).unwrap_or_default();
        let failed = h.failed.clone().unwrap_or_default();
        assert_eq!(failed.len(), MAX_FAILED_UNITS);
        assert!(h.failed_truncated);
        assert_eq!(failed[0].description, "Bad redx");
        assert_eq!(h.reboot, Some(vec!["pkg".into()]));
        let f = findings("h", &h, Limits::default());
        assert!(f.iter().any(|f| f.id == "units-more:h"));
    }

    #[test]
    fn unit_names_are_checked() {
        for ok in [
            "nginx.service",
            "getty@tty1.service",
            "dev-sda1.device",
            "a\\x2db.mount",
            "x.timer",
        ] {
            assert!(is_unit_name(ok), "{ok}");
        }
        for bad in [
            "",
            "nginx",
            ".service",
            "-x.service",
            "a b.service",
            "a;b.service",
            "x.exe",
            "a@b@c.service",
            "$(id).service",
        ] {
            assert!(!is_unit_name(bad), "{bad}");
        }
    }

    #[test]
    fn the_command_takes_no_input_and_only_reads() {
        for word in [
            "rm ",
            "kill",
            "systemctl start",
            "systemctl restart",
            "sudo",
        ] {
            assert!(!COMMAND.contains(word), "{word}");
        }
        // The only thing written anywhere is /dev/null.
        let rest = COMMAND.replace(">/dev/null", "").replace("2>&1", "");
        assert!(!rest.contains('>'), "writes a file");
        assert!(COMMAND.ends_with("echo @@end"));
    }

    #[test]
    fn unreachable_hosts_say_why() {
        let f = unreachable("db1", "the host key changed\u{1b}[0m");
        assert_eq!(f.id, "check:db1");
        assert_eq!(f.detail, "the host key changed");
        assert_eq!(f.action, Some(Action::Connect { host: "db1".into() }));
    }

    #[test]
    fn sizes_read_well() {
        assert_eq!(size(512), "512 KB");
        assert_eq!(size(1536), "1.5 MB");
        assert_eq!(size(10 * 1024 * 1024), "10 GB");
    }
}
