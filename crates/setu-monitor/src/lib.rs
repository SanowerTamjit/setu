//! Agentless monitoring: one fixed, read-only command is run on a Linux
//! server over SSH ([`COMMAND`]), and its output is parsed here into a
//! [`Sample`]. Two samples make a [`Snapshot`] with rates (CPU use, network
//! throughput). Nothing is installed on the server.
//!
//! The output comes from the server, so it is **untrusted**: only numbers
//! are read, every list is capped, and the text kept (mount points, device,
//! interface and process names) is stripped of control characters and
//! shortened.

use serde::Serialize;
use std::collections::HashMap;

/// What is run on the server for each sample. It takes no input, so nothing
/// from the window or from a host's settings ever reaches a shell.
/// Processes come last and are cut at 512 KiB, so a busy server cannot push
/// the other sections out of the output.
pub const COMMAND: &str = "export LC_ALL=C; \
for f in stat meminfo loadavg uptime net/dev diskstats; do echo \"@@$f\"; cat /proc/$f 2>/dev/null; done; \
echo @@df; df -P -k 2>/dev/null; \
echo @@pagesize; getconf PAGESIZE 2>/dev/null; \
echo @@procs; cat /proc/[0-9]*/stat 2>/dev/null | head -c 524288";

/// Output kept from one sample.
pub const MAX_OUTPUT: usize = 1024 * 1024;
/// Most disks, disk devices and network interfaces kept.
pub const MAX_DISKS: usize = 24;
/// Most CPU cores kept.
pub const MAX_CORES: usize = 512;
/// Most processes read from one sample.
pub const MAX_PROCS: usize = 8192;
/// Processes listed in a snapshot.
pub const TOP_PROCS: usize = 12;
const MAX_MOUNT: usize = 120;
const MAX_NAME: usize = 64;

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
    /// Each core's counters, in order (cpu0, cpu1, …).
    pub cores: Vec<CpuTimes>,
    /// Bytes received and sent per interface (loopback left out).
    pub interfaces: Vec<(String, u64, u64)>,
    /// Sectors (512 bytes) read and written per whole disk device.
    pub disk_io: Vec<(String, u64, u64)>,
    pub page_size: Option<u64>,
    pub procs: Vec<ProcStat>,
}

/// One process from `/proc/PID/stat`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcStat {
    pub pid: u32,
    pub command: String,
    /// User plus system CPU time, in ticks.
    pub ticks: u64,
    pub rss_pages: u64,
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
                "diskstats" => "diskstats",
                "df" => "df",
                "pagesize" => "pagesize",
                "procs" => "procs",
                _ => "",
            };
            continue;
        }
        any |= match section {
            "stat" => stat_line(line, &mut s),
            "meminfo" => meminfo_line(line, &mut s),
            "loadavg" => {
                let found = s.load.is_none() && loadavg(line).is_some();
                s.load = s.load.or_else(|| loadavg(line));
                found
            }
            "uptime" => {
                let secs = line
                    .split_whitespace()
                    .next()
                    .and_then(|w| w.parse::<f64>().ok())
                    .filter(|v| v.is_finite() && *v >= 0.0);
                let found = s.uptime_secs.is_none() && secs.is_some();
                s.uptime_secs = s.uptime_secs.or(secs.map(|v| v as u64));
                found
            }
            "net" => net_line(line, &mut s),
            "diskstats" => diskstats_line(line, &mut s),
            "df" => df_line(line, &mut s),
            "pagesize" => {
                let size = line
                    .trim()
                    .parse::<u64>()
                    .ok()
                    .filter(|p| (512..=1 << 20).contains(p));
                s.page_size = s.page_size.or(size);
                false
            }
            "procs" => proc_line(line, &mut s),
            _ => false,
        };
    }
    any.then_some(s)
}

fn numbers(text: &str) -> impl Iterator<Item = Option<u64>> + '_ {
    text.split_whitespace().map(|w| w.parse::<u64>().ok())
}

/// user nice system idle iowait irq softirq steal (guest is in user).
fn cpu_times<'a>(words: impl Iterator<Item = &'a str>) -> Option<CpuTimes> {
    let v: Vec<u64> = words.take(8).map_while(|w| w.parse().ok()).collect();
    (v.len() >= 4).then(|| {
        let idle = v[3].saturating_add(v.get(4).copied().unwrap_or(0));
        let total: u64 = v.iter().fold(0u64, |a, b| a.saturating_add(*b));
        CpuTimes {
            busy: total.saturating_sub(idle),
            total,
        }
    })
}

fn stat_line(line: &str, s: &mut Sample) -> bool {
    let mut words = line.split_whitespace();
    let Some(first) = words.next() else {
        return false;
    };
    if first == "cpu" {
        s.cpu = cpu_times(words);
        s.cpu.is_some()
    } else if first.len() > 3
        && first.starts_with("cpu")
        && first[3..].bytes().all(|b| b.is_ascii_digit())
    {
        s.cpus = s.cpus.saturating_add(1);
        if s.cores.len() < MAX_CORES {
            s.cores.push(cpu_times(words).unwrap_or_default());
        }
        true
    } else {
        false
    }
}

fn meminfo_line(line: &str, s: &mut Sample) -> bool {
    let Some((key, rest)) = line.split_once(':') else {
        return false;
    };
    let Some(Some(kb)) = numbers(rest).next() else {
        return false;
    };
    match key {
        "MemTotal" => s.mem_total_kb = kb,
        "MemAvailable" => s.mem_available_kb = kb,
        "SwapTotal" => s.swap_total_kb = kb,
        "SwapFree" => s.swap_free_kb = kb,
        _ => return false,
    }
    true
}

fn loadavg(line: &str) -> Option<[f64; 3]> {
    let v: Vec<f64> = line
        .split_whitespace()
        .take(3)
        .map_while(|w| w.parse::<f64>().ok().filter(|f| f.is_finite() && *f >= 0.0))
        .collect();
    (v.len() == 3).then(|| [v[0], v[1], v[2]])
}

fn net_line(line: &str, s: &mut Sample) -> bool {
    let Some((name, rest)) = line.split_once(':') else {
        return false;
    };
    let name = name.trim();
    if name.is_empty() || name == "lo" || name.contains(' ') {
        return false;
    }
    let v: Vec<u64> = rest
        .split_whitespace()
        .map_while(|w| w.parse().ok())
        .collect();
    if v.len() < 9 {
        return false;
    }
    let (rx, tx) = s.net.unwrap_or((0, 0));
    s.net = Some((rx.saturating_add(v[0]), tx.saturating_add(v[8])));
    if s.interfaces.len() < MAX_DISKS {
        s.interfaces.push((clean_name(name), v[0], v[8]));
    }
    true
}

/// Whole disks only: partitions would count the same bytes twice, and
/// loop, RAM and optical devices are not disks people watch.
fn whole_disk(name: &str) -> bool {
    let skip = ["loop", "ram", "zram", "sr", "fd", "dm-", "md"];
    if name.is_empty() || skip.iter().any(|p| name.starts_with(p)) {
        return false;
    }
    let digits = name.bytes().rev().take_while(u8::is_ascii_digit).count();
    let stem = &name[..name.len() - digits];
    if name.starts_with("nvme") || name.starts_with("mmcblk") {
        // nvme0n1 and mmcblk0 are disks; nvme0n1p2 and mmcblk0p1 are partitions.
        !(digits > 0
            && stem.ends_with('p')
            && stem.len() > 1
            && stem.as_bytes()[stem.len() - 2].is_ascii_digit())
    } else {
        // sda, vdb, xvdc are disks; sda1 is a partition.
        digits == 0
    }
}

fn diskstats_line(line: &str, s: &mut Sample) -> bool {
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.len() < 10 || s.disk_io.len() >= MAX_DISKS || !whole_disk(words[2]) {
        return false;
    }
    let (Ok(read), Ok(written)) = (words[5].parse::<u64>(), words[9].parse::<u64>()) else {
        return false;
    };
    s.disk_io.push((clean_name(words[2]), read, written));
    true
}

/// `PID (COMMAND) STATE PPID …`: the command may hold spaces and
/// parentheses, so it is everything up to the last `)`.
fn proc_line(line: &str, s: &mut Sample) -> bool {
    if s.procs.len() >= MAX_PROCS {
        return false;
    }
    let (Some(open), Some(close)) = (line.find('('), line.rfind(')')) else {
        return false;
    };
    if close < open {
        return false;
    }
    let Ok(pid) = line[..open].trim().parse::<u32>() else {
        return false;
    };
    let rest: Vec<&str> = line[close + 1..].split_whitespace().collect();
    if rest.len() < 22 {
        return false;
    }
    let num = |i: usize| rest[i].parse::<u64>().ok();
    let (Some(utime), Some(stime), Some(rss)) = (num(11), num(12), num(21)) else {
        return false;
    };
    s.procs.push(ProcStat {
        pid,
        command: clean_name(&line[open + 1..close]),
        ticks: utime.saturating_add(stime),
        rss_pages: rss,
    });
    true
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

fn df_line(line: &str, s: &mut Sample) -> bool {
    if s.disks.len() >= MAX_DISKS {
        return false;
    }
    let mut words = line.split_whitespace();
    let (Some(fs), Some(total), Some(used), Some(_avail), Some(_cap)) = (
        words.next(),
        words.next(),
        words.next(),
        words.next(),
        words.next(),
    ) else {
        return false;
    };
    let (Ok(total), Ok(used)) = (total.parse::<u64>(), used.parse::<u64>()) else {
        return false; // the header line, or garbage
    };
    let mount: String = words.collect::<Vec<_>>().join(" ");
    if total == 0 || skip_fs(fs) || !mount.starts_with('/') {
        return false;
    }
    let mount = clean(&mount);
    if s.disks.iter().any(|d| d.mount == mount) {
        return false;
    }
    s.disks.push(Disk {
        mount,
        total_kb: total,
        used_kb: used.min(total),
    });
    true
}

/// Printable characters only, at most [`MAX_NAME`] of them.
fn clean_name(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect()
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
    /// Each core's use since the last sample, 0-100; empty for the first.
    pub core_percents: Vec<f64>,
    pub interfaces: Vec<InterfaceRate>,
    pub disk_io: Vec<DiskIoRate>,
    /// The busiest processes (by CPU, then memory).
    pub processes: Vec<ProcessUse>,
    /// How many processes the server is running.
    pub process_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InterfaceRate {
    pub name: String,
    pub rx_per_sec: Option<f64>,
    pub tx_per_sec: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskIoRate {
    pub device: String,
    /// Bytes a second.
    pub read_per_sec: Option<f64>,
    pub write_per_sec: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessUse {
    pub pid: u32,
    pub command: String,
    /// Like top: 100 is one whole core. `None` for the first sample.
    pub cpu_percent: Option<f64>,
    pub rss_kb: u64,
    /// Share of all memory, 0-100.
    pub mem_percent: Option<f64>,
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
    let core_percents = match prev {
        Some(p) if p.cores.len() == cur.cores.len() => p
            .cores
            .iter()
            .zip(&cur.cores)
            .map(|(a, b)| {
                if b.total > a.total && b.busy >= a.busy {
                    percent(b.busy - a.busy, b.total - a.total)
                } else {
                    0.0
                }
            })
            .collect(),
        _ => Vec::new(),
    };
    let interfaces = cur
        .interfaces
        .iter()
        .map(|(name, rx, tx)| {
            let before = prev.and_then(|p| p.interfaces.iter().find(|i| &i.0 == name));
            InterfaceRate {
                name: name.clone(),
                rx_per_sec: before.and_then(|b| rate(b.1, *rx)),
                tx_per_sec: before.and_then(|b| rate(b.2, *tx)),
            }
        })
        .collect();
    let disk_io = cur
        .disk_io
        .iter()
        .map(|(device, read, written)| {
            let before = prev.and_then(|p| p.disk_io.iter().find(|d| &d.0 == device));
            let bytes = |a: u64, b: u64| rate(a, b).map(|r| r * 512.0);
            DiskIoRate {
                device: device.clone(),
                read_per_sec: before.and_then(|b| bytes(b.1, *read)),
                write_per_sec: before.and_then(|b| bytes(b.2, *written)),
            }
        })
        .collect();
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
        core_percents,
        interfaces,
        disk_io,
        processes: top_processes(prev, cur),
        process_count: cur.procs.len(),
    }
}

/// The busiest processes. CPU is the process's share of all CPU time since
/// the last sample, times the number of cores (so 100 is one core, as in
/// top); without a previous sample they are ordered by memory.
fn top_processes(prev: Option<&Sample>, cur: &Sample) -> Vec<ProcessUse> {
    let page_kb = cur.page_size.unwrap_or(4096) / 1024;
    let cpu_ticks = match (prev.and_then(|p| p.cpu), cur.cpu) {
        (Some(a), Some(b)) if b.total > a.total => Some(b.total - a.total),
        _ => None,
    };
    let before: HashMap<u32, &ProcStat> = prev
        .map(|p| p.procs.iter().map(|q| (q.pid, q)).collect())
        .unwrap_or_default();
    let cores = f64::from(cur.cpus.max(1));
    let mut list: Vec<ProcessUse> = cur
        .procs
        .iter()
        .map(|q| {
            let cpu_percent = match (before.get(&q.pid), cpu_ticks) {
                // Same process (a reused pid would have a new name or fewer ticks).
                (Some(b), Some(total)) if b.command == q.command && q.ticks >= b.ticks => Some(
                    ((q.ticks - b.ticks) as f64 / total as f64 * 100.0 * cores)
                        .clamp(0.0, 100.0 * cores),
                ),
                _ => None,
            };
            let rss_kb = q.rss_pages.saturating_mul(page_kb);
            ProcessUse {
                pid: q.pid,
                command: q.command.clone(),
                cpu_percent,
                rss_kb,
                mem_percent: (cur.mem_total_kb > 0).then(|| percent(rss_kb, cur.mem_total_kb)),
            }
        })
        .collect();
    list.sort_by(|a, b| {
        let cpu = |p: &ProcessUse| p.cpu_percent.unwrap_or(-1.0);
        cpu(b)
            .partial_cmp(&cpu(a))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.rss_kb.cmp(&a.rss_kb))
    });
    list.truncate(TOP_PROCS);
    list
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

    const DETAIL: &str = "@@stat
cpu  1000 0 0 1000 0 0 0 0 0 0
cpu0 900 0 0 100 0 0 0 0 0 0
cpu1 100 0 0 900 0 0 0 0 0 0
@@meminfo
MemTotal:        1000000 kB
MemAvailable:     500000 kB
@@net/dev
    lo: 9 1 0 0 0 0 0 0 9 1 0 0 0 0 0 0
  eth0: 1000 10 0 0 0 0 0 0 2000 20 0 0 0 0 0 0
  wg0: 10 1 0 0 0 0 0 0 20 1 0 0 0 0 0 0
@@diskstats
   8       0 sda 100 0 2000 0 50 0 4000 0 0 0 0
   8       1 sda1 100 0 2000 0 50 0 4000 0 0 0 0
 259       0 nvme0n1 10 0 100 0 10 0 100 0 0 0 0
 259       1 nvme0n1p1 10 0 100 0 10 0 100 0 0 0 0
   7       0 loop0 1 0 8 0 0 0 0 0 0 0 0
 179       0 mmcblk0 1 0 8 0 0 0 0 0 0 0 0
 179       1 mmcblk0p1 1 0 8 0 0 0 0 0 0 0 0
@@pagesize
4096
@@procs
1 (systemd) S 0 1 1 0 -1 4194560 1 1 0 0 50 50 0 0 20 0 1 0 1 1000 2500 x
42 (my (odd) app) R 1 42 42 0 -1 0 0 0 0 0 100 0 0 0 20 0 1 0 2 1000 25000 x
77 (bad\x1bname) S 1 77 77 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 3 1000 10 x
garbage line
";

    #[test]
    fn reads_cores_interfaces_disks_and_processes() {
        let s = parse(DETAIL).expect("sample");
        assert_eq!(s.cores.len(), 2);
        assert_eq!(
            s.interfaces,
            vec![("eth0".into(), 1000, 2000), ("wg0".into(), 10, 20)]
        );
        let devices: Vec<&str> = s.disk_io.iter().map(|d| d.0.as_str()).collect();
        assert_eq!(
            devices,
            vec!["sda", "nvme0n1", "mmcblk0"],
            "partitions and loop devices left out"
        );
        assert_eq!(s.page_size, Some(4096));
        assert_eq!(s.procs.len(), 3);
        assert_eq!(s.procs[1].command, "my (odd) app");
        assert_eq!(s.procs[1].ticks, 100);
        assert_eq!(s.procs[1].rss_pages, 25000);
    }

    #[test]
    fn rates_and_top_processes_from_two_samples() {
        let a = parse(DETAIL).expect("a");
        let b = parse(
            &DETAIL
                .replace("cpu  1000 0 0 1000", "cpu  1100 0 0 1100")
                .replace("cpu0 900 0 0 100", "cpu0 1000 0 0 100")
                .replace("cpu1 100 0 0 900", "cpu1 100 0 0 1000")
                .replace("eth0: 1000 10", "eth0: 3000 10")
                .replace("sda 100 0 2000 0 50 0 4000", "sda 100 0 2000 0 60 0 8000")
                .replace("0 0 0 0 0 100 0 0 0 20", "0 0 0 0 0 150 0 0 0 20"),
        )
        .expect("b");
        let s = snapshot(Some(&a), &b, 2.0, Thresholds::default());
        assert_eq!(s.core_percents, vec![100.0, 0.0]);
        assert_eq!(s.interfaces[0].rx_per_sec, Some(1000.0));
        assert_eq!(s.disk_io[0].write_per_sec, Some(4000.0 * 512.0 / 2.0));
        assert_eq!(s.disk_io[0].read_per_sec, Some(0.0));
        // 50 of 200 CPU ticks on a 2-core machine is half of all, so one whole core.
        assert_eq!(s.processes[0].command, "my (odd) app");
        assert_eq!(s.processes[0].cpu_percent, Some(50.0));
        assert_eq!(s.processes[0].rss_kb, 100_000);
        assert_eq!(s.processes[0].mem_percent, Some(10.0));
        assert_eq!(s.process_count, 3);

        // First sample: no rates, processes ordered by memory.
        let first = snapshot(None, &a, 0.0, Thresholds::default());
        assert!(first.core_percents.is_empty());
        assert_eq!(first.interfaces[0].rx_per_sec, None);
        assert_eq!(first.processes[0].pid, 42);
        assert_eq!(first.processes[0].cpu_percent, None);
    }

    #[test]
    fn process_names_are_cleaned_and_lists_capped() {
        // DETAIL holds an escape character in this process's name.
        let s = parse(DETAIL).expect("sample");
        assert_eq!(s.procs[2].command, "badname");
        let mut text = String::from("@@procs\n");
        for i in 0..(MAX_PROCS + 10) {
            text.push_str(&format!(
                "{i} (p) S 0 0 0 0 -1 0 0 0 0 0 1 1 0 0 20 0 1 0 1 1 1 x\n"
            ));
        }
        let s = parse(&text).expect("procs");
        assert_eq!(s.procs.len(), MAX_PROCS);
        assert_eq!(
            snapshot(None, &s, 0.0, Thresholds::default())
                .processes
                .len(),
            TOP_PROCS
        );
    }

    #[test]
    fn whole_disks_only() {
        for yes in ["sda", "vdb", "xvdc", "nvme0n1", "nvme10n2", "mmcblk0"] {
            assert!(whole_disk(yes), "{yes}");
        }
        for no in [
            "sda1",
            "nvme0n1p1",
            "mmcblk0p2",
            "loop3",
            "ram0",
            "zram0",
            "sr0",
            "dm-0",
            "md127",
            "",
        ] {
            assert!(!whole_disk(no), "{no}");
        }
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
        assert_eq!(s.cores.len(), s.cpus as usize);
        assert!(
            s.procs.iter().any(|p| p.pid == std::process::id()),
            "this test's own process"
        );
        assert!(s.page_size.is_some());
    }
}
