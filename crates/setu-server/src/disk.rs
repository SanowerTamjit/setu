//! Disk usage: how full each file system is (`df`), and which folders in a
//! folder take the space (`du`, one level deep, on one file system).
//! Both only read. The folder is the one input; [`check_path`] lets
//! through absolute paths without quotes or control characters, and it is
//! put in single quotes.

use crate::field;
use serde::Serialize;

/// Output kept from `df`.
pub const MAX_DF_OUTPUT: usize = 64 * 1024;
/// Output kept from `du`.
pub const MAX_DU_OUTPUT: usize = 64 * 1024;
/// Most file systems listed.
pub const MAX_FILESYSTEMS: usize = 64;
/// Most folders listed for one folder (the biggest).
pub const MAX_FOLDERS: usize = 40;
/// Longest a folder scan may run on the server, in seconds.
pub const DU_SECONDS: u32 = 60;
const MAX_PATH: usize = 1024;

/// Lists file systems with their type. Takes no input.
pub const DF_COMMAND: &str = "export LC_ALL=C; df -P -k -T 2>/dev/null";

/// File system types that are not disks.
const PSEUDO: &[&str] = &[
    "tmpfs",
    "devtmpfs",
    "squashfs",
    "proc",
    "sysfs",
    "devpts",
    "cgroup",
    "cgroup2",
    "efivarfs",
    "ramfs",
    "securityfs",
    "debugfs",
    "tracefs",
    "mqueue",
    "hugetlbfs",
    "fusectl",
    "configfs",
    "pstore",
    "bpf",
    "autofs",
    "nsfs",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Filesystem {
    pub device: String,
    pub kind: String,
    pub mount: String,
    pub total_kb: u64,
    pub used_kb: u64,
    pub available_kb: u64,
}

/// Reads the output of [`DF_COMMAND`]: real file systems only.
pub fn parse_df(output: &str) -> Vec<Filesystem> {
    let mut out: Vec<Filesystem> = Vec::new();
    for line in output.lines().skip(1) {
        let mut words = line.split_whitespace();
        let (Some(device), Some(kind), Some(total), Some(used), Some(avail), Some(_cap)) = (
            words.next(),
            words.next(),
            words.next(),
            words.next(),
            words.next(),
            words.next(),
        ) else {
            continue;
        };
        let mount = words.collect::<Vec<_>>().join(" ");
        let (Ok(total), Ok(used), Ok(avail)) = (
            total.parse::<u64>(),
            used.parse::<u64>(),
            avail.parse::<u64>(),
        ) else {
            continue;
        };
        if total == 0
            || PSEUDO.contains(&kind)
            || !mount.starts_with('/')
            || device.starts_with("/dev/loop")
        {
            continue;
        }
        if out.iter().any(|f| f.mount == mount) {
            continue;
        }
        if out.len() >= MAX_FILESYSTEMS {
            break;
        }
        out.push(Filesystem {
            device: field(device, 120),
            kind: field(kind, 24),
            mount: field(&mount, 200),
            total_kb: total,
            used_kb: used.min(total),
            available_kb: avail,
        });
    }
    out
}

/// Checks a folder to measure: absolute, at most 1024 bytes, no quotes,
/// control characters or bidirectional overrides. Trailing slashes go
/// (except for `/`). Returns the cleaned path.
pub fn check_path(path: &str) -> Result<String, String> {
    let bad = || format!("{:?} is not a folder path (like /var/log)", field(path, 80));
    if !path.starts_with('/') || path.len() > MAX_PATH {
        return Err(bad());
    }
    if path.chars().any(|c| {
        c.is_control()
            || c == '\''
            || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    }) {
        return Err(bad());
    }
    let trimmed = path.trim_end_matches('/');
    Ok(if trimmed.is_empty() {
        "/".into()
    } else {
        trimmed.into()
    })
}

/// The command that measures the folders in `path`, biggest first.
pub fn du_command(path: &str, use_sudo: bool) -> Result<String, String> {
    let path = check_path(path)?;
    Ok(format!(
        "export LC_ALL=C; {}timeout {DU_SECONDS} du -x -k -d 1 -- '{path}' 2>/dev/null | sort -rn | head -n {}",
        if use_sudo { "sudo -n " } else { "" },
        MAX_FOLDERS + 1
    ))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    /// The name inside the measured folder.
    pub name: String,
    pub kb: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderSizes {
    pub path: String,
    /// The whole folder, when `du` got to the end.
    pub total_kb: Option<u64>,
    pub folders: Vec<Folder>,
    /// The command that measured it, to show beside the result.
    pub command: String,
}

/// Reads `du` output for `path` (already checked). Lines whose path is not
/// directly inside `path` are ignored, so a server cannot slip in names.
pub fn parse_du(path: &str, output: &str, command: String) -> FolderSizes {
    let prefix = if path == "/" {
        "/".to_string()
    } else {
        format!("{path}/")
    };
    let mut total_kb = None;
    let mut folders = Vec::new();
    for line in output.lines() {
        let Some((kb, name)) = line.split_once('\t') else {
            continue;
        };
        let Ok(kb) = kb.trim().parse::<u64>() else {
            continue;
        };
        if name == path {
            total_kb = Some(kb);
            continue;
        }
        let Some(child) = name.strip_prefix(&prefix) else {
            continue;
        };
        if child.is_empty() || child.contains('/') || folders.len() >= MAX_FOLDERS {
            continue;
        }
        folders.push(Folder {
            name: field(child, 200),
            kb,
        });
    }
    folders.sort_by_key(|f| std::cmp::Reverse(f.kb));
    FolderSizes {
        path: path.to_string(),
        total_kb,
        folders,
        command,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DF: &str = "Filesystem     Type     1024-blocks      Used Available Capacity Mounted on
/dev/sda1      ext4        41152736  30000000   9000000      77% /
tmpfs          tmpfs        1000000         0   1000000       0% /run
/dev/sdb1      xfs        104857600  50000000  54857600      48% /var/lib/my data
/dev/loop3     squashfs       56832     56832         0     100% /snap/core/1
overlay        overlay     41152736  30000000   9000000      77% /var/lib/docker/overlay2/x/merged
bad line
";

    #[test]
    fn real_file_systems_are_kept() {
        let fs = parse_df(DF);
        let mounts: Vec<_> = fs.iter().map(|f| f.mount.as_str()).collect();
        assert_eq!(
            mounts,
            ["/", "/var/lib/my data", "/var/lib/docker/overlay2/x/merged"]
        );
        assert_eq!(
            (fs[0].total_kb, fs[0].used_kb, fs[0].kind.as_str()),
            (41_152_736, 30_000_000, "ext4")
        );
    }

    #[test]
    fn paths_are_checked() {
        assert_eq!(check_path("/var/log/").as_deref(), Ok("/var/log"));
        assert_eq!(check_path("/").as_deref(), Ok("/"));
        assert_eq!(check_path("///").as_deref(), Ok("/"));
        assert_eq!(check_path("/srv/my data").as_deref(), Ok("/srv/my data"));
        for bad in [
            "var/log",
            "",
            "~/x",
            "/a'b",
            "/a\nb",
            "/a\u{0}b",
            "/a\u{202e}b",
        ] {
            assert!(check_path(bad).is_err(), "{bad:?}");
        }
        assert!(check_path(&format!("/{}", "a".repeat(1100))).is_err());
    }

    #[test]
    fn the_du_command_quotes_the_path() {
        let c = du_command("/var/lib/", true).unwrap_or_default();
        assert!(c.contains("sudo -n timeout 60 du -x -k -d 1 -- '/var/lib' 2>/dev/null | sort -rn | head -n 41"), "{c}");
        // $ and ; are plain text inside single quotes.
        let c = du_command("/x;$(reboot)", false).unwrap_or_default();
        assert!(c.contains("-- '/x;$(reboot)'"), "{c}");
        assert!(du_command("/a'; reboot; '", false).is_err());
    }

    #[test]
    fn du_output_is_read() {
        let out = "900\t/var\n500\t/var/lib\n300\t/var/log\n50\t/var/cache\n10\t/etc/passwd\n5\t/var/lib/deep\nxx\t/var/x\n";
        let r = parse_du("/var", out, "cmd".into());
        assert_eq!(r.total_kb, Some(900));
        let names: Vec<_> = r.folders.iter().map(|f| (f.name.as_str(), f.kb)).collect();
        assert_eq!(names, [("lib", 500), ("log", 300), ("cache", 50)]);
        let root = parse_du("/", "100\t/\n60\t/usr\n", "cmd".into());
        assert_eq!(root.folders[0].name, "usr");
        assert_eq!(root.total_kb, Some(100));
    }
}
