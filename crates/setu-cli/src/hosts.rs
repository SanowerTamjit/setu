//! `setu hosts`: list saved hosts and import them from `~/.ssh/config`.

use crate::doctor::Context;
use crate::output::{pad, use_color, Paint};
use setu_hosts::import::{self, Status};
use setu_hosts::ssh_config::SshConfig;
use setu_hosts::{Host, HostStore};
use std::process::ExitCode;

fn store(ctx: &Context) -> Result<HostStore, String> {
    Ok(HostStore::in_dir(&ctx.config_dir.clone()?))
}

/// `user@host:port`, leaving out what OpenSSH would fill in.
fn target(h: &Host) -> String {
    let mut s = String::new();
    if let Some(u) = h.user.as_deref().filter(|u| !u.is_empty()) {
        s.push_str(u);
        s.push('@');
    }
    s.push_str(&h.host);
    if h.port != 22 {
        s.push_str(&format!(":{}", h.port));
    }
    s
}

fn matches(h: &Host, search: &str) -> bool {
    let q = search.to_lowercase();
    [
        Some(h.name.as_str()),
        Some(h.host.as_str()),
        h.user.as_deref(),
        h.folder.as_deref(),
    ]
    .into_iter()
    .flatten()
    .chain(h.tags.iter().map(String::as_str))
    .any(|f| f.to_lowercase().contains(&q))
}

/// Text table of `hosts`, sorted by folder then name.
pub fn table(hosts: &[&Host], color: bool) -> String {
    let p = Paint::new(color);
    let mut rows: Vec<&&Host> = hosts.iter().collect();
    rows.sort_by(|a, b| {
        (a.folder.as_deref().unwrap_or(""), a.name.to_lowercase())
            .cmp(&(b.folder.as_deref().unwrap_or(""), b.name.to_lowercase()))
    });
    let name_w = rows
        .iter()
        .map(|h| h.name.chars().count())
        .max()
        .unwrap_or(0)
        .max(4);
    let targets: Vec<String> = rows.iter().map(|h| target(h)).collect();
    let target_w = targets
        .iter()
        .map(|t| t.chars().count())
        .max()
        .unwrap_or(0)
        .max(6);
    let mut out = format!(
        "{}\n",
        p.dim(&format!(
            "{}  {}  {}",
            pad("NAME", name_w),
            pad("TARGET", target_w),
            "FOLDER / TAGS"
        ))
    );
    for (h, t) in rows.iter().zip(&targets) {
        let mut extra = h.folder.clone().unwrap_or_default();
        if !h.tags.is_empty() {
            if !extra.is_empty() {
                extra.push_str("  ");
            }
            extra.push_str(&p.cyan(&h.tags.join(" ")));
        }
        if let Some(jump) = &h.jump {
            extra.push_str(&p.dim(&format!("  via {jump}")));
        }
        out.push_str(&format!(
            "{}  {}  {extra}\n",
            p.bold(&pad(&h.name, name_w)),
            pad(t, target_w)
        ));
    }
    out
}

pub fn list(ctx: &Context, search: Option<&str>, json: bool) -> Result<ExitCode, String> {
    let store = store(ctx)?;
    let inventory = store.load().map_err(|e| e.to_string())?;
    let hosts: Vec<&Host> = inventory
        .hosts
        .iter()
        .filter(|h| search.is_none_or(|s| matches(h, s)))
        .collect();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&hosts).map_err(|e| e.to_string())?
        );
        return Ok(ExitCode::SUCCESS);
    }
    if hosts.is_empty() {
        if inventory.hosts.is_empty() {
            println!(
                "No saved hosts in {}. Add them in the app, or run: setu hosts import",
                ctx.show(store.path())
            );
        } else {
            println!("No saved host matches \"{}\".", search.unwrap_or_default());
        }
        // Nothing found is a normal answer for a search, not an error.
        return Ok(ExitCode::SUCCESS);
    }
    print!("{}", table(&hosts, use_color()));
    Ok(ExitCode::SUCCESS)
}

pub fn import(ctx: &Context, write: bool, folder: &str) -> Result<ExitCode, String> {
    let p = Paint::new(use_color());
    let path = ctx.home.join(".ssh").join("config");
    let config = match SshConfig::load(&path, &ctx.home) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("No {} to import from.", ctx.show(&path));
            return Ok(ExitCode::SUCCESS);
        }
        Err(e) => return Err(format!("could not read {}: {e}", ctx.show(&path))),
    };
    let store = store(ctx)?;
    let mut inventory = store.load().map_err(|e| e.to_string())?;
    let plan = import::plan(&config, &inventory);
    for w in &config.warnings {
        println!("{} {w}", p.yellow("!"));
    }
    if plan.is_empty() {
        println!("No hosts found in {}.", ctx.show(&path));
        return Ok(ExitCode::SUCCESS);
    }
    let name_w = plan
        .iter()
        .map(|c| c.host.name.chars().count())
        .max()
        .unwrap_or(0);
    let mut chosen = Vec::new();
    for c in &plan {
        let colored = match c.status {
            Status::New => p.green("new      "),
            Status::Changed => p.yellow("changed  "),
            Status::Unchanged => p.dim("same     "),
            Status::Invalid => p.red("skipped  "),
        };
        println!(
            "{colored}{}  {}",
            p.bold(&pad(&c.host.name, name_w)),
            target(&c.host)
        );
        for note in &c.notes {
            println!("{}{}", " ".repeat(11 + name_w), p.dim(note));
        }
        if matches!(c.status, Status::New | Status::Changed) {
            chosen.push(c.host.clone());
        }
    }
    let new = plan.iter().filter(|c| c.status == Status::New).count();
    let changed = plan.iter().filter(|c| c.status == Status::Changed).count();
    println!();
    if chosen.is_empty() {
        println!("Everything in {} is already saved.", ctx.show(&path));
        return Ok(ExitCode::SUCCESS);
    }
    if !write {
        println!(
            "{new} new, {changed} changed. Nothing saved yet: run {} to save them into \"{folder}\".",
            p.bold("setu hosts import --write")
        );
        return Ok(ExitCode::SUCCESS);
    }
    let folder = Some(folder.trim()).filter(|f| !f.is_empty());
    let summary = import::apply(&mut inventory, chosen, folder).map_err(|e| e.to_string())?;
    store.save(&inventory).map_err(|e| e.to_string())?;
    println!(
        "Saved: {} added, {} updated in {}.",
        summary.added,
        summary.updated,
        ctx.show(store.path())
    );
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(name: &str) -> Host {
        let mut h = Host::new(name, "10.0.0.1");
        h.user = Some("ops".into());
        h.port = 2222;
        h.folder = Some("Production/Web".into());
        h.tags = vec!["nginx".into()];
        h
    }

    #[test]
    fn target_leaves_out_defaults() {
        assert_eq!(target(&host("a")), "ops@10.0.0.1:2222");
        assert_eq!(target(&Host::new("b", "example.com")), "example.com");
    }

    #[test]
    fn search_covers_every_field() {
        let h = host("web-01");
        for q in ["WEB", "10.0", "ops", "production", "nginx"] {
            assert!(matches(&h, q), "{q}");
        }
        assert!(!matches(&h, "db"));
    }

    #[test]
    fn table_is_aligned_and_sorted() {
        let a = host("web-02");
        let mut b = host("db");
        b.folder = Some("A".into());
        let text = table(&[&a, &b], false);
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("NAME"));
        assert!(lines[1].starts_with("db    "));
        assert!(lines[2].starts_with("web-02"));
        assert_eq!(lines[1].find("ops@"), lines[2].find("ops@"));
    }
}
