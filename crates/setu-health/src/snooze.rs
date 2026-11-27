//! Snoozed findings: hidden from the list until a time, kept in
//! `problems.json` in Setu's config folder.
//!
//! ```json
//! { "version": 1, "snoozed": { "unit:web1:nginx.service": 1764280800000 } }
//! ```
//!
//! The file holds finding ids (host names, unit names, mount points) and
//! times, nothing else. It is written atomically (temp file + rename) and
//! is 0600 on Unix. A broken file is treated as empty rather than stopping
//! the Problems view, and is replaced on the next snooze.

use crate::Finding;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const FILE: &str = "problems.json";
const VERSION: u32 = 1;
/// Most snoozes kept; the ones ending soonest go first.
pub const MAX_SNOOZES: usize = 1000;
/// Longest a snooze may last.
pub const MAX_MINUTES: u64 = 90 * 24 * 60;
/// Longest finding id accepted.
const MAX_ID: usize = 600;

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    version: u32,
    #[serde(default)]
    snoozed: BTreeMap<String, u64>,
}

/// The snoozes in one config folder.
#[derive(Debug, Clone)]
pub struct Snoozes {
    path: PathBuf,
}

/// Checks a finding id the window sent back: short, one line, no control
/// characters.
pub fn check_id(id: &str) -> Result<&str, String> {
    if id.is_empty() || id.len() > MAX_ID || id.chars().any(char::is_control) {
        return Err("that is not a problem Setu listed".into());
    }
    Ok(id)
}

impl Snoozes {
    pub fn in_dir(config_dir: &Path) -> Self {
        Snoozes {
            path: config_dir.join(FILE),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Snoozes still in force at `now_ms`, by finding id.
    pub fn active(&self, now_ms: u64) -> BTreeMap<String, u64> {
        let mut map = self.read();
        map.retain(|_, until| *until > now_ms);
        map
    }

    fn read(&self) -> BTreeMap<String, u64> {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|t| serde_json::from_str::<File>(&t).ok())
            .filter(|f| f.version <= VERSION)
            .map(|f| f.snoozed)
            .unwrap_or_default()
    }

    fn write(&self, mut map: BTreeMap<String, u64>, now_ms: u64) -> Result<(), String> {
        map.retain(|_, until| *until > now_ms);
        if map.len() > MAX_SNOOZES {
            let mut by_end: Vec<(String, u64)> = map.into_iter().collect();
            by_end.sort_by_key(|e| std::cmp::Reverse(e.1));
            by_end.truncate(MAX_SNOOZES);
            map = by_end.into_iter().collect();
        }
        let text = serde_json::to_string_pretty(&File {
            version: VERSION,
            snoozed: map,
        })
        .map_err(|e| e.to_string())?;
        write_private(&self.path, text.as_bytes())
            .map_err(|e| format!("could not save {}: {e}", self.path.display()))
    }

    /// Hides `id` for `minutes` (1 minute to 90 days) from `now_ms`.
    /// Returns when it ends.
    pub fn snooze(&self, id: &str, minutes: u64, now_ms: u64) -> Result<u64, String> {
        let id = check_id(id)?;
        let until = now_ms.saturating_add(minutes.clamp(1, MAX_MINUTES) * 60_000);
        let mut map = self.read();
        map.insert(id.to_string(), until);
        self.write(map, now_ms)?;
        Ok(until)
    }

    /// Shows `id` again.
    pub fn wake(&self, id: &str, now_ms: u64) -> Result<(), String> {
        let id = check_id(id)?;
        let mut map = self.read();
        if map.remove(id).is_some() {
            self.write(map, now_ms)?;
        }
        Ok(())
    }
}

/// A finding that is snoozed, and until when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snoozed {
    #[serde(flatten)]
    pub finding: Finding,
    pub until_ms: u64,
}

/// Splits findings into the ones to show and the snoozed ones.
pub fn split(
    findings: Vec<Finding>,
    active: &BTreeMap<String, u64>,
) -> (Vec<Finding>, Vec<Snoozed>) {
    let mut shown = Vec::new();
    let mut snoozed = Vec::new();
    for f in findings {
        match active.get(&f.id) {
            Some(&until_ms) => snoozed.push(Snoozed {
                finding: f,
                until_ms,
            }),
            None => shown.push(f),
        }
    }
    (shown, snoozed)
}

fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{finding, Severity};

    #[test]
    fn snoozes_last_until_their_time() {
        let dir = tempfile::tempdir().expect("temp dir");
        let s = Snoozes::in_dir(dir.path());
        assert!(s.active(0).is_empty(), "no file yet");
        let until = s
            .snooze("unit:web1:nginx.service", 60, 1_000)
            .expect("snooze");
        assert_eq!(until, 1_000 + 3_600_000);
        assert_eq!(s.active(2_000).get("unit:web1:nginx.service"), Some(&until));
        assert!(s.active(until).is_empty(), "over at its time");
        s.wake("unit:web1:nginx.service", 2_000).expect("wake");
        assert!(s.active(2_000).is_empty());
    }

    #[test]
    fn lengths_are_clamped_and_ids_checked() {
        let dir = tempfile::tempdir().expect("temp dir");
        let s = Snoozes::in_dir(dir.path());
        assert_eq!(s.snooze("a", 0, 0).expect("snooze"), 60_000);
        assert_eq!(
            s.snooze("b", u64::MAX, 0).expect("snooze"),
            MAX_MINUTES * 60_000
        );
        assert!(s.snooze("", 5, 0).is_err());
        assert!(s.snooze("a\nb", 5, 0).is_err());
        assert!(s.snooze(&"x".repeat(700), 5, 0).is_err());
    }

    #[test]
    fn expired_snoozes_are_dropped_and_the_list_is_capped() {
        let dir = tempfile::tempdir().expect("temp dir");
        let s = Snoozes::in_dir(dir.path());
        s.snooze("old", 1, 0).expect("snooze");
        // Written an hour later: "old" has ended and is not kept.
        s.snooze("new", 10, 3_600_000).expect("snooze");
        let text = std::fs::read_to_string(s.path()).expect("file");
        assert!(!text.contains("\"old\""));
        let mut map = BTreeMap::new();
        for i in 0..(MAX_SNOOZES as u64 + 50) {
            map.insert(format!("id{i}"), 10_000 + i);
        }
        s.write(map, 0).expect("write");
        let kept = s.active(0);
        assert_eq!(kept.len(), MAX_SNOOZES);
        assert!(!kept.contains_key("id0"), "the ones ending soonest go");
    }

    #[test]
    fn a_broken_file_counts_as_empty() {
        let dir = tempfile::tempdir().expect("temp dir");
        let s = Snoozes::in_dir(dir.path());
        std::fs::write(s.path(), "{ not json").expect("write");
        assert!(s.active(0).is_empty());
        s.snooze("a", 5, 0).expect("a snooze replaces it");
        assert_eq!(s.active(0).len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("temp dir");
        let s = Snoozes::in_dir(&dir.path().join("setu"));
        s.snooze("a", 5, 0).expect("snooze");
        let mode = |p: &Path| {
            std::fs::metadata(p)
                .map(|m| m.permissions().mode() & 0o777)
                .unwrap_or(0)
        };
        assert_eq!(mode(s.path()), 0o600);
        assert_eq!(mode(&dir.path().join("setu")), 0o700);
    }

    #[test]
    fn split_puts_snoozed_findings_aside() {
        let list = vec![
            finding("a", Severity::Error, None, "a"),
            finding("b", Severity::Warning, None, "b"),
        ];
        let mut active = BTreeMap::new();
        active.insert("b".to_string(), 99);
        let (shown, snoozed) = split(list, &active);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].id, "a");
        assert_eq!(snoozed[0].finding.id, "b");
        assert_eq!(snoozed[0].until_ms, 99);
    }
}
