//! Reading hosts from other tools: PuTTY (a `.reg` export of its saved
//! sessions), MobaXterm (`.mxtsessions`), CSV (Termius and spreadsheets),
//! and Ansible inventories (INI or YAML).
//!
//! Only connection settings come across. Saved passwords are never read,
//! even where the other tool keeps them.

use crate::{Auth, Host};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Where hosts were read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Putty,
    MobaXterm,
    Csv,
    Ansible,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Putty => "PuTTY sessions",
            Format::MobaXterm => "MobaXterm sessions",
            Format::Csv => "CSV",
            Format::Ansible => "Ansible inventory",
        }
    }
}

/// Hosts found in a file, each with notes on what did not come across.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    pub hosts: Vec<(Host, Vec<String>)>,
    /// Entries left out on purpose (telnet sessions, …), worded for the user.
    pub skipped: Vec<String>,
}

/// Text from a file that may be UTF-16 (regedit exports are) or UTF-8.
pub fn decode_text(bytes: &[u8]) -> String {
    let utf16 = |le: bool, body: &[u8]| {
        let units: Vec<u16> = body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| {
                if le {
                    u16::from_le_bytes(c)
                } else {
                    u16::from_be_bytes(c)
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(true, rest),
        [0xFE, 0xFF, rest @ ..] => utf16(false, rest),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Guesses the format from the file name and its first lines.
pub fn detect(file_name: &str, text: &str) -> Option<Format> {
    let name = file_name.to_ascii_lowercase();
    if text.contains("\\SimonTatham\\PuTTY\\Sessions\\") {
        return Some(Format::Putty);
    }
    if name.ends_with(".mxtsessions") || (text.contains("[Bookmarks") && text.contains("=#")) {
        return Some(Format::MobaXterm);
    }
    if name.ends_with(".csv") {
        return Some(Format::Csv);
    }
    if name.ends_with(".yml")
        || name.ends_with(".yaml")
        || name.ends_with(".ini")
        || name == "hosts"
        || name == "inventory"
        || text.contains("ansible_")
    {
        return Some(Format::Ansible);
    }
    let first = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default();
    if first.contains(',') && first.to_ascii_lowercase().contains("host") {
        return Some(Format::Csv);
    }
    None
}

/// Reads `text` as `format`.
pub fn parse(format: Format, text: &str) -> Parsed {
    match format {
        Format::Putty => parse_putty(text),
        Format::MobaXterm => parse_mobaxterm(text),
        Format::Csv => parse_csv(text),
        Format::Ansible => {
            let looks_yaml = text
                .lines()
                .map(str::trim_end)
                .find(|l| {
                    !l.trim().is_empty()
                        && !l.trim_start().starts_with('#')
                        && !l.starts_with("---")
                })
                .is_some_and(|l| !l.starts_with('[') && l.ends_with(':'));
            if looks_yaml {
                parse_ansible_yaml(text)
            } else {
                parse_ansible_ini(text)
            }
        }
    }
}

/// `user@host` as one field: split it.
fn split_user(value: &str) -> (Option<String>, String) {
    match value.rsplit_once('@') {
        Some((u, h)) if !u.is_empty() => (Some(u.to_string()), h.to_string()),
        _ => (None, value.to_string()),
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn key_note(path: &str) -> Option<String> {
    path.to_ascii_lowercase().ends_with(".ppk").then(|| {
        format!("The key {path} is a PuTTY key; Setu reads it, or convert it with `puttygen key.ppk -O private-openssh`.")
    })
}

// ---- PuTTY ----

fn parse_putty(text: &str) -> Parsed {
    let mut out = Parsed::default();
    let mut sessions: Vec<(String, BTreeMap<String, String>)> = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            if let Some((_, name)) = section.split_once("\\PuTTY\\Sessions\\") {
                sessions.push((percent_decode(name), BTreeMap::new()));
            }
            continue;
        }
        let Some((_, values)) = sessions.last_mut() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim_matches('"').to_string();
        let value = if let Some(v) = value.strip_prefix("dword:") {
            u32::from_str_radix(v.trim(), 16)
                .map(|n| n.to_string())
                .unwrap_or_default()
        } else {
            value.trim_matches('"').replace("\\\\", "\\")
        };
        values.insert(key, value);
    }
    for (name, v) in sessions {
        if name == "Default Settings" {
            continue;
        }
        let protocol = v.get("Protocol").map(String::as_str).unwrap_or("ssh");
        if protocol != "ssh" {
            out.skipped
                .push(format!("{name}: a {protocol} session, not SSH"));
            continue;
        }
        let Some(address) = v.get("HostName").filter(|h| !h.is_empty()) else {
            out.skipped.push(format!("{name}: no host name"));
            continue;
        };
        let (user_in_host, address) = split_user(address);
        let mut host = Host::new(name.clone(), address);
        let mut notes = Vec::new();
        if let Some(port) = v.get("PortNumber").and_then(|p| p.parse::<u16>().ok()) {
            host.port = port;
        }
        host.user = v
            .get("UserName")
            .filter(|u| !u.is_empty())
            .cloned()
            .or(user_in_host);
        if let Some(key) = v.get("PublicKeyFile").filter(|k| !k.is_empty()) {
            host.auth = Auth::Key;
            host.key_file = Some(key.clone());
            notes.extend(key_note(key));
        }
        if v.get("ProxyMethod").is_some_and(|m| m != "0") {
            notes
                .push("Uses a PuTTY proxy, which is not imported; set a jump host instead.".into());
        }
        if !v.get("PortForwardings").is_none_or(String::is_empty) {
            notes.push("Port forwards are not imported.".into());
        }
        out.hosts.push((host, notes));
    }
    out
}

// ---- MobaXterm ----

fn parse_mobaxterm(text: &str) -> Parsed {
    let mut out = Parsed::default();
    let mut folder: Option<String> = None;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            folder = None;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key == "SubRep" {
            folder = Some(value.replace('\\', "/")).filter(|f| !f.is_empty());
            continue;
        }
        if key == "ImgNum" || !value.starts_with('#') {
            continue;
        }
        // name=#<type>#<fields separated by %>#<display>#...
        let mut parts = value.split('#');
        parts.next();
        let kind = parts.next().unwrap_or_default();
        let fields: Vec<&str> = parts.next().unwrap_or_default().split('%').collect();
        if kind != "109" {
            out.skipped.push(format!("{key}: not an SSH session"));
            continue;
        }
        let address = fields.get(1).copied().unwrap_or_default();
        if address.is_empty() {
            out.skipped.push(format!("{key}: no host name"));
            continue;
        }
        let mut host = Host::new(key, address);
        let mut notes = Vec::new();
        if let Some(port) = fields.get(2).and_then(|p| p.parse::<u16>().ok()) {
            host.port = port;
        }
        host.user = fields
            .get(3)
            .filter(|u| !u.is_empty())
            .map(|u| u.to_string());
        host.folder = folder.clone();
        // The private key sits further along; it is the field that looks like one.
        if let Some(key_path) = fields.iter().skip(4).find(|f| looks_like_key_path(f)) {
            let path = key_path
                .replace("_ProfileDir_", "~")
                .replace("_CurrentDrive_:", "")
                .replace('\\', "/");
            notes.extend(key_note(&path));
            host.auth = Auth::Key;
            host.key_file = Some(path);
        }
        // SSH gateway (jump host): fields 7..9 are host, port, user.
        if let Some(gw) = fields
            .get(7)
            .filter(|g| !g.is_empty() && !g.starts_with('-'))
        {
            let port = fields.get(8).filter(|p| !p.is_empty() && **p != "22");
            let user = fields.get(9).filter(|u| !u.is_empty());
            host.jump = Some(format!(
                "{}{}{}",
                user.map(|u| format!("{u}@")).unwrap_or_default(),
                gw,
                port.map(|p| format!(":{p}")).unwrap_or_default()
            ));
        }
        out.hosts.push((host, notes));
    }
    out
}

fn looks_like_key_path(field: &str) -> bool {
    let lower = field.to_ascii_lowercase();
    (lower.contains('\\') || lower.contains('/'))
        && (lower.ends_with(".ppk")
            || lower.ends_with(".pem")
            || lower.ends_with(".key")
            || lower
                .rsplit(['/', '\\'])
                .next()
                .is_some_and(|f| f.starts_with("id_")))
}

// ---- CSV (Termius and spreadsheets) ----

fn csv_rows(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            (true, '"') => quoted = false,
            (true, c) => cell.push(c),
            (false, '"') => quoted = true,
            (false, ',' | ';' | '\t') => row.push(std::mem::take(&mut cell)),
            (false, '\r') => {}
            (false, '\n') => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            (false, c) => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows.retain(|r| r.iter().any(|c| !c.trim().is_empty()));
    rows
}

fn parse_csv(text: &str) -> Parsed {
    let mut out = Parsed::default();
    let rows = csv_rows(text);
    let Some((header, body)) = rows.split_first() else {
        return out;
    };
    let find = |names: &[&str]| {
        header.iter().position(|h| {
            let h = h.trim().to_ascii_lowercase().replace([' ', '_', '-'], "");
            names.contains(&h.as_str())
        })
    };
    let name_col = find(&["label", "name", "alias", "title", "hostname"]);
    let host_col = find(&["address", "host", "ip", "ipaddress", "server", "hostname"])
        .filter(|c| Some(*c) != name_col)
        .or(name_col);
    let Some(host_col) = host_col else {
        out.skipped
            .push("No column for the address (call it host, address or ip).".into());
        return out;
    };
    let port_col = find(&["port", "sshport"]);
    let user_col = find(&["username", "user", "login"]);
    let folder_col = find(&["group", "folder", "groups", "path"]);
    let tags_col = find(&["tags", "tag", "labels"]);
    let key_col = find(&["key", "keyfile", "identityfile", "privatekey", "sshkey"]);
    let jump_col = find(&["jump", "jumphost", "proxyjump", "bastion"]);
    let cell = |row: &Vec<String>, col: Option<usize>| {
        col.and_then(|c| row.get(c))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    for (i, row) in body.iter().enumerate() {
        let Some(address) = cell(row, Some(host_col)) else {
            out.skipped.push(format!("Row {}: no address", i + 2));
            continue;
        };
        let (user_in_host, address) = split_user(&address);
        let name = cell(row, name_col).unwrap_or_else(|| address.clone());
        let mut host = Host::new(name, address);
        let mut notes = Vec::new();
        if let Some(port) = cell(row, port_col) {
            match port.parse::<u16>() {
                Ok(p) => host.port = p,
                Err(_) => notes.push(format!("Port `{port}` is not a number; using 22.")),
            }
        }
        host.user = cell(row, user_col).or(user_in_host);
        host.folder = cell(row, folder_col).map(|f| f.replace(['>', '\\'], "/"));
        host.tags = cell(row, tags_col)
            .map(|t| {
                t.split([',', ';', '|'])
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        if let Some(key) = cell(row, key_col) {
            notes.extend(key_note(&key));
            host.auth = Auth::Key;
            host.key_file = Some(key);
        }
        host.jump = cell(row, jump_col);
        out.hosts.push((host, notes));
    }
    out
}

// ---- Ansible ----

/// One host as Ansible sees it: its variables and the groups it is in.
#[derive(Default)]
struct AnsibleHost {
    vars: BTreeMap<String, String>,
    groups: Vec<String>,
}

#[derive(Default)]
struct AnsibleInventory {
    hosts: BTreeMap<String, AnsibleHost>,
    order: Vec<String>,
    group_vars: BTreeMap<String, BTreeMap<String, String>>,
    children: BTreeMap<String, Vec<String>>,
}

impl AnsibleInventory {
    fn add_host(&mut self, name: &str, group: &str, vars: BTreeMap<String, String>) {
        if !self.hosts.contains_key(name) {
            self.order.push(name.to_string());
        }
        let h = self.hosts.entry(name.to_string()).or_default();
        if !group.is_empty()
            && group != "all"
            && group != "ungrouped"
            && !h.groups.iter().any(|g| g == group)
        {
            h.groups.push(group.to_string());
        }
        h.vars.extend(vars);
    }

    /// Groups that contain `group`, nearest first, ending with `all`.
    fn parents(&self, group: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut queue = vec![group.to_string()];
        let mut seen = BTreeSet::new();
        while let Some(g) = queue.pop() {
            for (parent, kids) in &self.children {
                if kids.contains(&g) && seen.insert(parent.clone()) {
                    out.push(parent.clone());
                    queue.push(parent.clone());
                }
            }
        }
        out
    }

    fn finish(self) -> Parsed {
        let mut out = Parsed::default();
        for name in &self.order {
            let h = &self.hosts[name];
            // Variables: all < parent groups < groups < host.
            let mut vars = self.group_vars.get("all").cloned().unwrap_or_default();
            let mut chain: Vec<String> = Vec::new();
            for g in &h.groups {
                let mut parents = self.parents(g);
                parents.reverse();
                chain.extend(parents);
                chain.push(g.clone());
            }
            for g in &chain {
                if let Some(gv) = self.group_vars.get(g) {
                    vars.extend(gv.clone());
                }
            }
            vars.extend(h.vars.clone());
            let get = |k: &str| vars.get(k).filter(|v| !v.is_empty()).cloned();
            if get("ansible_connection")
                .is_some_and(|c| c != "ssh" && c != "paramiko" && c != "smart")
            {
                out.skipped.push(format!(
                    "{name}: uses the {} connection, not SSH",
                    vars["ansible_connection"]
                ));
                continue;
            }
            let address = get("ansible_host")
                .or_else(|| get("ansible_ssh_host"))
                .unwrap_or_else(|| name.clone());
            let mut host = Host::new(name.clone(), address);
            let mut notes = Vec::new();
            if let Some(port) = get("ansible_port").or_else(|| get("ansible_ssh_port")) {
                match port.parse::<u16>() {
                    Ok(p) => host.port = p,
                    Err(_) => notes.push(format!("Port `{port}` is not a number; using 22.")),
                }
            }
            host.user = get("ansible_user").or_else(|| get("ansible_ssh_user"));
            if let Some(key) =
                get("ansible_ssh_private_key_file").or_else(|| get("ansible_private_key_file"))
            {
                if key.contains("{{") {
                    notes.push(format!(
                        "The key file `{key}` uses a template; set it by hand."
                    ));
                } else {
                    host.auth = Auth::Key;
                    host.key_file = Some(key);
                }
            }
            if let Some(args) =
                get("ansible_ssh_common_args").or_else(|| get("ansible_ssh_extra_args"))
            {
                host.jump = jump_from_args(&args);
                if host.jump.is_none() && args.contains("ProxyCommand") {
                    notes.push(
                        "Uses ProxyCommand, which Setu does not run; imported without it.".into(),
                    );
                }
            }
            if vars.contains_key("ansible_password") || vars.contains_key("ansible_ssh_pass") {
                notes.push("Has a password in the inventory; it is not imported. Setu asks when connecting.".into());
            }
            host.folder = h.groups.first().cloned();
            host.tags = h.groups.iter().skip(1).cloned().collect();
            out.hosts.push((host, notes));
        }
        out
    }
}

/// `-J bastion` or `-o ProxyJump=bastion` in SSH arguments.
fn jump_from_args(args: &str) -> Option<String> {
    let words: Vec<&str> = args
        .split_whitespace()
        .map(|w| w.trim_matches(['\'', '"']))
        .collect();
    for (i, w) in words.iter().enumerate() {
        if *w == "-J" {
            return words.get(i + 1).map(|s| s.to_string());
        }
        if let Some(rest) = w.strip_prefix("-J") {
            return Some(rest.to_string());
        }
        let opt = if *w == "-o" {
            words.get(i + 1).copied()
        } else {
            w.strip_prefix("-o")
        };
        if let Some(opt) = opt {
            if let Some((k, v)) = opt.split_once('=') {
                if k.eq_ignore_ascii_case("ProxyJump") {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// `web[01:03].example.com` → web01…web03; `db-[a:c]` → db-a…db-c.
fn expand_range(pattern: &str) -> Vec<String> {
    let (Some(open), Some(close)) = (pattern.find('['), pattern.find(']')) else {
        return vec![pattern.to_string()];
    };
    if close < open {
        return vec![pattern.to_string()];
    }
    let (pre, inner, post) = (
        &pattern[..open],
        &pattern[open + 1..close],
        &pattern[close + 1..],
    );
    let mut parts = inner.split(':');
    let (Some(a), Some(b)) = (parts.next(), parts.next()) else {
        return vec![pattern.to_string()];
    };
    let step = parts
        .next()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1)
        .max(1);
    let mut out = Vec::new();
    if let (Ok(x), Ok(y)) = (a.parse::<usize>(), b.parse::<usize>()) {
        let width = if a.starts_with('0') { a.len() } else { 0 };
        for n in (x..=y).step_by(step).take(1000) {
            out.extend(expand_range(&format!("{pre}{n:0width$}{post}")));
        }
    } else if let (Some(x), Some(y)) = (a.chars().next(), b.chars().next()) {
        for c in (x..=y).step_by(step).take(1000) {
            out.extend(expand_range(&format!("{pre}{c}{post}")));
        }
    }
    if out.is_empty() {
        vec![pattern.to_string()]
    } else {
        out
    }
}

/// `key=value key2="a b"` as Ansible writes host variables.
fn inline_vars(text: &str) -> BTreeMap<String, String> {
    let mut vars = BTreeMap::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        let Some(eq) = rest.find('=') else { break };
        let key = rest[..eq].trim().to_string();
        let after = &rest[eq + 1..];
        let (value, next) =
            if let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') {
                match after[1..].find(q) {
                    Some(end) => (&after[1..1 + end], &after[end + 2..]),
                    None => (&after[1..], ""),
                }
            } else {
                match after.find(char::is_whitespace) {
                    Some(end) => (&after[..end], &after[end..]),
                    None => (after, ""),
                }
            };
        vars.insert(key, value.to_string());
        rest = next.trim_start();
    }
    vars
}

fn parse_ansible_ini(text: &str) -> Parsed {
    let mut inv = AnsibleInventory::default();
    let mut section = ("ungrouped".to_string(), "hosts");
    for raw in text.lines() {
        let line = raw.split(" #").next().unwrap_or_default().trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = match name.split_once(':') {
                Some((g, "vars")) => (g.to_string(), "vars"),
                Some((g, "children")) => (g.to_string(), "children"),
                _ => (name.to_string(), "hosts"),
            };
            continue;
        }
        match section.1 {
            "vars" => {
                if let Some((k, v)) = line.split_once('=') {
                    inv.group_vars.entry(section.0.clone()).or_default().insert(
                        k.trim().to_string(),
                        v.trim().trim_matches(['"', '\'']).to_string(),
                    );
                }
            }
            "children" => inv
                .children
                .entry(section.0.clone())
                .or_default()
                .push(line.to_string()),
            _ => {
                let (name, vars) = match line.split_once(char::is_whitespace) {
                    Some((n, v)) => (n, inline_vars(v)),
                    None => (line, BTreeMap::new()),
                };
                for host in expand_range(name) {
                    inv.add_host(&host, &section.0, vars.clone());
                }
            }
        }
    }
    inv.finish()
}

/// The small part of YAML that inventories use: nested mappings of
/// `key: value`, with comments and quoted values.
#[derive(Debug, Clone, PartialEq)]
enum Yaml {
    Scalar(String),
    Map(Vec<(String, Yaml)>),
}

fn yaml_scalar(v: &str) -> String {
    let v = v.split(" #").next().unwrap_or_default().trim();
    if v == "~" || v == "null" {
        return String::new();
    }
    v.trim_matches(['"', '\'']).to_string()
}

fn parse_yaml(text: &str) -> Yaml {
    let lines: Vec<(usize, &str)> = text
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with('#') && t != "---" && t != "..."
        })
        .map(|l| (l.len() - l.trim_start().len(), l.trim()))
        .collect();
    fn block(lines: &[(usize, &str)], i: &mut usize, indent: usize) -> Yaml {
        let mut map = Vec::new();
        while *i < lines.len() {
            let (ind, text) = lines[*i];
            if ind < indent {
                break;
            }
            *i += 1;
            // `key: value`, or `key:` opening a block. Keys may hold colons
            // themselves (`web[01:03]:`), so split at ": " or a final ":".
            let Some((key, value)) = text
                .split_once(": ")
                .or_else(|| text.strip_suffix(':').map(|k| (k, "")))
            else {
                continue;
            };
            let key = key.trim().trim_matches(['"', '\'']).to_string();
            let value = value.trim();
            if value.is_empty() || value.starts_with('#') {
                let child = if *i < lines.len() && lines[*i].0 > ind {
                    let next = lines[*i].0;
                    block(lines, i, next)
                } else {
                    Yaml::Scalar(String::new())
                };
                map.push((key, child));
            } else {
                map.push((key, Yaml::Scalar(yaml_scalar(value))));
            }
        }
        Yaml::Map(map)
    }
    let mut i = 0;
    let first = lines.first().map_or(0, |l| l.0);
    block(&lines, &mut i, first)
}

fn parse_ansible_yaml(text: &str) -> Parsed {
    fn walk(inv: &mut AnsibleInventory, group: &str, node: &Yaml) {
        let Yaml::Map(entries) = node else { return };
        for (key, value) in entries {
            match (key.as_str(), value) {
                ("hosts", Yaml::Map(hosts)) => {
                    for (name, vars) in hosts {
                        let vars = match vars {
                            Yaml::Map(v) => v
                                .iter()
                                .filter_map(|(k, v)| match v {
                                    Yaml::Scalar(s) => Some((k.clone(), s.clone())),
                                    Yaml::Map(_) => None,
                                })
                                .collect(),
                            Yaml::Scalar(_) => BTreeMap::new(),
                        };
                        for host in expand_range(name) {
                            inv.add_host(&host, group, vars.clone());
                        }
                    }
                }
                ("vars", Yaml::Map(vars)) => {
                    let gv = inv.group_vars.entry(group.to_string()).or_default();
                    for (k, v) in vars {
                        if let Yaml::Scalar(s) = v {
                            gv.insert(k.clone(), s.clone());
                        }
                    }
                }
                ("children", Yaml::Map(kids)) => {
                    for (child, body) in kids {
                        inv.children
                            .entry(group.to_string())
                            .or_default()
                            .push(child.clone());
                        walk(inv, child, body);
                    }
                }
                _ => {}
            }
        }
    }
    let mut inv = AnsibleInventory::default();
    if let Yaml::Map(top) = parse_yaml(text) {
        for (group, body) in &top {
            walk(&mut inv, group, body);
        }
    }
    inv.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(p: &Parsed) -> Vec<&str> {
        p.hosts.iter().map(|(h, _)| h.name.as_str()).collect()
    }

    #[test]
    fn putty_reg_export() {
        let text = "Windows Registry Editor Version 5.00\r\n\r\n\
[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\Default%20Settings]\r\n\"HostName\"=\"\"\r\n\r\n\
[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\web%2001]\r\n\
\"HostName\"=\"deploy@10.0.0.5\"\r\n\"PortNumber\"=dword:00000948\r\n\"Protocol\"=\"ssh\"\r\n\
\"PublicKeyFile\"=\"C:\\\\Users\\\\me\\\\web.ppk\"\r\n\r\n\
[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\switch]\r\n\"HostName\"=\"10.0.0.1\"\r\n\"Protocol\"=\"telnet\"\r\n";
        // regedit writes UTF-16LE with a byte order mark.
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        let decoded = decode_text(&bytes);
        assert_eq!(detect("putty.reg", &decoded), Some(Format::Putty));
        let p = parse(Format::Putty, &decoded);
        assert_eq!(names(&p), ["web 01"]);
        let (h, notes) = &p.hosts[0];
        assert_eq!(
            (h.host.as_str(), h.port, h.user.as_deref()),
            ("10.0.0.5", 2376, Some("deploy"))
        );
        assert_eq!(h.key_file.as_deref(), Some("C:\\Users\\me\\web.ppk"));
        assert!(notes[0].contains("PuTTY key"));
        assert_eq!(p.skipped, ["switch: a telnet session, not SSH"]);
    }

    #[test]
    fn mobaxterm_sessions() {
        let text = "[Bookmarks]\nSubRep=\nImgNum=42\n[Bookmarks_1]\nSubRep=Production\\Web\nImgNum=41\n\
web-01=#109#0%10.0.0.5%2222%ops%%-1%-1%bastion.example.com%22%jump%0%0%0%_ProfileDir_\\.ssh\\id_ed25519%%-1%0%0%0%%1080%%0%0%1#MobaFont%10%0%0%-1%15#0# #-1\n\
rdp-box=#91#4%10.0.0.9%3389%admin%0%-1#MobaFont#0# #-1\n";
        assert_eq!(detect("x.mxtsessions", text), Some(Format::MobaXterm));
        let p = parse(Format::MobaXterm, text);
        assert_eq!(names(&p), ["web-01"]);
        let h = &p.hosts[0].0;
        assert_eq!(
            (h.host.as_str(), h.port, h.user.as_deref()),
            ("10.0.0.5", 2222, Some("ops"))
        );
        assert_eq!(h.folder.as_deref(), Some("Production/Web"));
        assert_eq!(h.key_file.as_deref(), Some("~/.ssh/id_ed25519"));
        assert_eq!(h.jump.as_deref(), Some("jump@bastion.example.com"));
        assert_eq!(p.skipped.len(), 1);
    }

    #[test]
    fn csv_with_any_header_names() {
        let text = "Label,Address,Port,Username,Group,Tags\n\
web-01,10.0.0.5,22,ops,Production,\"web, nginx\"\n\
\"db, primary\",db@10.0.0.6,,,Production > DB,\n,,,\n";
        assert_eq!(detect("termius.csv", text), Some(Format::Csv));
        let p = parse(Format::Csv, text);
        assert_eq!(names(&p), ["web-01", "db, primary"]);
        assert_eq!(p.hosts[0].0.tags, ["web", "nginx"]);
        let db = &p.hosts[1].0;
        assert_eq!(
            (db.host.as_str(), db.user.as_deref()),
            ("10.0.0.6", Some("db"))
        );
        assert_eq!(db.folder.as_deref(), Some("Production / DB"));
    }

    #[test]
    fn ansible_ini_with_groups_vars_and_ranges() {
        let text = "\
bastion ansible_host=203.0.113.10 ansible_user=jump

[web]
web[01:03].example.com

[db]
db-01 ansible_host=10.0.0.21 ansible_port=2200 # primary

[web:vars]
ansible_user=deploy
ansible_ssh_common_args='-o ProxyJump=jump@203.0.113.10'

[prod:children]
web
db

[prod:vars]
ansible_user=ops
ansible_ssh_private_key_file=~/.ssh/prod
";
        let p = parse(detect("hosts", text).expect("format"), text);
        assert_eq!(
            names(&p),
            [
                "bastion",
                "web01.example.com",
                "web02.example.com",
                "web03.example.com",
                "db-01"
            ]
        );
        let web = &p.hosts[1].0;
        assert_eq!(
            web.user.as_deref(),
            Some("deploy"),
            "group vars beat parent group vars"
        );
        assert_eq!(web.jump.as_deref(), Some("jump@203.0.113.10"));
        assert_eq!(web.key_file.as_deref(), Some("~/.ssh/prod"));
        assert_eq!(web.folder.as_deref(), Some("web"));
        let db = &p.hosts[4].0;
        assert_eq!(
            (db.host.as_str(), db.port, db.user.as_deref()),
            ("10.0.0.21", 2200, Some("ops"))
        );
        assert!(p.hosts[0].0.folder.is_none());
    }

    #[test]
    fn ansible_yaml() {
        let text = "\
---
all:
  vars:
    ansible_user: admin
  hosts:
    bastion:
      ansible_host: 203.0.113.10
  children:
    web:
      hosts:
        web-01:
          ansible_host: 10.0.0.11   # blue
        web-02:
        web-[05:06]:
      vars:
        ansible_ssh_common_args: \"-J admin@203.0.113.10\"
    windows:
      hosts:
        win-01:
          ansible_connection: winrm
";
        let p = parse(detect("inventory.yml", text).expect("format"), text);
        assert_eq!(
            names(&p),
            ["bastion", "web-01", "web-02", "web-05", "web-06"]
        );
        let w1 = &p.hosts[1].0;
        assert_eq!(
            (w1.host.as_str(), w1.user.as_deref()),
            ("10.0.0.11", Some("admin"))
        );
        assert_eq!(w1.jump.as_deref(), Some("admin@203.0.113.10"));
        assert_eq!(p.hosts[2].0.host, "web-02");
        assert_eq!(p.skipped, ["win-01: uses the winrm connection, not SSH"]);
    }

    #[test]
    fn ranges() {
        assert_eq!(expand_range("a[1:3]"), ["a1", "a2", "a3"]);
        assert_eq!(expand_range("n[08:10]x"), ["n08x", "n09x", "n10x"]);
        assert_eq!(expand_range("db-[a:c]"), ["db-a", "db-b", "db-c"]);
        assert_eq!(expand_range("plain"), ["plain"]);
    }
}
