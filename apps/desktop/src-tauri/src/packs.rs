//! Packs: bundles that add to Setu, starting with role packs of snippets.
//!
//! A pack is one JSON manifest (format 1): an id, name, version, the
//! permissions it asks for, the systems it supports, and what it adds.
//! Built-in packs ship inside the app; others are installed from a local
//! file into `packs/` in Setu's config folder. Packs hold data only in this
//! format, nothing that runs, so the permission list is empty for now and
//! shown before install for when packs can do more.

use serde::{Deserialize, Serialize};
use setu_core::paths::{self, Os, ProcessEnv};
use std::path::PathBuf;

const BUILT_IN: &[&str] = &[include_str!("../packs/sysadmin.json")];

const MAX_BYTES: usize = 1024 * 1024;
const MAX_SNIPPETS: usize = 1000;
const MAX_COMMAND: usize = 8000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    pub id: String,
    pub title: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contributes {
    #[serde(default)]
    pub snippets: Vec<Snippet>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// `role-pack` or `snippets`.
    pub kind: String,
    #[serde(default)]
    pub roles: Vec<String>,
    /// `linux`, `macos`, `windows`; empty means all.
    #[serde(default)]
    pub os: Vec<String>,
    /// What the pack asks to be allowed to do, shown before install.
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub dependencies: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub contributes: Contributes,
}

fn valid_id(id: &str) -> bool {
    let mut parts = id.split('.');
    let ok_part = |p: &str| {
        !p.is_empty()
            && p.len() <= 40
            && p.starts_with(|c: char| c.is_ascii_lowercase())
            && p.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };
    let first = parts.next().is_some_and(ok_part);
    let rest: Vec<&str> = parts.collect();
    first && !rest.is_empty() && rest.iter().all(|p| ok_part(p))
}

fn valid_version(v: &str) -> bool {
    let core = v.split(['-', '+']).next().unwrap_or_default();
    let nums: Vec<&str> = core.split('.').collect();
    nums.len() == 3
        && nums
            .iter()
            .all(|n| !n.is_empty() && n.len() <= 9 && n.chars().all(|c| c.is_ascii_digit()))
}

/// Reads and checks a manifest. Errors are worded for the person installing.
pub fn parse(text: &str) -> Result<Manifest, String> {
    if text.len() > MAX_BYTES {
        return Err("the pack is over 1 MB".into());
    }
    let m: Manifest = serde_json::from_str(text).map_err(|e| format!("not a Setu pack: {e}"))?;
    if m.format != 1 {
        return Err(format!(
            "this pack uses format {}, which this version of Setu does not read",
            m.format
        ));
    }
    if !valid_id(&m.id) {
        return Err(format!(
            "`{}` is not a valid pack id (use a dotted name like acme.web-tools)",
            m.id
        ));
    }
    if m.name.trim().is_empty() || m.name.len() > 80 {
        return Err("the pack needs a name of up to 80 characters".into());
    }
    if !valid_version(&m.version) {
        return Err(format!("`{}` is not a version like 1.2.0", m.version));
    }
    if m.kind != "role-pack" && m.kind != "snippets" {
        return Err(format!("Setu does not know packs of kind `{}` yet", m.kind));
    }
    if !m.dependencies.is_empty() {
        return Err(
            "this pack depends on other packs, which this version of Setu cannot install yet"
                .into(),
        );
    }
    for os in &m.os {
        if !matches!(os.as_str(), "linux" | "macos" | "windows") {
            return Err(format!("unknown system `{os}` in os"));
        }
    }
    let snippets = &m.contributes.snippets;
    if snippets.len() > MAX_SNIPPETS {
        return Err(format!("the pack has more than {MAX_SNIPPETS} snippets"));
    }
    let mut seen = std::collections::BTreeSet::new();
    for s in snippets {
        if s.id.is_empty() || !seen.insert(s.id.as_str()) {
            return Err(format!(
                "snippet ids must be unique and not empty (`{}`)",
                s.id
            ));
        }
        if s.title.trim().is_empty() || s.command.trim().is_empty() {
            return Err(format!("snippet `{}` needs a title and a command", s.id));
        }
        if s.command.len() > MAX_COMMAND {
            return Err(format!("snippet `{}` is too long", s.id));
        }
        if s.command
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return Err(format!("snippet `{}` contains control characters", s.id));
        }
    }
    Ok(m)
}

fn packs_dir() -> Result<PathBuf, String> {
    paths::config_dir(&ProcessEnv, Os::current())
        .map(|d| d.join("packs"))
        .map_err(|e| e.to_string())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackView {
    #[serde(flatten)]
    manifest: Manifest,
    built_in: bool,
}

fn built_in() -> Vec<Manifest> {
    BUILT_IN.iter().filter_map(|t| parse(t).ok()).collect()
}

/// Built-in packs, then installed ones. A broken file is skipped, not fatal.
#[tauri::command]
pub fn packs_list() -> Vec<PackView> {
    let mut out: Vec<PackView> = built_in()
        .into_iter()
        .map(|manifest| PackView {
            manifest,
            built_in: true,
        })
        .collect();
    if let Ok(entries) = packs_dir().and_then(|d| std::fs::read_dir(d).map_err(|e| e.to_string())) {
        let mut installed: Vec<Manifest> = entries
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|t| parse(&t).ok())
            .filter(|m| !out.iter().any(|p| p.manifest.id == m.id))
            .collect();
        installed.sort_by(|a, b| a.name.cmp(&b.name));
        out.extend(installed.into_iter().map(|manifest| PackView {
            manifest,
            built_in: false,
        }));
    }
    out
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inspected {
    manifest: Manifest,
    /// The version installed now, when this would replace it.
    replaces: Option<String>,
}

/// Checks a pack file before install, so its permissions can be shown.
#[tauri::command]
pub fn pack_inspect(text: String) -> Result<Inspected, String> {
    let manifest = parse(&text)?;
    if built_in().iter().any(|b| b.id == manifest.id) {
        return Err(format!(
            "`{}` is built into Setu and cannot be replaced",
            manifest.id
        ));
    }
    let replaces = packs_list()
        .into_iter()
        .find(|p| p.manifest.id == manifest.id)
        .map(|p| p.manifest.version);
    Ok(Inspected { manifest, replaces })
}

/// Installs (or upgrades) a pack from the text of its manifest.
#[tauri::command]
pub fn pack_install(text: String) -> Result<Manifest, String> {
    let Inspected { manifest, .. } = pack_inspect(text)?;
    let dir = packs_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.json", manifest.id));
    let tmp = dir.join(format!(".{}.json.tmp", manifest.id));
    // Written from the checked manifest, not the raw file.
    let json = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(manifest)
}

#[tauri::command]
pub fn pack_remove(id: String) -> Result<(), String> {
    if !valid_id(&id) {
        return Err("not a pack id".into());
    }
    if built_in().iter().any(|b| b.id == id) {
        return Err("built-in packs can be turned off, not removed".into());
    }
    let path = packs_dir()?.join(format!("{id}.json"));
    std::fs::remove_file(&path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_packs_are_valid() {
        for text in BUILT_IN {
            let m = parse(text).expect("built-in pack parses");
            assert!(m.contributes.snippets.len() >= 20);
        }
    }

    #[test]
    fn checks_ids_versions_and_snippets() {
        let pack = |id: &str, version: &str, command: &str| {
            format!(
                r#"{{"format":1,"id":"{id}","name":"N","version":"{version}","kind":"snippets",
                "contributes":{{"snippets":[{{"id":"a","title":"A","command":"{command}"}}]}}}}"#
            )
        };
        assert!(parse(&pack("acme.tools", "1.0.0", "ls")).is_ok());
        assert!(parse(&pack("acme.tools", "1.0.0-beta.1", "ls")).is_ok());
        assert!(parse(&pack("tools", "1.0.0", "ls")).is_err(), "needs a dot");
        assert!(parse(&pack("../x.y", "1.0.0", "ls")).is_err());
        assert!(parse(&pack("Acme.Tools", "1.0.0", "ls")).is_err());
        assert!(parse(&pack("acme.tools", "1.0", "ls")).is_err());
        assert!(
            parse(&pack("acme.tools", "1.0.0", "ls\\u001b[2J")).is_err(),
            "escape codes"
        );
        assert!(parse(r#"{"format":1,"id":"a.b","name":"N","version":"1.0.0","kind":"snippets","run":"x"}"#).is_err(), "unknown fields");
        assert!(
            parse(r#"{"format":2,"id":"a.b","name":"N","version":"1.0.0","kind":"snippets"}"#)
                .is_err()
        );
    }
}
