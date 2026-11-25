//! Agentless monitoring: one fixed, read-only command is run on a Linux
//! server over SSH ([`COMMAND`]), and its output is parsed here into a
//! [`Sample`]. Two samples make a [`Snapshot`] with rates (CPU use, network
//! throughput). Nothing is installed on the server.
//!
//! The output comes from the server, so it is **untrusted**: only numbers
//! are read, every list is capped, and the one text field kept (a disk's
//! mount point) is stripped of control characters and shortened.

use serde::Serialize;

/// What is run on the server for each sample. It takes no input, so nothing
/// from the window or from a host's settings ever reaches a shell.
pub const COMMAND: &str = "export LC_ALL=C; \
for f in stat meminfo loadavg uptime net/dev; do echo \"@@$f\"; cat /proc/$f 2>/dev/null; done; \
echo @@df; df -P -k 2>/dev/null";

/// Output kept from one sample.
pub const MAX_OUTPUT: usize = 256 * 1024;
/// Most disks and network interfaces kept.
pub const MAX_DISKS: usize = 24;
const MAX_MOUNT: usize = 120;

/// CPU time counters from the first line of `/proc/stat`, in ticks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CpuTimes {
    pub busy: u64,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Disk {
    pub mount: String,
    pub total_kb: u64,
    pub used_kb: u64,
}

impl Disk {
    pub fn percent(&self) -> f64 {
        percent(self.used_kb, self.total_kb)
    }
}

/// One reading of the server.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sample {
    pub cpu: Option<CpuTimes>,
    pub cpus: u32,
    pub mem_total_kb: u64,
    pub mem_available_kb: u64,
    pub swap_total_kb: u64,
    pub swap_free_kb: u64,
    pub load: Option<[f64; 3]>,
    pub uptime_secs: Option<u64>,
    /// Bytes received and sent on every interface but loopback.
    pub net: Option<(u64, u64)>,
    pub disks: Vec<Disk>,
}

/// Reads the output of [`COMMAND`]. Sections that are missing or garbled
/// are left empty; `None` means nothing at all could be read (not Linux,
/// or not a shell that ran the command).
pub fn parse(output: &str) -> Option<Sample> {
    let mut s = Sample::default();
    let mut section = "";
    let mut any = false;
    for line in output.lines() {
        if let Some(name) = line.strip_prefix("@@") {
            section = match name.trim() {
                "stat" => "stat",
                "meminfo" => "meminfo",
                "loadavg" => "loadavg",
                "uptime" => "uptime",
                "net/dev" => "net",
                "df" => "df",
                _ => "",
            };
            continue;
        }
        let before = s.clone();
        match section {
            "stat" => stat_line(line, &mut s),
            "meminfo" => meminfo_line(line, &mut s),
            "loadavg" => s.load = s.load.or_else(|| loadavg(line)),
            "uptime" => {
                s.uptime_secs = s.uptime_secs.or_else(|| {
                    let secs = line.split_whitespace().next()?.parse::<f64>().ok()?;
                    (secs.is_finite() && secs >= 0.0).then_some(secs as u64)
                })
            }
            "net" => net_line(line, &mut s),
            "df" => df_line(line, &mut s),
            _ => {}
        }
        any |= s != before;
    }
    any.then_some(s)
}

fn numbers(text: &str) -> impl Iterator<Item = Option<u64>> + '_ {
    text.split_whitespace().map(|w| w.parse::<u64>().ok())
}

fn stat_line(line: &str, s: &mut Sample) {
    let mut words = line.split_whitespace();
    let Some(first) = words.next() else { return };
    if first == "cpu" {
        // user nice system idle iowait irq softirq steal (guest is in user).
        let v: Vec<u64> = words.take(8).map_while(|w| w.parse().ok()).collect();
        if v.len() >= 4 {
            let idle = v[3] + v.get(4).copied().unwrap_or(0);
            let total: u64 = v.iter().fold(0u64, |a, b| a.saturating_add(*b));
            s.cpu = Some(CpuTimes {
                busy: total.saturating_sub(idle),
                total,
            });
        }
    } else if first.len() > 3
        && first.starts_with("cpu")
        && first[3..].bytes().all(|b| b.is_ascii_digit())
    {
        s.cpus = s.cpus.saturating_add(1);
    }
}

fn meminfo_line(line: &str, s: &mut Sample) {
    let Some((key, rest)) = line.split_once(':') else {
        return;
    };
    let Some(Some(kb)) = numbers(rest).next() else {
        return;
    };
    match key {
        "MemTotal" => s.mem_total_kb = kb,
        "MemAvailable" => s.mem_available_kb = kb,
        "SwapTotal" => s.swap_total_kb = kb,
        "SwapFree" => s.swap_free_kb = kb,
        _ => {}
    }
}

fn loadavg(line: &str) -> Option<[f64; 3]> {
    let v: Vec<f64> = line
        .split_whitespace()
        .take(3)
        .map_while(|w| w.parse::<f64>().ok().filter(|f| f.is_finite() && *f >= 0.0))
        .collect();
    (v.len() == 3).then(|| [v[0], v[1], v[2]])
}

fn net_line(line: &str, s: &mut Sample) {
    let Some((name, rest)) = line.split_once(':') else {
        return;
    };
    let name = name.trim();
    if name.is_empty() || name == "lo" || name.contains(' ') {
        return;
    }
    let v: Vec<u64> = rest
        .split_whitespace()
        .map_while(|w| w.parse().ok())
        .collect();
    if v.len() < 9 {
        return;
    }
    let (rx, tx) = s.net.unwrap_or((0, 0));
    s.net = Some((rx.saturating_add(v[0]), tx.saturating_add(v[8])));
}

/// Pseudo file systems that are not disks people fill up.
fn skip_fs(fs: &str) -> bool {
    matches!(
        fs,
        "tmpfs"
            | "devtmpfs"
            | "udev"
            | "none"
            | "overlay"
            | "shm"
            | "proc"
            | "sysfs"
            | "cgroup"
            | "efivarfs"
    ) || fs.starts_with("/dev/loop")
}

fn df_line(line: &str, s: &mut Sample) {
    if s.disks.len() >= MAX_DISKS {
        return;
    }
    let mut words = line.split_whitespace();
    let (Some(fs), Some(total), Some(used), Some(_avail), Some(_cap)) = (
        words.next(),
        words.next(),
        words.next(),
        words.next(),
        words.next(),
    ) else {
        return;
    };
    let (Ok(total), Ok(used)) = (total.parse::<u64>(), used.parse::<u64>()) else {
        return; // the header line, or garbage
    };
    let mount: String = words.collect::<Vec<_>>().join(" ");
    if total == 0 || skip_fs(fs) || !mount.starts_with('/') {
        return;
    }
    let mount = clean(&mount);
    if s.disks.iter().any(|d| d.mount == mount) {
        return;
    }
    s.disks.push(Disk {
        mount,
        total_kb: total,
        used_kb: used.min(total),
    });
}

/// Printable characters only, at most [`MAX_MOUNT`] of them.
fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(MAX_MOUNT)
        .collect()
}

fn percent(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        (part as f64 / whole as f64 * 100.0).clamp(0.0, 100.0)
    }
}

/// What the window shows for one reading.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// CPU in use since the last sample, 0-100; `None` for the first one.
    pub cpu_percent: Option<f64>,
    pub cpus: u32,
    pub mem_percent: Option<f64>,
    pub mem_used_kb: u64,
    pub mem_total_kb: u64,
    pub swap_percent: Option<f64>,
    pub load: Option<[f64; 3]>,
    pub uptime_secs: Option<u64>,
    /// Bytes a second, received and sent; `None` for the first one.
    pub net_rx_per_sec: Option<f64>,
    pub net_tx_per_sec: Option<f64>,
    pub disks: Vec<DiskUse>,
    pub alerts: Vec<Alert>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskUse {
    pub mount: String,
    pub total_kb: u64,
    pub used_kb: u64,
    pub percent: f64,
}

/// Limits above which a reading is flagged, in percent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thresholds {
    pub cpu: f64,
    pub mem: f64,
    pub disk: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            cpu: 90.0,
            mem: 90.0,
            disk: 90.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Alert {
    Cpu { percent: f64 },
    Memory { percent: f64 },
    Disk { mount: String, percent: f64 },
}

/// Turns the latest sample (and the one before it, `secs` earlier) into
/// figures, flagging anything over `limits`.
pub fn snapshot(prev: Option<&Sample>, cur: &Sample, secs: f64, limits: Thresholds) -> Snapshot {
    let cpu_percent = match (prev.and_then(|p| p.cpu), cur.cpu) {
        (Some(a), Some(b)) if b.total > a.total && b.busy >= a.busy => {
            Some(percent(b.busy - a.busy, b.total - a.total))
        }
        _ => None,
    };
    let rate = |a: u64, b: u64| (secs > 0.0 && b >= a).then(|| (b - a) as f64 / secs);
    let (net_rx_per_sec, net_tx_per_sec) = match (prev.and_then(|p| p.net), cur.net) {
        (Some((ar, at)), Some((br, bt))) => (rate(ar, br), rate(at, bt)),
        _ => (None, None),
    };
    let mem_used_kb = cur.mem_total_kb.saturating_sub(cur.mem_available_kb);
    let mem_percent = (cur.mem_total_kb > 0).then(|| percent(mem_used_kb, cur.mem_total_kb));
    let swap_percent = (cur.swap_total_kb > 0).then(|| {
        percent(
            cur.swap_total_kb.saturating_sub(cur.swap_free_kb),
            cur.swap_total_kb,
        )
    });
    let disks: Vec<DiskUse> = cur
        .disks
        .iter()
        .map(|d| DiskUse {
            mount: d.mount.clone(),
            total_kb: d.total_kb,
            used_kb: d.used_kb,
            percent: d.percent(),
        })
        .collect();
    let mut alerts = Vec::new();
    if let Some(p) = cpu_percent.filter(|p| *p >= limits.cpu) {
        alerts.push(Alert::Cpu { percent: p });
    }
    if let Some(p) = mem_percent.filter(|p| *p >= limits.mem) {
        alerts.push(Alert::Memory { percent: p });
    }
    for d in disks.iter().filter(|d| d.percent >= limits.disk) {
        alerts.push(Alert::Disk {
            mount: d.mount.clone(),
            percent: d.percent,
        });
    }
    Snapshot {
        cpu_percent,
        cpus: cur.cpus,
        mem_percent,
        mem_used_kb,
        mem_total_kb: cur.mem_total_kb,
        swap_percent,
        load: cur.load,
        uptime_secs: cur.uptime_secs,
        net_rx_per_sec,
        net_tx_per_sec,
        disks,
        alerts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = "@@stat
cpu  1000 0 500 8000 500 0 0 0 0 0
cpu0 500 0 250 4000 250 0 0 0 0 0
cpu1 500 0 250 4000 250 0 0 0 0 0
intr 12345
@@meminfo
MemTotal:        4000000 kB
MemFree:          500000 kB
MemAvailable:    1000000 kB
SwapTotal:       2000000 kB
SwapFree:        1500000 kB
@@loadavg
0.52 0.40 0.31 1/123 4567
@@uptime
86400.25 160000.00
@@net/dev
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 9999 1 0 0 0 0 0 0 9999 1 0 0 0 0 0 0
  eth0: 1000 10 0 0 0 0 0 0 2000 20 0 0 0 0 0 0
  eth1: 500 5 0 0 0 0 0 0 100 1 0 0 0 0 0 0
@@df
Filesystem     1024-blocks     Used Available Capacity Mounted on
tmpfs               100000        0    100000       0% /dev/shm
/dev/sda1         50000000 47500000   2500000      95% /
/dev/sdb1         10000000  1000000   9000000      10% /srv/My Data
/dev/loop0           60000    60000         0     100% /snap/core/1
";

    #[test]
    fn reads_every_section() {
        let s = parse(ONE).expect("sample");
        assert_eq!(
            s.cpu,
            Some(CpuTimes {
                busy: 1500,
                total: 10000
            })
        );
        assert_eq!(s.cpus, 2);
        assert_eq!((s.mem_total_kb, s.mem_available_kb), (4_000_000, 1_000_000));
        assert_eq!((s.swap_total_kb, s.swap_free_kb), (2_000_000, 1_500_000));
        assert_eq!(s.load, Some([0.52, 0.40, 0.31]));
        assert_eq!(s.uptime_secs, Some(86400));
        assert_eq!(s.net, Some((1500, 2100)), "loopback left out");
        let mounts: Vec<&str> = s.disks.iter().map(|d| d.mount.as_str()).collect();
        assert_eq!(
            mounts,
            vec!["/", "/srv/My Data"],
            "tmpfs and loop devices left out"
        );
    }

    #[test]
    fn two_samples_make_rates_and_alerts() {
        let a = parse(ONE).expect("a");
        let b = parse(
            &ONE.replace("cpu  1000 0 500 8000 500", "cpu  1950 0 500 8040 510")
                .replace("eth0: 1000 10", "eth0: 5000 10")
                .replace("MemAvailable:    1000000", "MemAvailable:     200000"),
        )
        .expect("b");
        let first = snapshot(None, &a, 2.0, Thresholds::default());
        assert_eq!(first.cpu_percent, None);
        assert_eq!(first.net_rx_per_sec, None);
        assert_eq!(first.mem_percent, Some(75.0));
        assert_eq!(first.swap_percent, Some(25.0));
        assert_eq!(
            first.alerts,
            vec![Alert::Disk {
                mount: "/".into(),
                percent: 95.0
            }]
        );

        let next = snapshot(Some(&a), &b, 2.0, Thresholds::default());
        // 950 busy ticks out of 1000.
        assert_eq!(next.cpu_percent, Some(95.0));
        assert_eq!(next.net_rx_per_sec, Some(2000.0));
        assert_eq!(next.net_tx_per_sec, Some(0.0));
        assert_eq!(next.mem_percent, Some(95.0));
        assert_eq!(next.alerts.len(), 3, "{:?}", next.alerts);
    }

    #[test]
    fn counters_that_go_backwards_give_no_rate() {
        // A reboot or a counter wrap between samples.
        let a = parse(ONE).expect("a");
        let b = parse(
            &ONE.replace("cpu  1000", "cpu  10")
                .replace("eth0: 1000", "eth0: 1"),
        )
        .expect("b");
        let s = snapshot(Some(&a), &b, 2.0, Thresholds::default());
        assert_eq!(s.cpu_percent, None);
        assert_eq!(s.net_rx_per_sec, None);
    }

    #[test]
    fn untrusted_output_is_handled() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("sh: cat: not found\n<html>"), None);
        // Garbage numbers are skipped, not trusted.
        let s = parse("@@meminfo\nMemTotal: lots kB\n@@loadavg\nNaN inf -1\n@@stat\ncpu x y z\n");
        assert_eq!(s, None);
        // Control characters in a mount point are dropped; length is capped.
        let long = "x".repeat(500);
        let s = parse(&format!(
            "@@df\n/dev/sda1 100 50 50 50% /a\x1b[31mb\n/dev/sda2 100 50 50 50% /{long}\n"
        ))
        .expect("disks");
        assert_eq!(s.disks[0].mount, "/a[31mb");
        assert_eq!(s.disks[1].mount.chars().count(), MAX_MOUNT);
        // Used above total is clamped.
        let s = parse("@@df\n/dev/sda1 100 500 0 100% /\n").expect("disk");
        assert_eq!(s.disks[0].percent(), 100.0);
    }

    #[test]
    fn disk_list_is_capped() {
        let mut text = String::from("@@df\n");
        for i in 0..100 {
            text.push_str(&format!("/dev/sd{i} 100 1 99 1% /m{i}\n"));
        }
        assert_eq!(parse(&text).expect("disks").disks.len(), MAX_DISKS);
    }
}

/// The real command on this computer, as a server's shell would run it.
#[cfg(all(test, target_os = "linux"))]
mod live {
    #[test]
    fn the_command_works_on_linux() {
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(super::COMMAND)
            .output()
            .expect("sh");
        let s = super::parse(&String::from_utf8_lossy(&out.stdout)).expect("a sample");
        assert!(s.cpu.is_some() && s.cpus >= 1, "{s:?}");
        assert!(s.mem_total_kb > 0 && s.load.is_some() && s.uptime_secs.is_some());
        assert!(!s.disks.is_empty(), "{s:?}");
    }
}
