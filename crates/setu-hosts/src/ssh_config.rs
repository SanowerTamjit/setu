//! A reader for OpenSSH client config files (`~/.ssh/config`).
//!
//! It covers what host import needs: `Host` blocks with `*`, `?` and `!`
//! patterns, `Include` (with globs in the file name), `keyword value` and
//! `keyword=value` forms, quoted values, and OpenSSH's rule that the first
//! value found for a keyword wins. `Match` blocks are skipped with a warning,
//! because they depend on runtime conditions Setu cannot evaluate here.

use std::path::{Path, PathBuf};

/// OpenSSH's own nesting limit for `Include`.
const MAX_INCLUDE_DEPTH: usize = 16;

#[derive(Debug, Clone)]
struct Pattern {
    negated: bool,
    /// Lowercased glob.
    glob: String,
}

#[derive(Debug, Clone)]
enum Condition {
    /// Lines before the first `Host` or `Match`.
    Always,
    Host(Vec<Pattern>),
    Match,
}

#[derive(Debug, Clone)]
struct Block {
    condition: Condition,
    /// Lowercased keyword and its arguments, in file order.
    options: Vec<(String, Vec<String>)>,
}

/// A parsed config, with every `Include` already read.
#[derive(Debug, Clone, Default)]
pub struct SshConfig {
    blocks: Vec<Block>,
    /// Concrete host names from `Host` lines, in file order.
    aliases: Vec<String>,
    /// Things the reader skipped, worded for the user.
    pub warnings: Vec<String>,
}

/// The options that apply to one host after matching every block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolved {
    options: Vec<(String, Vec<String>)>,
}

impl Resolved {
    /// The first argument of `keyword` (case-insensitive), if set.
    pub fn get(&self, keyword: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(keyword))
            .and_then(|(_, args)| args.first())
            .map(String::as_str)
    }

    /// Whether `keyword` was set at all.
    pub fn has(&self, keyword: &str) -> bool {
        self.options
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case(keyword))
    }
}

impl SshConfig {
    /// Reads `path` and everything it includes. Relative `Include` paths are
    /// resolved against `home/.ssh`, as OpenSSH does for the user config.
    pub fn load(path: &Path, home: &Path) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Ok(Self::parse_with_includes(&text, home))
    }

    /// Parses config text; `Include` lines are read from disk.
    pub fn parse_with_includes(text: &str, home: &Path) -> Self {
        let mut config = SshConfig {
            blocks: vec![Block {
                condition: Condition::Always,
                options: Vec::new(),
            }],
            ..SshConfig::default()
        };
        let mut warned_match = false;
        config.parse_into(text, home, 0, &mut warned_match);
        config
    }

    fn parse_into(&mut self, text: &str, home: &Path, depth: usize, warned_match: &mut bool) {
        for line in text.lines() {
            let Some((keyword, args)) = split_line(line) else {
                continue;
            };
            match keyword.as_str() {
                "host" => {
                    let patterns: Vec<Pattern> = args
                        .iter()
                        .map(|a| match a.strip_prefix('!') {
                            Some(rest) => Pattern {
                                negated: true,
                                glob: rest.to_ascii_lowercase(),
                            },
                            None => Pattern {
                                negated: false,
                                glob: a.to_ascii_lowercase(),
                            },
                        })
                        .collect();
                    for arg in &args {
                        let concrete = !arg.starts_with('!') && !arg.contains(['*', '?']);
                        if concrete && !self.aliases.iter().any(|a| a.eq_ignore_ascii_case(arg)) {
                            self.aliases.push(arg.clone());
                        }
                    }
                    self.blocks.push(Block {
                        condition: Condition::Host(patterns),
                        options: Vec::new(),
                    });
                }
                "match" => {
                    if !*warned_match {
                        self.warnings.push(
                            "Match blocks were skipped; hosts are imported without their settings."
                                .into(),
                        );
                        *warned_match = true;
                    }
                    self.blocks.push(Block {
                        condition: Condition::Match,
                        options: Vec::new(),
                    });
                }
                "include" => {
                    if depth >= MAX_INCLUDE_DEPTH {
                        self.warnings
                            .push("Include is nested too deeply; stopped reading includes.".into());
                        continue;
                    }
                    // Included lines join the current block, but a Host inside the
                    // included file must not leak into the lines after Include.
                    let resume = self.current_condition();
                    let before = self.blocks.len();
                    for arg in &args {
                        for file in self.expand_include(arg, home) {
                            match std::fs::read_to_string(&file) {
                                Ok(text) => self.parse_into(&text, home, depth + 1, warned_match),
                                Err(e) => self.warnings.push(format!(
                                    "Could not read included file {}: {e}",
                                    file.display()
                                )),
                            }
                        }
                    }
                    if self.blocks.len() != before {
                        self.blocks.push(Block {
                            condition: resume,
                            options: Vec::new(),
                        });
                    }
                }
                _ => {
                    if let Some(block) = self.blocks.last_mut() {
                        block.options.push((keyword, args));
                    }
                }
            }
        }
    }

    fn current_condition(&self) -> Condition {
        self.blocks
            .last()
            .map(|b| b.condition.clone())
            .unwrap_or(Condition::Always)
    }

    fn expand_include(&mut self, arg: &str, home: &Path) -> Vec<PathBuf> {
        let path = if let Some(rest) = arg.strip_prefix("~/") {
            home.join(rest)
        } else if Path::new(arg).is_absolute() {
            PathBuf::from(arg)
        } else {
            home.join(".ssh").join(arg)
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
        if parent.to_string_lossy().contains(['*', '?']) {
            self.warnings.push(format!(
                "Include {arg}: wildcards are only supported in the file name."
            ));
            return Vec::new();
        }
        if !name.contains(['*', '?']) {
            // OpenSSH ignores includes that match nothing.
            return if path.is_file() {
                vec![path]
            } else {
                Vec::new()
            };
        }
        let glob = name.to_ascii_lowercase();
        let mut files: Vec<PathBuf> = match std::fs::read_dir(&parent) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .filter(|p| {
                    p.file_name()
                        .map(|n| glob_match(&glob, &n.to_string_lossy().to_ascii_lowercase()))
                        .unwrap_or(false)
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        files.sort();
        files
    }

    /// Concrete host names (no wildcards or negations), in file order.
    pub fn aliases(&self) -> &[String] {
        &self.aliases
    }

    /// The options OpenSSH would use for `alias`: every matching block in
    /// order, first value per keyword wins.
    pub fn resolve(&self, alias: &str) -> Resolved {
        let name = alias.to_ascii_lowercase();
        let mut out = Resolved::default();
        for block in &self.blocks {
            let applies = match &block.condition {
                Condition::Always => true,
                Condition::Match => false,
                Condition::Host(patterns) => host_matches(patterns, &name),
            };
            if !applies {
                continue;
            }
            for (keyword, args) in &block.options {
                if !out.has(keyword) {
                    out.options.push((keyword.clone(), args.clone()));
                }
            }
        }
        out
    }
}

/// Splits a config line into a lowercased keyword and its arguments.
fn split_line(line: &str) -> Option<(String, Vec<String>)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let end = line
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(line.len());
    let keyword = line[..end].to_ascii_lowercase();
    let mut rest = line[end..].trim_start();
    if let Some(after) = rest.strip_prefix('=') {
        rest = after.trim_start();
    }
    Some((keyword, split_args(rest)))
}

/// Whitespace-separated arguments; double quotes group, `\"` and `\\` escape.
fn split_args(s: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            '\\' if matches!(chars.peek(), Some('"') | Some('\\')) => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
                has_token = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if has_token {
                    args.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            c => {
                current.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        args.push(current);
    }
    args
}

fn host_matches(patterns: &[Pattern], name: &str) -> bool {
    let mut matched = false;
    for p in patterns {
        if glob_match(&p.glob, name) {
            if p.negated {
                return false;
            }
            matched = true;
        }
    }
    matched
}

/// `*` matches any run of characters and `?` exactly one.
pub(crate) fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> SshConfig {
        SshConfig::parse_with_includes(text, Path::new("/nonexistent-home"))
    }

    #[test]
    fn globs() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("web-??", "web-01"));
        assert!(!glob_match("web-??", "web-1"));
        assert!(glob_match("*.example.com", "a.b.example.com"));
        assert!(!glob_match("*.example.com", "example.com"));
        assert!(glob_match("a*b*c", "axxbyyc"));
    }

    #[test]
    fn line_forms() {
        assert_eq!(
            split_line("  HostName=10.0.0.1"),
            Some(("hostname".into(), vec!["10.0.0.1".into()]))
        );
        assert_eq!(
            split_line("User = deploy"),
            Some(("user".into(), vec!["deploy".into()]))
        );
        assert_eq!(
            split_line(r#"IdentityFile "~/my keys/id_ed25519""#),
            Some(("identityfile".into(), vec!["~/my keys/id_ed25519".into()]))
        );
        assert_eq!(split_line("# comment"), None);
        assert_eq!(split_line("   "), None);
    }

    #[test]
    fn first_value_wins_and_wildcards_apply() {
        let config = parse(
            "User global\n\
             Host web-01 web-02\n  HostName 10.0.0.%h\n  Port 2222\n\
             Host web-*\n  User www\n  Port 22\n\
             Host * !bastion\n  ProxyJump bastion\n\
             Host bastion\n  HostName 203.0.113.10\n",
        );
        assert_eq!(config.aliases(), ["web-01", "web-02", "bastion"]);
        let web = config.resolve("web-01");
        // The global User comes first in the file, so it wins over `Host web-*`.
        assert_eq!(web.get("user"), Some("global"));
        assert_eq!(web.get("port"), Some("2222"));
        assert_eq!(web.get("proxyjump"), Some("bastion"));
        let bastion = config.resolve("BASTION");
        assert_eq!(bastion.get("hostname"), Some("203.0.113.10"));
        assert_eq!(bastion.get("proxyjump"), None);
    }

    #[test]
    fn match_blocks_are_skipped_with_a_warning() {
        let config = parse("Match host x\n  User nope\nMatch all\nHost a\n  User yes\n");
        assert_eq!(config.resolve("a").get("user"), Some("yes"));
        assert_eq!(config.warnings.len(), 1);
    }

    #[test]
    fn includes_with_globs_and_scoping() {
        let home = tempfile::tempdir().expect("tempdir");
        let ssh = home.path().join(".ssh");
        std::fs::create_dir_all(ssh.join("config.d")).expect("mkdir");
        std::fs::write(
            ssh.join("config.d/10-prod"),
            "Host prod-db\n  HostName db.prod\n",
        )
        .expect("write");
        std::fs::write(
            ssh.join("config.d/20-dev.conf"),
            "Host dev\n  HostName dev.local\n",
        )
        .expect("write");
        let text = "Host jump\n  Include config.d/*\n  User ops\nHost other\n";
        let config = SshConfig::parse_with_includes(text, home.path());
        assert_eq!(config.aliases(), ["jump", "prod-db", "dev", "other"]);
        // `User ops` after the Include still belongs to `Host jump`.
        assert_eq!(config.resolve("jump").get("user"), Some("ops"));
        assert_eq!(config.resolve("dev").get("user"), None);
        assert_eq!(config.resolve("prod-db").get("hostname"), Some("db.prod"));
        assert!(config.warnings.is_empty());
    }

    #[test]
    fn self_include_stops_at_the_depth_limit() {
        let home = tempfile::tempdir().expect("tempdir");
        let ssh = home.path().join(".ssh");
        std::fs::create_dir_all(&ssh).expect("mkdir");
        std::fs::write(ssh.join("loop"), "Include loop\nHost x\n").expect("write");
        let config = SshConfig::parse_with_includes("Include loop\n", home.path());
        assert_eq!(config.aliases(), ["x"]);
        assert!(!config.warnings.is_empty());
    }
}
