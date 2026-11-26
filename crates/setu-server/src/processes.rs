//! Processes: the list (from `ps`, busiest first) and the command that
//! signals one. Only the command *name* is read, never its arguments or
//! environment, which can hold passwords.

use crate::field;
use serde::Serialize;

/// Output kept from the list command.
pub const MAX_LIST_OUTPUT: usize = 512 * 1024;
/// Most processes listed.
pub const MAX_PROCESSES: usize = 500;
/// Highest process id Linux hands out (`pid_max` limit).
const PID_MAX: u32 = 4_194_304;
const MAX_NAME: usize = 64;
const MAX_USER: usize = 32;

/// Lists processes, busiest first, with the command name only (`comm`).
/// Takes no input.
pub const LIST_COMMAND: &str = "export LC_ALL=C; \
ps -eo pid=,ppid=,user=,pcpu=,pmem=,rss=,etimes=,stat=,comm= --sort=-pcpu 2>&1 | head -n 500";

/// One process.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Process {
    pub pid: u32,
    pub ppid: u32,
    pub user: String,
    /// Percent of one CPU, averaged over its life (as `ps` counts it).
    pub cpu: f64,
    /// Percent of memory.
    pub mem: f64,
    pub rss_kb: u64,
    /// Seconds since it started.
    pub elapsed_secs: u64,
    /// `R` running, `S` sleeping, `D` waiting on disk, `Z` zombie…
    pub state: String,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessList {
    pub processes: Vec<Process>,
    pub truncated: bool,
}

/// Reads the output of [`LIST_COMMAND`].
pub fn parse_list(output: &str) -> Result<ProcessList, String> {
    let mut processes = Vec::new();
    let mut truncated = false;
    let mut lines = 0usize;
    for line in output.lines() {
        lines += 1;
        let mut words = line.split_whitespace();
        let (
            Some(pid),
            Some(ppid),
            Some(user),
            Some(cpu),
            Some(mem),
            Some(rss),
            Some(elapsed),
            Some(state),
        ) = (
            words.next(),
            words.next(),
            words.next(),
            words.next(),
            words.next(),
            words.next(),
            words.next(),
            words.next(),
        )
        else {
            continue;
        };
        let (Ok(pid), Ok(ppid)) = (pid.parse::<u32>(), ppid.parse::<u32>()) else {
            continue;
        };
        let percent = |v: &str| v.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0);
        let (Some(cpu), Some(mem)) = (percent(cpu), percent(mem)) else {
            continue;
        };
        if processes.len() >= MAX_PROCESSES {
            truncated = true;
            continue;
        }
        processes.push(Process {
            pid,
            ppid,
            user: field(user, MAX_USER),
            cpu,
            mem: mem.min(100.0),
            rss_kb: rss.parse().unwrap_or(0),
            elapsed_secs: elapsed.parse().unwrap_or(0),
            state: field(state, 8),
            command: field(&words.collect::<Vec<_>>().join(" "), MAX_NAME),
        });
    }
    if processes.is_empty() {
        let said = field(output, 200);
        return Err(if lines == 0 || said.is_empty() {
            "the server did not list any processes".into()
        } else {
            format!("could not read the process list (Setu needs the procps `ps`): {said}")
        });
    }
    Ok(ProcessList {
        processes,
        truncated,
    })
}

/// A signal Setu can send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Ask it to stop (SIGTERM): it can clean up first.
    Term,
    /// Stop it at once (SIGKILL): no clean-up.
    Kill,
}

impl Signal {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "term" => Ok(Signal::Term),
            "kill" => Ok(Signal::Kill),
            _ => Err(format!("{:?} is not a signal Setu sends", field(s, 20))),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Signal::Term => "TERM",
            Signal::Kill => "KILL",
        }
    }
}

/// Checks a process id: a number, not 0 or 1 (init), at most `pid_max`.
pub fn check_pid(pid: u32) -> Result<u32, String> {
    if pid <= 1 || pid > PID_MAX {
        return Err(format!("{pid} is not a process Setu can signal"));
    }
    Ok(pid)
}

/// The exact command that signals a process, as shown before it runs.
pub fn signal_command(signal: Signal, pid: u32, use_sudo: bool) -> Result<String, String> {
    let pid = check_pid(pid)?;
    Ok(format!(
        "{}kill -{} {pid}",
        if use_sudo { "sudo -n " } else { "" },
        signal.name()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PS: &str = "  812     1 www-data  12.5  1.2  20480  3600 Sl   nginx
    1     0 root       0.0  0.3  11000 86400 Ss   systemd
  950   812 www-data   0.1  0.1   4000    10 S    Web Content
 garbage line
  999     1 a-very-long-user-name-that-goes-on 0.0 0.0 0 5 R \u{1b}[31mevil\u{7}
";

    #[test]
    fn the_list_is_read() {
        let list = parse_list(PS).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(list.processes.len(), 4);
        let nginx = &list.processes[0];
        assert_eq!(
            (nginx.pid, nginx.ppid, nginx.user.as_str()),
            (812, 1, "www-data")
        );
        assert_eq!(
            (nginx.cpu, nginx.mem, nginx.rss_kb, nginx.elapsed_secs),
            (12.5, 1.2, 20480, 3600)
        );
        assert_eq!(nginx.command, "nginx");
        // Names with spaces stay whole; escapes are removed.
        assert_eq!(list.processes[2].command, "Web Content");
        assert_eq!(list.processes[3].command, "evil");
        assert!(!list.truncated);
    }

    #[test]
    fn the_list_is_capped() {
        let ps = "10 1 root 0.0 0.0 1 1 S x\n".repeat(MAX_PROCESSES + 5);
        let list = parse_list(&ps).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(list.processes.len(), MAX_PROCESSES);
        assert!(list.truncated);
    }

    #[test]
    fn no_ps_is_explained() {
        assert!(parse_list("ps: unrecognized option: eo\n").is_err_and(|e| e.contains("procps")));
        assert!(parse_list("").is_err());
    }

    #[test]
    fn signals_are_exact() {
        assert_eq!(
            signal_command(Signal::Term, 812, false).as_deref(),
            Ok("kill -TERM 812")
        );
        assert_eq!(
            signal_command(Signal::Kill, 812, true).as_deref(),
            Ok("sudo -n kill -KILL 812")
        );
        assert!(signal_command(Signal::Kill, 1, true).is_err());
        assert!(signal_command(Signal::Kill, 0, true).is_err());
        assert!(signal_command(Signal::Term, PID_MAX + 1, false).is_err());
        assert_eq!(Signal::parse("term"), Ok(Signal::Term));
        assert!(Signal::parse("HUP").is_err());
        assert!(Signal::parse("9; reboot").is_err());
    }
}
