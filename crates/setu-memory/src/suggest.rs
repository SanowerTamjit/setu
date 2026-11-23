//! Finds work that repeats and offers to automate it.
//!
//! - **Snippet**: one longer command typed often (4+ times, in 2+ separate
//!   sittings), or a short run of commands always typed together in a
//!   local shell (saved as one line joined by `&&`).
//! - **Workflow**: the same 2 to 5 commands in the same order, on saved
//!   hosts, in 3+ separate sittings. Becomes a workflow with one step per
//!   command, each after the previous one succeeded.
//! - **Fleet**: one command run on 3+ saved hosts. Becomes a workflow with
//!   one step on all of them, or on a tag they share.
//!
//! A sitting is the commands of one terminal pane with no break longer
//! than half an hour. Commands where a secret was masked are left out,
//! and so are everyday commands like `ls` and `cd` on their own.

use crate::store::{Entry, MemoryFile};
use serde::Serialize;
use setu_workflow::{slugify, Link, Step, When, Workflow, FORMAT_VERSION};
use std::collections::{BTreeMap, BTreeSet, HashMap};

const GAP: u64 = 30 * 60;
const MIN_SNIPPET: usize = 4;
const MIN_SEQUENCE: usize = 3;
const MIN_FLEET: usize = 3;
const MAX_SEQUENCE: usize = 5;
const MAX_SUGGESTIONS: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SuggestionKind {
    Snippet,
    Workflow,
    Fleet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Suggestion {
    /// Stable across runs, so a dismissal sticks.
    pub id: String,
    pub kind: SuggestionKind,
    pub title: String,
    /// Why Setu suggests it, in a sentence.
    pub reason: String,
    pub commands: Vec<String>,
    /// Saved hosts it ran on (none for local shells).
    pub hosts: Vec<String>,
    /// For fleet suggestions: a tag every one of those hosts has.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// How many times (sittings for sequences, hosts for fleets).
    pub count: usize,
    /// When it last happened, seconds since 1970.
    pub last: u64,
}

const EVERYDAY: &[&str] = &[
    "ls", "ll", "la", "l", "cd", "pwd", "clear", "cls", "exit", "logout", "history", "whoami",
    "date", "top", "htop", "q", "bg", "fg", "jobs", "reset", "echo", "man", "help", "true",
    "false",
];

/// `ls`, `cd /x`, `clear`: too small to be worth automating on their own.
pub fn is_everyday(command: &str) -> bool {
    let first = command.split_whitespace().next().unwrap_or("");
    command.len() < 4 || EVERYDAY.contains(&first)
}

/// FNV-1a: a short, stable id without another dependency.
fn stable_id(kind: SuggestionKind, commands: &[String]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let tag = match kind {
        SuggestionKind::Snippet => "snippet",
        SuggestionKind::Workflow => "workflow",
        SuggestionKind::Fleet => "fleet",
    };
    for b in tag.bytes().chain(commands.join("\n").bytes()) {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{}-{h:016x}", &tag[..1])
}

fn short(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}

fn list(names: &[String]) -> String {
    match names.len() {
        0 => String::new(),
        1 => names[0].clone(),
        2 => format!("{} and {}", names[0], names[1]),
        3 => format!("{}, {} and {}", names[0], names[1], names[2]),
        n => format!("{}, {} and {} more", names[0], names[1], n - 2),
    }
}

/// The sittings: each pane's commands, split at long breaks, without
/// masked entries and without the same command twice in a row.
fn sittings(entries: &[Entry]) -> Vec<Vec<&Entry>> {
    let mut by_session: BTreeMap<&str, Vec<&Entry>> = BTreeMap::new();
    for e in entries.iter().filter(|e| !e.masked) {
        by_session.entry(e.session.as_str()).or_default().push(e);
    }
    let mut out = Vec::new();
    for (_, mut list) in by_session {
        list.sort_by_key(|e| e.id);
        let mut current: Vec<&Entry> = Vec::new();
        for e in list {
            if let Some(prev) = current.last() {
                if e.at.saturating_sub(prev.at) > GAP || prev.host != e.host {
                    out.push(std::mem::take(&mut current));
                } else if prev.command == e.command {
                    continue;
                }
            }
            current.push(e);
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    out
}

#[derive(Default)]
struct Seen {
    sittings: BTreeSet<usize>,
    hosts: BTreeSet<Option<String>>,
    times: usize,
    last: u64,
}

/// Everything worth suggesting now, most frequent first. `hosts` are the
/// saved hosts and their tags; hosts that are gone are left out.
pub fn suggestions(memory: &MemoryFile, hosts: &[(String, Vec<String>)]) -> Vec<Suggestion> {
    let saved: HashMap<&str, &Vec<String>> = hosts.iter().map(|(n, t)| (n.as_str(), t)).collect();
    let is_saved = |h: &Option<String>| h.as_deref().is_some_and(|h| saved.contains_key(h));
    let dismissed: BTreeSet<&str> = memory.dismissed.iter().map(String::as_str).collect();
    let snippets: BTreeSet<&str> = memory.snippets.iter().map(|s| s.command.as_str()).collect();
    let groups = sittings(&memory.entries);
    let mut out: Vec<Suggestion> = Vec::new();

    // Single commands: how often, in how many sittings, on which hosts.
    let mut single: BTreeMap<&str, Seen> = BTreeMap::new();
    for (i, g) in groups.iter().enumerate() {
        for e in g {
            let s = single.entry(e.command.as_str()).or_default();
            s.sittings.insert(i);
            s.hosts.insert(e.host.clone());
            s.times += 1;
            s.last = s.last.max(e.at);
        }
    }

    // Sequences of 2..=5 commands in a row in one sitting.
    let mut seqs: BTreeMap<Vec<&str>, Seen> = BTreeMap::new();
    for (i, g) in groups.iter().enumerate() {
        for n in 2..=MAX_SEQUENCE.min(g.len()) {
            for w in g.windows(n) {
                let cmds: Vec<&str> = w.iter().map(|e| e.command.as_str()).collect();
                if cmds.iter().filter(|c| !is_everyday(c)).count() < 2 {
                    continue;
                }
                let s = seqs.entry(cmds).or_default();
                if s.sittings.insert(i) {
                    s.times += 1;
                }
                s.hosts.insert(w[0].host.clone());
                s.last = s.last.max(w[n - 1].at);
            }
        }
    }
    seqs.retain(|_, s| s.sittings.len() >= MIN_SEQUENCE);
    // Keep the longest: drop a sequence when a longer one holding it
    // happened at least as often.
    let keys: Vec<Vec<&str>> = seqs.keys().cloned().collect();
    let dominated: BTreeSet<Vec<&str>> = keys
        .iter()
        .filter(|a| {
            keys.iter().any(|b| {
                b.len() > a.len()
                    && b.windows(a.len()).any(|w| w == a.as_slice())
                    && seqs[b].sittings.len() >= seqs[*a].sittings.len()
            })
        })
        .cloned()
        .collect();
    let mut in_sequence: BTreeSet<&str> = BTreeSet::new();
    for (cmds, s) in &seqs {
        if dominated.contains(cmds) {
            continue;
        }
        let commands: Vec<String> = cmds.iter().map(|c| c.to_string()).collect();
        let count = s.sittings.len();
        if s.hosts.iter().all(is_saved) {
            let names: Vec<String> = s.hosts.iter().flatten().cloned().collect();
            out.push(Suggestion {
                id: stable_id(SuggestionKind::Workflow, &commands),
                kind: SuggestionKind::Workflow,
                title: format!("Routine: {}", short(&commands.join(" → "), 70)),
                reason: format!(
                    "You ran these {} commands in this order {count} times, on {}.",
                    commands.len(),
                    list(&names)
                ),
                commands,
                hosts: names,
                tag: None,
                count,
                last: s.last,
            });
            in_sequence.extend(cmds.iter().copied());
        } else if s.hosts.iter().all(Option::is_none) {
            let line = commands.join(" && ");
            if line.len() > crate::store::MAX_COMMAND || snippets.contains(line.as_str()) {
                continue;
            }
            out.push(Suggestion {
                id: stable_id(SuggestionKind::Snippet, std::slice::from_ref(&line)),
                kind: SuggestionKind::Snippet,
                title: short(&line, 70),
                reason: format!(
                    "You typed these {} commands one after another {count} times. Save them as one snippet.",
                    commands.len()
                ),
                commands: vec![line],
                hosts: Vec::new(),
                tag: None,
                count,
                last: s.last,
            });
            in_sequence.extend(cmds.iter().copied());
        }
    }

    for (cmd, s) in &single {
        if is_everyday(cmd) {
            continue;
        }
        let on_hosts: Vec<String> = s
            .hosts
            .iter()
            .filter(|h| is_saved(h))
            .flatten()
            .cloned()
            .collect();
        // A command that is part of a suggested routine comes with it.
        if on_hosts.len() >= MIN_FLEET && !in_sequence.contains(cmd) {
            let tag = shared_tag(&on_hosts, hosts);
            let commands = vec![cmd.to_string()];
            out.push(Suggestion {
                id: stable_id(SuggestionKind::Fleet, &commands),
                kind: SuggestionKind::Fleet,
                title: format!("Run {} on {} hosts at once", short(cmd, 50), on_hosts.len()),
                reason: format!(
                    "You ran this on {} one by one. A workflow can run it on {} together.",
                    list(&on_hosts),
                    tag.as_ref().map_or("all of them".to_string(), |t| format!(
                        "every host tagged {t}"
                    ))
                ),
                commands,
                count: on_hosts.len(),
                hosts: on_hosts,
                tag,
                last: s.last,
            });
        }
        if s.times >= MIN_SNIPPET
            && s.sittings.len() >= 2
            && cmd.len() >= 10
            && !snippets.contains(cmd)
            && !in_sequence.contains(cmd)
        {
            let commands = vec![cmd.to_string()];
            out.push(Suggestion {
                id: stable_id(SuggestionKind::Snippet, &commands),
                kind: SuggestionKind::Snippet,
                title: short(cmd, 70),
                reason: format!(
                    "You typed this {} times in {} sittings. A snippet types it for you from the palette.",
                    s.times,
                    s.sittings.len()
                ),
                commands,
                hosts: s.hosts.iter().flatten().cloned().collect(),
                tag: None,
                count: s.times,
                last: s.last,
            });
        }
    }

    out.retain(|s| !dismissed.contains(s.id.as_str()));
    out.sort_by(|a, b| {
        let rank = |k: SuggestionKind| match k {
            SuggestionKind::Workflow => 0,
            SuggestionKind::Fleet => 1,
            SuggestionKind::Snippet => 2,
        };
        rank(a.kind)
            .cmp(&rank(b.kind))
            .then(b.count.cmp(&a.count))
            .then(b.last.cmp(&a.last))
    });
    out.truncate(MAX_SUGGESTIONS);
    out
}

/// A tag every host in `names` has, preferring the one with fewest hosts.
fn shared_tag(names: &[String], hosts: &[(String, Vec<String>)]) -> Option<String> {
    let tags_of = |n: &str| {
        hosts
            .iter()
            .find(|(h, _)| h == n)
            .map(|(_, t)| t.clone())
            .unwrap_or_default()
    };
    let first = tags_of(names.first()?);
    first
        .into_iter()
        .filter(|t| {
            names
                .iter()
                .all(|n| tags_of(n).iter().any(|x| x.eq_ignore_ascii_case(t)))
        })
        .min_by_key(|t| {
            hosts
                .iter()
                .filter(|(_, ts)| ts.iter().any(|x| x.eq_ignore_ascii_case(t)))
                .count()
        })
}

/// Turns a typed sequence into workflow steps. Each step runs in a fresh
/// SSH session, so a `cd` or `export` typed on its own would be lost: it
/// is carried into every later step instead (`cd /srv/app && git pull`).
/// Returns each step's display name (the command as typed) and command.
fn as_steps(commands: &[String]) -> Vec<(String, String)> {
    let mut prefix = String::new();
    let mut out = Vec::new();
    for c in commands {
        let first = c.split_whitespace().next().unwrap_or("");
        if first == "cd" || first == "export" || first == "pushd" {
            prefix.push_str(c);
            prefix.push_str(" && ");
        } else {
            out.push((c.clone(), format!("{prefix}{c}")));
        }
    }
    out
}

/// A workflow draft for a workflow or fleet suggestion: nothing is saved
/// or run; it opens in the builder for the person to check and save.
/// `taken` are the ids of saved workflows.
pub fn to_workflow(
    s: &Suggestion,
    hosts: &[(String, Vec<String>)],
    taken: &[String],
) -> Result<Workflow, String> {
    let saved: Vec<String> = s
        .hosts
        .iter()
        .filter(|h| hosts.iter().any(|(n, _)| n == *h))
        .cloned()
        .collect();
    let (name, steps) = match s.kind {
        SuggestionKind::Snippet => {
            return Err("this suggestion is a snippet, not a workflow".into())
        }
        SuggestionKind::Workflow => {
            if saved.is_empty() {
                return Err("the hosts these commands ran on are no longer saved".into());
            }
            let targets: Vec<&str> = saved.iter().map(String::as_str).collect();
            let steps: Vec<Step> = as_steps(&s.commands)
                .iter()
                .enumerate()
                .map(|(i, (name, c))| {
                    Step::command(&format!("step-{}", i + 1), &short(name, 40), c, &targets)
                        .at(40.0 + 280.0 * i as f64, 80.0)
                })
                .collect();
            let first = steps
                .first()
                .map(|st| st.name.clone())
                .ok_or("nothing to run in this routine")?;
            (format!("Routine: {first}"), steps)
        }
        SuggestionKind::Fleet => {
            let cmd = s.commands.first().ok_or("no command")?;
            let targets: Vec<String> = match &s.tag {
                Some(t) => vec![format!("tag:{t}")],
                None if !saved.is_empty() => saved.clone(),
                None => return Err("the hosts this ran on are no longer saved".into()),
            };
            let targets: Vec<&str> = targets.iter().map(String::as_str).collect();
            let step = Step::command("run", &short(cmd, 40), cmd, &targets).at(40.0, 80.0);
            (format!("On all: {}", short(cmd, 40)), vec![step])
        }
    };
    let links = steps
        .windows(2)
        .map(|w| Link::new(&w[0].id, &w[1].id, When::Success))
        .collect();
    let base = slugify(&name);
    let mut id = base.clone();
    let mut n = 2;
    while taken.iter().any(|t| t == &id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    let workflow = Workflow {
        version: FORMAT_VERSION,
        id,
        name,
        description: format!("Made from work memory. {}", s.reason),
        steps,
        links,
    };
    workflow.validate().map_err(|e| e.to_string())?;
    Ok(workflow)
}

/// A Markdown summary of the person's routine work, for them to share with
/// a teammate or paste into an AI assistant as context. Masked commands
/// are left out; nothing leaves the device unless the person copies it.
pub fn summary(memory: &MemoryFile, hosts: &[(String, Vec<String>)]) -> String {
    let mut counts: BTreeMap<&str, (usize, BTreeSet<&str>)> = BTreeMap::new();
    for e in memory
        .entries
        .iter()
        .filter(|e| !e.masked && !is_everyday(&e.command))
    {
        let c = counts.entry(e.command.as_str()).or_default();
        c.0 += 1;
        if let Some(h) = &e.host {
            c.1.insert(h.as_str());
        }
    }
    let mut top: Vec<_> = counts.into_iter().collect();
    top.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(a.0.cmp(b.0)));
    let mut md = String::from("# My routine ops work\n\nFrom Setu's work memory (on my device; secrets masked).\n\n## Commands I run most\n\n");
    if top.is_empty() {
        md.push_str("Nothing yet.\n");
    }
    for (cmd, (n, on)) in top.iter().take(15) {
        let on: Vec<String> = on.iter().map(|s| s.to_string()).collect();
        let where_ = if on.is_empty() {
            String::new()
        } else {
            format!(", on {}", list(&on))
        };
        md.push_str(&format!("- `{}` ({n}×{where_})\n", cmd.replace('`', "'")));
    }
    let found = suggestions(memory, hosts);
    let routines: Vec<&Suggestion> = found
        .iter()
        .filter(|s| s.kind != SuggestionKind::Snippet)
        .collect();
    if !routines.is_empty() {
        md.push_str("\n## Routines I repeat\n\n");
        for s in routines {
            md.push_str(&format!("- {}\n", s.reason));
            for c in &s.commands {
                md.push_str(&format!("  - `{}`\n", c.replace('`', "'")));
            }
        }
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hosts() -> Vec<(String, Vec<String>)> {
        vec![
            ("web-1".into(), vec!["web".into(), "prod".into()]),
            ("web-2".into(), vec!["web".into(), "prod".into()]),
            ("web-3".into(), vec!["web".into(), "prod".into()]),
            ("db-1".into(), vec!["db".into(), "prod".into()]),
        ]
    }

    fn memory() -> MemoryFile {
        let mut m = MemoryFile::default();
        m.settings.recording = true;
        m
    }

    /// Runs `cmds` in a new sitting (its own pane), starting at `t`.
    fn sitting(m: &mut MemoryFile, pane: &str, host: Option<&str>, t: u64, cmds: &[&str]) {
        for (i, c) in cmds.iter().enumerate() {
            m.record(c, host, pane, t + 10 * i as u64).expect("record");
        }
    }

    #[test]
    fn repeated_sequence_on_hosts_becomes_a_workflow() {
        let mut m = memory();
        let routine = [
            "cd /srv/app",
            "git pull",
            "systemctl restart app",
            "systemctl status app",
        ];
        sitting(&mut m, "a", Some("web-1"), 1000, &routine);
        sitting(&mut m, "b", Some("web-2"), 5000, &routine);
        // Same pane after a long break counts as another sitting.
        sitting(
            &mut m,
            "a",
            Some("web-1"),
            9000,
            &[
                "ls",
                "git pull",
                "systemctl restart app",
                "systemctl status app",
            ],
        );
        let found = suggestions(&m, &hosts());
        let wf: Vec<_> = found
            .iter()
            .filter(|s| s.kind == SuggestionKind::Workflow)
            .collect();
        assert_eq!(wf.len(), 1, "{found:#?}");
        assert_eq!(
            wf[0].commands,
            vec!["git pull", "systemctl restart app", "systemctl status app"]
        );
        assert_eq!(wf[0].hosts, vec!["web-1", "web-2"]);
        assert_eq!(wf[0].count, 3);

        let draft = to_workflow(wf[0], &hosts(), &["routine-git-pull".into()]).expect("draft");
        assert_eq!(draft.steps[0].command, "git pull");
        assert_eq!(draft.id, "routine-git-pull-2");
        assert_eq!(draft.steps.len(), 3);
        assert_eq!(draft.links.len(), 2);
        assert_eq!(draft.steps[0].targets, vec!["web-1", "web-2"]);
    }

    #[test]
    fn cd_and_export_carry_into_later_steps() {
        let cmds: Vec<String> = ["cd /srv/app", "git pull", "export ENV=prod", "./deploy.sh"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let steps = as_steps(&cmds);
        assert_eq!(
            steps,
            vec![
                (
                    "git pull".to_string(),
                    "cd /srv/app && git pull".to_string()
                ),
                (
                    "./deploy.sh".to_string(),
                    "cd /srv/app && export ENV=prod && ./deploy.sh".to_string()
                ),
            ]
        );
    }

    #[test]
    fn same_command_on_many_hosts_becomes_a_fleet_workflow() {
        let mut m = memory();
        for (i, h) in ["web-1", "web-2", "web-3"].iter().enumerate() {
            sitting(
                &mut m,
                &format!("p{i}"),
                Some(h),
                100 * i as u64,
                &["df -h /var"],
            );
        }
        let found = suggestions(&m, &hosts());
        let fleet = found
            .iter()
            .find(|s| s.kind == SuggestionKind::Fleet)
            .expect("fleet");
        // web is the smallest tag all three share.
        assert_eq!(fleet.tag.as_deref(), Some("web"));
        let draft = to_workflow(fleet, &hosts(), &[]).expect("draft");
        assert_eq!(draft.steps[0].targets, vec!["tag:web"]);
    }

    #[test]
    fn frequent_local_command_becomes_a_snippet_and_dismissal_sticks() {
        let mut m = memory();
        for i in 0..4u64 {
            sitting(
                &mut m,
                &format!("p{i}"),
                None,
                i * 4000,
                &["journalctl -u nginx --since today"],
            );
        }
        let found = suggestions(&m, &hosts());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, SuggestionKind::Snippet);
        assert!(to_workflow(&found[0], &hosts(), &[]).is_err());
        let id = found[0].id.clone();
        m.dismiss(&id).expect("dismiss");
        assert!(suggestions(&m, &hosts()).is_empty());
        // Saved as a snippet: not suggested again either.
        m.dismissed.clear();
        m.add_snippet("Nginx today", "journalctl -u nginx --since today", 1)
            .expect("snippet");
        assert!(suggestions(&m, &hosts()).is_empty());
    }

    #[test]
    fn masked_and_everyday_commands_are_never_suggested() {
        let mut m = memory();
        for i in 0..5u64 {
            sitting(
                &mut m,
                &format!("p{i}"),
                None,
                i * 4000,
                &["ls -la", "MYSQL_PWD_TOKEN=x mysql shop", "cd /tmp"],
            );
        }
        assert!(suggestions(&m, &hosts()).is_empty());
        assert!(!summary(&m, &hosts()).contains("MYSQL"));
    }

    #[test]
    fn hosts_that_are_gone_are_not_suggested() {
        let mut m = memory();
        for (i, h) in ["old-1", "old-2", "old-3"].iter().enumerate() {
            sitting(&mut m, &format!("p{i}"), Some(h), 0, &["df -h /var"]);
        }
        assert!(suggestions(&m, &hosts())
            .iter()
            .all(|s| s.kind != SuggestionKind::Fleet));
    }

    #[test]
    fn summary_lists_top_commands() {
        let mut m = memory();
        sitting(&mut m, "a", Some("web-1"), 0, &["uptime -p", "free -m"]);
        let md = summary(&m, &hosts());
        assert!(md.contains("`uptime -p` (1×, on web-1)"), "{md}");
    }
}
