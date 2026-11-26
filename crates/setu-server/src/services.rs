//! systemd services: the list, one service's status, its journal, and the
//! commands that change it (start, stop, restart, reload, enable, disable).
//!
//! The list and status commands are fixed and read-only. The only input in
//! any command is a unit name that passed [`check_unit`] and a line count
//! that is clamped, so nothing from the window or from a server reaches a
//! shell unchecked.

use crate::{field, plain_text, tail};
use serde::Serialize;
use std::collections::HashMap;

/// Output kept from the list command.
pub const MAX_LIST_OUTPUT: usize = 2 * 1024 * 1024;
/// Output kept from the status command.
pub const MAX_DETAILS_OUTPUT: usize = 64 * 1024;
/// Log text kept (the newest lines).
pub const MAX_LOG_OUTPUT: usize = 512 * 1024;
/// Output kept from a change (start, stop…).
pub const MAX_ACTION_OUTPUT: usize = 16 * 1024;
/// Most services listed.
pub const MAX_SERVICES: usize = 4000;
/// Fewest and most log lines asked for.
pub const LOG_LINES: (u32, u32) = (10, 5000);
/// Longest unit name systemd accepts.
const MAX_UNIT: usize = 255;
const MAX_DESCRIPTION: usize = 200;
const MAX_STATE: usize = 32;
const MAX_PATH: usize = 300;

/// Lists every service unit systemd has loaded, then every service unit
/// file (to show the ones that are installed but not loaded, and whether
/// each starts at boot). Takes no input.
pub const LIST_COMMAND: &str = "export LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_PAGER=; \
command -v systemctl >/dev/null 2>&1 || { echo @@nosystemd; exit 0; }; \
echo @@units; systemctl list-units --type=service --all --no-legend --no-pager --plain 2>/dev/null; echo \"@@rc $?\"; \
echo @@files; systemctl list-unit-files --type=service --no-legend --no-pager 2>/dev/null";

/// The properties read for one service.
const SHOW_PROPERTIES: &str = "Id,Description,LoadState,ActiveState,SubState,UnitFileState,MainPID,ActiveEnterTimestamp,MemoryCurrent,TasksCurrent,NRestarts,FragmentPath";

/// Checks a service name the way systemd does: letters, digits and
/// `: _ . \ -`, at most one `@` (an instance), ending in `.service`. That
/// leaves out spaces, quotes and every character a shell treats specially.
/// Returns the name, trimmed.
pub fn check_unit(name: &str) -> Result<&str, String> {
    let name = name.trim();
    let bad = || {
        format!(
            "{:?} is not a service name (like nginx.service)",
            field(name, 80)
        )
    };
    if name.is_empty() || name.len() > MAX_UNIT || name.starts_with(['-', '.', '@']) {
        return Err(bad());
    }
    let Some(stem) = name.strip_suffix(".service") else {
        return Err(bad());
    };
    if stem.is_empty()
        || stem.ends_with('@')
        || stem.matches('@').count() > 1
        || !stem
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '.' | '\\' | '-' | '@'))
    {
        return Err(bad());
    }
    Ok(name)
}

/// The unit name as it goes into a command. It already passed
/// [`check_unit`]; a name with a `\` (systemd's escapes, like `\x2d`) is
/// put in single quotes so the shell keeps it as it is.
fn quoted(unit: &str) -> String {
    if unit.contains('\\') {
        format!("'{unit}'")
    } else {
        unit.to_string()
    }
}

fn sudo(use_sudo: bool) -> &'static str {
    // -n: fail at once instead of asking for a password Setu would never send.
    if use_sudo {
        "sudo -n "
    } else {
        ""
    }
}

/// A change to a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
    Reload,
    /// Start at boot.
    Enable,
    /// Do not start at boot.
    Disable,
}

impl ServiceAction {
    pub const ALL: [ServiceAction; 6] = [
        ServiceAction::Start,
        ServiceAction::Stop,
        ServiceAction::Restart,
        ServiceAction::Reload,
        ServiceAction::Enable,
        ServiceAction::Disable,
    ];

    /// The `systemctl` verb, which is also the name the window uses.
    pub fn verb(self) -> &'static str {
        match self {
            ServiceAction::Start => "start",
            ServiceAction::Stop => "stop",
            ServiceAction::Restart => "restart",
            ServiceAction::Reload => "reload",
            ServiceAction::Enable => "enable",
            ServiceAction::Disable => "disable",
        }
    }

    pub fn parse(verb: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|a| a.verb() == verb)
            .ok_or_else(|| {
                format!(
                    "{:?} is not something Setu can do to a service",
                    field(verb, 20)
                )
            })
    }
}

/// The exact command for a change, as shown to the person before it runs
/// and as run after they agree.
pub fn action_command(action: ServiceAction, unit: &str, use_sudo: bool) -> Result<String, String> {
    let unit = check_unit(unit)?;
    Ok(format!(
        "{}systemctl {} {}",
        sudo(use_sudo),
        action.verb(),
        quoted(unit)
    ))
}

/// The command that reads one service's status.
pub fn details_command(unit: &str, use_sudo: bool) -> Result<String, String> {
    let unit = check_unit(unit)?;
    Ok(format!(
        "export LC_ALL=C SYSTEMD_COLORS=0; {}systemctl show --no-pager -p {SHOW_PROPERTIES} {}",
        sudo(use_sudo),
        quoted(unit)
    ))
}

/// Which journal lines to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogLevel {
    #[default]
    All,
    /// Warnings and worse.
    Warnings,
    /// Errors and worse.
    Errors,
}

impl LogLevel {
    pub fn parse(s: &str) -> Self {
        match s {
            "warnings" => LogLevel::Warnings,
            "errors" => LogLevel::Errors,
            _ => LogLevel::All,
        }
    }
}

/// The command that reads a service's newest journal lines. `lines` is
/// clamped to [`LOG_LINES`].
pub fn logs_command(
    unit: &str,
    lines: u32,
    level: LogLevel,
    use_sudo: bool,
) -> Result<String, String> {
    let unit = check_unit(unit)?;
    let lines = lines.clamp(LOG_LINES.0, LOG_LINES.1);
    let level = match level {
        LogLevel::All => "",
        LogLevel::Warnings => " -p warning",
        LogLevel::Errors => " -p err",
    };
    Ok(format!(
        "export LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_PAGER=; {}journalctl -u {} -n {lines}{level} --no-pager -o short-iso 2>&1",
        sudo(use_sudo),
        quoted(unit)
    ))
}

/// One service in the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    pub name: String,
    pub description: String,
    /// `loaded`, `not-found`, `masked`… Empty when only the unit file is
    /// known (it is installed but systemd has not loaded it).
    pub load: String,
    /// `active`, `inactive`, `failed`, `activating`…
    pub active: String,
    /// `running`, `exited`, `dead`…
    pub sub: String,
    /// Whether it starts at boot: `enabled`, `disabled`, `static`,
    /// `masked`… Empty when there is no unit file.
    pub startup: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceList {
    pub services: Vec<Service>,
    /// More services than [`MAX_SERVICES`]; the rest were left out.
    pub truncated: bool,
}

/// Reads the output of [`LIST_COMMAND`]. An error says why there is no
/// list (no systemd).
pub fn parse_list(output: &str) -> Result<ServiceList, String> {
    let mut section = "";
    let mut rc: Option<i32> = None;
    let mut services: Vec<Service> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut startup: Vec<(String, String)> = Vec::new();
    let mut truncated = false;
    for line in output.lines() {
        let line = line.trim_end();
        if let Some(mark) = line.strip_prefix("@@") {
            let mark = mark.trim();
            if mark == "nosystemd" {
                return Err("this server has no systemctl: managing services needs systemd".into());
            }
            if let Some(code) = mark.strip_prefix("rc ") {
                rc = code.trim().parse().ok();
                continue;
            }
            section = match mark {
                "units" => "units",
                "files" => "files",
                _ => "",
            };
            continue;
        }
        match section {
            "units" => {
                // UNIT LOAD ACTIVE SUB DESCRIPTION…, perhaps after a ● mark.
                let rest = line
                    .trim_start()
                    .trim_start_matches(['●', '*', '×'])
                    .trim_start();
                let mut words = rest.split_whitespace();
                let (Some(name), Some(load), Some(active), Some(sub)) =
                    (words.next(), words.next(), words.next(), words.next())
                else {
                    continue;
                };
                let Ok(name) = check_unit(name) else { continue };
                // Names other units mention but that are not installed.
                if load == "not-found" || index.contains_key(name) {
                    continue;
                }
                if services.len() >= MAX_SERVICES {
                    truncated = true;
                    continue;
                }
                let description = words.collect::<Vec<_>>().join(" ");
                index.insert(name.to_string(), services.len());
                services.push(Service {
                    name: name.to_string(),
                    description: field(&description, MAX_DESCRIPTION),
                    load: field(load, MAX_STATE),
                    active: field(active, MAX_STATE),
                    sub: field(sub, MAX_STATE),
                    startup: String::new(),
                });
            }
            "files" => {
                let mut words = line.split_whitespace();
                let (Some(name), Some(state)) = (words.next(), words.next()) else {
                    continue;
                };
                // Templates (foo@.service) need an instance to run, and an
                // alias is another name for a unit already listed.
                if state == "alias" {
                    continue;
                }
                if let Ok(name) = check_unit(name) {
                    startup.push((name.to_string(), field(state, MAX_STATE)));
                }
            }
            _ => {}
        }
    }
    for (name, state) in startup {
        match index.get(&name) {
            Some(&i) => services[i].startup = state,
            None if services.len() < MAX_SERVICES => {
                index.insert(name.clone(), services.len());
                services.push(Service {
                    name,
                    description: String::new(),
                    load: String::new(),
                    active: "inactive".into(),
                    sub: "dead".into(),
                    startup: state,
                });
            }
            None => truncated = true,
        }
    }
    if services.is_empty() && rc.is_some_and(|c| c != 0) {
        return Err(
            "systemd is not running on this server (systemctl could not list services)".into(),
        );
    }
    services.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(ServiceList {
        services,
        truncated,
    })
}

/// One service's status.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceDetails {
    pub name: String,
    pub description: String,
    pub load: String,
    pub active: String,
    pub sub: String,
    pub startup: String,
    pub main_pid: Option<u32>,
    /// When it last became active, as systemd writes it.
    pub since: Option<String>,
    pub memory_bytes: Option<u64>,
    pub tasks: Option<u64>,
    pub restarts: Option<u64>,
    /// The unit file.
    pub path: Option<String>,
}

/// Reads the output of [`details_command`]: `Key=value` lines.
pub fn parse_details(unit: &str, output: &str) -> Result<ServiceDetails, String> {
    let mut d = ServiceDetails {
        name: unit.to_string(),
        ..ServiceDetails::default()
    };
    let mut any = false;
    // systemd's "not set" for numbers: u64::MAX, or [not set].
    let number = |v: &str| v.parse::<u64>().ok().filter(|n| *n != u64::MAX);
    let text = |v: &str, max: usize| Some(field(v, max)).filter(|s| !s.is_empty());
    for line in output.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        any = true;
        match key.trim() {
            "Id" => {
                if let Ok(id) = check_unit(value) {
                    d.name = id.to_string();
                }
            }
            "Description" => d.description = field(value, MAX_DESCRIPTION),
            "LoadState" => d.load = field(value, MAX_STATE),
            "ActiveState" => d.active = field(value, MAX_STATE),
            "SubState" => d.sub = field(value, MAX_STATE),
            "UnitFileState" => d.startup = field(value, MAX_STATE),
            "MainPID" => d.main_pid = value.parse::<u32>().ok().filter(|p| *p > 0),
            "ActiveEnterTimestamp" => d.since = text(value, 64),
            "MemoryCurrent" => d.memory_bytes = number(value),
            "TasksCurrent" => d.tasks = number(value),
            "NRestarts" => d.restarts = number(value),
            "FragmentPath" => d.path = text(value, MAX_PATH),
            _ => {}
        }
    }
    if !any {
        let said = field(output, 200);
        return Err(if said.is_empty() {
            "the server did not report the service's status".into()
        } else {
            said
        });
    }
    Ok(d)
}

/// A service's journal lines, cleaned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceLogs {
    pub text: String,
    /// Older lines were cut to keep the text short.
    pub truncated: bool,
    /// The command that read them, to show beside the logs.
    pub command: String,
}

/// Cleans journal output for the window.
pub fn clean_logs(output: &str, command: String) -> ServiceLogs {
    let (text, truncated) = tail(&plain_text(output), MAX_LOG_OUTPUT);
    ServiceLogs {
        text,
        truncated,
        command,
    }
}

/// What a change did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResult {
    pub command: String,
    pub ok: bool,
    pub code: Option<u32>,
    /// What the command printed (often nothing), cleaned.
    pub output: String,
    /// A plain explanation of a failure, when one is known.
    pub hint: Option<String>,
}

/// Builds the result of a change from what the server sent back.
pub fn action_result(
    command: String,
    output: &str,
    code: Option<u32>,
    use_sudo: bool,
) -> ActionResult {
    let (output, _) = tail(plain_text(output).trim(), MAX_ACTION_OUTPUT);
    let ok = code == Some(0);
    let hint = if ok { None } else { explain(&output, use_sudo) };
    ActionResult {
        command,
        ok,
        code,
        output,
        hint,
    }
}

/// Puts common failures in plain words.
pub fn explain(output: &str, use_sudo: bool) -> Option<String> {
    let lower = output.to_ascii_lowercase();
    if lower.contains("a password is required") || lower.contains("a terminal is required") {
        return Some(
            "sudo wants a password on this server. Setu never types passwords into sudo: allow this command without a password for your user (NOPASSWD in sudoers), or run it in a terminal."
                .into(),
        );
    }
    if lower.contains("is not in the sudoers") || lower.contains("not allowed to execute") {
        return Some("Your user may not run this with sudo on this server.".into());
    }
    if lower.contains("sudo: command not found") || lower.contains("sudo: not found") {
        return Some("This server has no sudo.".into());
    }
    if !use_sudo
        && (lower.contains("interactive authentication required")
            || lower.contains("access denied")
            || lower.contains("permission denied"))
    {
        return Some("This needs administrator rights. Turn on \"Use sudo\" and try again.".into());
    }
    if lower.contains("not found") && lower.contains("unit") {
        return Some("systemd does not know this service on this server.".into());
    }
    if lower.contains("job type reload is not applicable")
        || lower.contains("does not support reload")
    {
        return Some("This service cannot reload its settings; restart it instead.".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_names_are_checked() {
        for ok in [
            "nginx.service",
            "getty@tty1.service",
            "systemd-fsck@dev-disk-by\\x2duuid-1234.service",
            "dbus-org.freedesktop.login1.service",
            "a:b_c.service",
        ] {
            assert_eq!(check_unit(ok), Ok(ok), "{ok}");
        }
        assert_eq!(check_unit("  sshd.service \n"), Ok("sshd.service"));
        for bad in [
            "",
            "nginx",
            "nginx.socket",
            ".service",
            "-x.service",
            "--now.service",
            ".hidden.service",
            "getty@.service",
            "a@b@c.service",
            "a b.service",
            "a;reboot.service",
            "a'b.service",
            "a\"b.service",
            "$(reboot).service",
            "`id`.service",
            "a|b.service",
            "a&b.service",
            "a>b.service",
            "a\nb.service",
            "a/b.service",
            "ä.service",
            "a*.service",
        ] {
            assert!(check_unit(bad).is_err(), "{bad:?}");
        }
        assert!(check_unit(&format!("{}.service", "a".repeat(250))).is_err());
    }

    #[test]
    fn change_commands_are_exact() {
        assert_eq!(
            action_command(ServiceAction::Restart, "nginx.service", false).as_deref(),
            Ok("systemctl restart nginx.service")
        );
        assert_eq!(
            action_command(ServiceAction::Enable, "nginx.service", true).as_deref(),
            Ok("sudo -n systemctl enable nginx.service")
        );
        assert_eq!(
            action_command(ServiceAction::Stop, "a\\x2db.service", false).as_deref(),
            Ok("systemctl stop 'a\\x2db.service'")
        );
        assert!(action_command(ServiceAction::Start, "x; reboot", true).is_err());
        for a in ServiceAction::ALL {
            assert_eq!(ServiceAction::parse(a.verb()), Ok(a));
        }
        assert!(ServiceAction::parse("mask").is_err());
        assert!(ServiceAction::parse("start; reboot").is_err());
    }

    #[test]
    fn read_commands_clamp_and_quote() {
        let c =
            logs_command("nginx.service", 1_000_000, LogLevel::Errors, false).unwrap_or_default();
        assert!(
            c.contains("journalctl -u nginx.service -n 5000 -p err --no-pager"),
            "{c}"
        );
        let c = logs_command("nginx.service", 0, LogLevel::All, true).unwrap_or_default();
        assert!(
            c.contains("sudo -n journalctl -u nginx.service -n 10 --no-pager"),
            "{c}"
        );
        assert!(logs_command("a b.service", 10, LogLevel::All, false).is_err());
        let c = details_command("nginx.service", false).unwrap_or_default();
        assert!(c.ends_with("FragmentPath nginx.service"), "{c}");
        assert_eq!(LogLevel::parse("warnings"), LogLevel::Warnings);
        assert_eq!(LogLevel::parse("anything"), LogLevel::All);
    }

    const LIST: &str = "@@units
cron.service                 loaded    active   running Regular background program processing daemon
nginx.service                loaded    failed   failed  A high performance web server
● ghost.service              not-found inactive dead    ghost.service
evil.service                 loaded    active   running \u{1b}[31mRed\u{1b}[0m text\u{7}
not a unit line
@@rc 0
@@files
cron.service                 enabled         enabled
nginx.service                disabled        enabled
getty@.service               enabled         enabled
dbus-org.freedesktop.login1.service alias  -
postgresql.service           disabled        enabled
evil.service                 static          -
";

    #[test]
    fn the_list_merges_units_and_files() {
        let list = parse_list(LIST).unwrap_or_else(|e| panic!("{e}"));
        let names: Vec<_> = list.services.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "cron.service",
                "evil.service",
                "nginx.service",
                "postgresql.service"
            ]
        );
        let nginx = &list.services[2];
        assert_eq!(
            (nginx.active.as_str(), nginx.sub.as_str()),
            ("failed", "failed")
        );
        assert_eq!(nginx.startup, "disabled");
        assert_eq!(nginx.description, "A high performance web server");
        assert_eq!(list.services[1].description, "Red text");
        // Installed but never loaded.
        let pg = &list.services[3];
        assert_eq!(
            (pg.load.as_str(), pg.active.as_str(), pg.startup.as_str()),
            ("", "inactive", "disabled")
        );
        assert!(!list.truncated);
    }

    #[test]
    fn no_systemd_is_explained() {
        assert!(parse_list("@@nosystemd\n").is_err_and(|e| e.contains("needs systemd")));
        assert!(parse_list("@@units\n@@rc 1\n@@files\n").is_err_and(|e| e.contains("not running")));
        // An empty but working systemd is just an empty list.
        assert_eq!(
            parse_list("@@units\n@@rc 0\n@@files\n").map(|l| l.services.len()),
            Ok(0)
        );
    }

    #[test]
    fn the_list_is_capped() {
        let mut out = String::from("@@units\n");
        for i in 0..MAX_SERVICES + 10 {
            out.push_str(&format!("s{i}.service loaded active running S\n"));
        }
        out.push_str("@@rc 0\n@@files\nextra.service enabled\n");
        let list = parse_list(&out).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(list.services.len(), MAX_SERVICES);
        assert!(list.truncated);
    }

    #[test]
    fn details_are_read() {
        let out = "Id=nginx.service
Description=A high performance web server
LoadState=loaded
ActiveState=active
SubState=running
UnitFileState=enabled
MainPID=1234
ActiveEnterTimestamp=Thu 2026-11-26 10:00:00 UTC
MemoryCurrent=18446744073709551615
TasksCurrent=3
NRestarts=0
FragmentPath=/lib/systemd/system/nginx.service
";
        let d = parse_details("nginx.service", out).unwrap_or_default();
        assert_eq!(d.main_pid, Some(1234));
        assert_eq!(d.memory_bytes, None);
        assert_eq!(d.tasks, Some(3));
        assert_eq!(d.startup, "enabled");
        assert_eq!(d.since.as_deref(), Some("Thu 2026-11-26 10:00:00 UTC"));
        assert_eq!(d.path.as_deref(), Some("/lib/systemd/system/nginx.service"));
        let stopped = parse_details(
            "x.service",
            "MainPID=0\nMemoryCurrent=[not set]\nActiveEnterTimestamp=\n",
        )
        .unwrap_or_default();
        assert_eq!(
            (stopped.main_pid, stopped.memory_bytes, stopped.since),
            (None, None, None)
        );
        assert!(parse_details("x.service", "sudo: a password is required\n").is_err());
    }

    #[test]
    fn logs_are_cleaned_and_cut() {
        let logs = clean_logs("a \u{1b}[1mbold\u{1b}[0m\r\nb\n", "cmd".into());
        assert_eq!(logs.text, "a bold\nb\n");
        assert!(!logs.truncated);
        let big = "line\n".repeat(MAX_LOG_OUTPUT);
        let logs = clean_logs(&big, "cmd".into());
        assert!(
            logs.truncated && logs.text.len() <= MAX_LOG_OUTPUT && logs.text.starts_with("line\n")
        );
    }

    #[test]
    fn failures_get_plain_hints() {
        let r = action_result("x".into(), "sudo: a password is required\n", Some(1), true);
        assert!(!r.ok && r.hint.as_deref().is_some_and(|h| h.contains("NOPASSWD")));
        let r = action_result(
            "x".into(),
            "Failed to restart nginx.service: Interactive authentication required.",
            Some(1),
            false,
        );
        assert!(r.hint.as_deref().is_some_and(|h| h.contains("Use sudo")));
        let r = action_result(
            "x".into(),
            "Failed to start nope.service: Unit nope.service not found.",
            Some(5),
            true,
        );
        assert!(r
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("does not know")));
        let r = action_result("x".into(), "", Some(0), false);
        assert!(r.ok && r.hint.is_none());
        assert!(!action_result("x".into(), "", None, false).ok);
    }
}
