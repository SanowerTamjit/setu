use crate::error::{invalid, WorkflowError};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// The file format this version writes.
pub const FORMAT_VERSION: u32 = 1;

/// Size limits, checked by [`Workflow::validate`] so a hand-edited or
/// imported file cannot make the app do unbounded work.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub steps: usize,
    pub links: usize,
    pub targets: usize,
    pub name: usize,
    pub command: usize,
    pub message: usize,
    pub wait_seconds: u32,
    pub timeout_seconds: u32,
}

pub const LIMITS: Limits = Limits {
    steps: 100,
    links: 400,
    targets: 200,
    name: 100,
    command: 8 * 1024,
    message: 2000,
    wait_seconds: 24 * 60 * 60,
    timeout_seconds: 24 * 60 * 60,
};

/// What a step does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Runs `command` on every target host.
    #[default]
    Command,
    /// Waits until a person approves (the run goes on) or rejects (the step fails).
    Approval,
    /// Waits `seconds`.
    Wait,
}

/// When a link lets the next step run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum When {
    #[default]
    Success,
    Failure,
    Always,
}

impl When {
    fn is_default(&self) -> bool {
        *self == When::Success
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    /// Unique in the workflow; letters, digits, `-` and `_`.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: Kind,
    /// For commands: what runs on each host, through the login shell like `ssh host command`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    /// For commands: saved host names, or `tag:<tag>` for every host with that tag.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<String>,
    /// For commands: one host after another (stopping at the first failure)
    /// instead of all at once.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub one_at_a_time: bool,
    /// For commands: stop waiting after this many seconds (the step fails).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u32>,
    /// For approvals: what to check before approving.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    /// For waits.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub seconds: u32,
    /// Where the step sits on the canvas.
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl Step {
    /// A command step with the required fields.
    pub fn command(id: &str, name: &str, command: &str, targets: &[&str]) -> Self {
        Step {
            id: id.into(),
            name: name.into(),
            kind: Kind::Command,
            command: command.into(),
            targets: targets.iter().map(|t| t.to_string()).collect(),
            one_at_a_time: false,
            timeout_seconds: None,
            message: String::new(),
            seconds: 0,
            x: 0.0,
            y: 0.0,
        }
    }

    pub fn approval(id: &str, name: &str, message: &str) -> Self {
        Step {
            kind: Kind::Approval,
            message: message.into(),
            ..Step::command(id, name, "", &[])
        }
    }

    pub fn wait(id: &str, name: &str, seconds: u32) -> Self {
        Step {
            kind: Kind::Wait,
            seconds,
            ..Step::command(id, name, "", &[])
        }
    }

    pub fn at(mut self, x: f64, y: f64) -> Self {
        self.x = x;
        self.y = y;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "When::is_default")]
    pub when: When,
}

impl Link {
    pub fn new(from: &str, to: &str, when: When) -> Self {
        Link {
            from: from.into(),
            to: to.into(),
            when,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workflow {
    #[serde(default = "format_version")]
    pub version: u32,
    /// The file name without `.toml`; letters, digits, `-` and `_`.
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, rename = "step", alias = "steps")]
    pub steps: Vec<Step>,
    #[serde(default, rename = "link", alias = "links")]
    pub links: Vec<Link>,
}

fn format_version() -> u32 {
    FORMAT_VERSION
}

/// Whether `s` is a safe identifier: it becomes a file name, so it must not
/// hold path separators, dots or anything a shell or file system treats
/// specially.
fn is_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Turns a name into an id: lower case, runs of other characters become `-`.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 48 {
            break;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "workflow".into()
    } else {
        out
    }
}

fn check_text(what: &str, s: &str, max: usize) -> Result<(), WorkflowError> {
    if s.len() > max {
        return Err(invalid(format!("{what} is longer than {max} characters")));
    }
    if s.chars().any(|c| c == '\0') {
        return Err(invalid(format!("{what} contains a NUL character")));
    }
    Ok(())
}

impl Workflow {
    pub fn new(id: &str, name: &str) -> Self {
        Workflow {
            version: FORMAT_VERSION,
            id: id.into(),
            name: name.into(),
            description: String::new(),
            steps: Vec::new(),
            links: Vec::new(),
        }
    }

    pub fn step(&self, id: &str) -> Option<&Step> {
        self.steps.iter().find(|s| s.id == id)
    }

    /// Checks every rule: ids, sizes, links that point at real steps, and no
    /// loops. The desktop app calls this on every save and run, whatever
    /// the web view already checked.
    pub fn validate(&self) -> Result<(), WorkflowError> {
        if !is_id(&self.id) {
            return Err(invalid(
                "the workflow id must be 1 to 64 letters, digits, - or _",
            ));
        }
        if self.name.trim().is_empty() {
            return Err(invalid("give the workflow a name"));
        }
        check_text("the workflow name", &self.name, LIMITS.name)?;
        check_text("the description", &self.description, LIMITS.message)?;
        if self.steps.len() > LIMITS.steps {
            return Err(invalid(format!(
                "a workflow can have at most {} steps",
                LIMITS.steps
            )));
        }
        if self.links.len() > LIMITS.links {
            return Err(invalid(format!(
                "a workflow can have at most {} links",
                LIMITS.links
            )));
        }
        let mut ids = HashSet::new();
        for step in &self.steps {
            self.validate_step(step)?;
            if !ids.insert(step.id.as_str()) {
                return Err(invalid(format!("two steps have the id `{}`", step.id)));
            }
        }
        let mut seen = HashSet::new();
        for link in &self.links {
            for end in [&link.from, &link.to] {
                if !ids.contains(end.as_str()) {
                    return Err(invalid(format!("a link points at a missing step `{end}`")));
                }
            }
            if link.from == link.to {
                return Err(invalid(format!(
                    "step `{}` cannot link to itself",
                    link.from
                )));
            }
            if !seen.insert((&link.from, &link.to)) {
                return Err(invalid(format!(
                    "`{}` is linked to `{}` twice",
                    link.from, link.to
                )));
            }
        }
        if let Some(step) = self.find_loop() {
            return Err(invalid(format!(
                "the links make a loop through `{step}`; a workflow must always move forward"
            )));
        }
        Ok(())
    }

    fn validate_step(&self, step: &Step) -> Result<(), WorkflowError> {
        if !is_id(&step.id) {
            return Err(invalid(format!(
                "step id `{}` must be 1 to 64 letters, digits, - or _",
                step.id.chars().take(64).collect::<String>()
            )));
        }
        let label = if step.name.trim().is_empty() {
            step.id.clone()
        } else {
            step.name.clone()
        };
        check_text("a step name", &step.name, LIMITS.name)?;
        check_text(
            &format!("the message of `{label}`"),
            &step.message,
            LIMITS.message,
        )?;
        match step.kind {
            Kind::Command => {
                if step.command.trim().is_empty() {
                    return Err(invalid(format!("step `{label}` has no command")));
                }
                check_text(
                    &format!("the command of `{label}`"),
                    &step.command,
                    LIMITS.command,
                )?;
                if step.targets.is_empty() {
                    return Err(invalid(format!("step `{label}` has no hosts to run on")));
                }
                if step.targets.len() > LIMITS.targets {
                    return Err(invalid(format!(
                        "step `{label}` has more than {} targets",
                        LIMITS.targets
                    )));
                }
                for t in &step.targets {
                    if t.trim().is_empty() {
                        return Err(invalid(format!("step `{label}` has an empty host name")));
                    }
                    check_text("a host name", t, LIMITS.name)?;
                }
                if let Some(t) = step.timeout_seconds {
                    if t == 0 || t > LIMITS.timeout_seconds {
                        return Err(invalid(format!(
                            "the timeout of `{label}` must be 1 to {} seconds",
                            LIMITS.timeout_seconds
                        )));
                    }
                }
            }
            Kind::Approval => {}
            Kind::Wait => {
                if step.seconds == 0 || step.seconds > LIMITS.wait_seconds {
                    return Err(invalid(format!(
                        "the wait in `{label}` must be 1 to {} seconds",
                        LIMITS.wait_seconds
                    )));
                }
            }
        }
        if !step.x.is_finite() || !step.y.is_finite() {
            return Err(invalid(format!("step `{label}` has no position")));
        }
        Ok(())
    }

    /// A step on a loop, if the links make one (depth-first search).
    fn find_loop(&self) -> Option<String> {
        let mut next: HashMap<&str, Vec<&str>> = HashMap::new();
        for l in &self.links {
            next.entry(l.from.as_str()).or_default().push(l.to.as_str());
        }
        // 0 = not visited, 1 = on the current path, 2 = done.
        let mut mark: HashMap<&str, u8> = HashMap::new();
        for start in self.steps.iter().map(|s| s.id.as_str()) {
            if mark.get(start).copied().unwrap_or(0) != 0 {
                continue;
            }
            let mut stack: Vec<(&str, usize)> = vec![(start, 0)];
            mark.insert(start, 1);
            while let Some((node, i)) = stack.pop() {
                let children = next.get(node).map(Vec::as_slice).unwrap_or(&[]);
                if let Some(&child) = children.get(i) {
                    stack.push((node, i + 1));
                    match mark.get(child).copied().unwrap_or(0) {
                        1 => return Some(child.to_string()),
                        0 => {
                            mark.insert(child, 1);
                            stack.push((child, 0));
                        }
                        _ => {}
                    }
                } else {
                    mark.insert(node, 2);
                }
            }
        }
        None
    }
}

/// Expands a step's targets against the saved hosts: names stay as they
/// are, `tag:<tag>` becomes every host carrying the tag (case-insensitive).
/// Duplicates are dropped, the order is kept. `hosts` is `(name, tags)`.
pub fn resolve_targets(
    targets: &[String],
    hosts: &[(String, Vec<String>)],
) -> Result<Vec<String>, WorkflowError> {
    let mut out: Vec<String> = Vec::new();
    let push = |name: &str, out: &mut Vec<String>| {
        if !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
    };
    for t in targets {
        let t = t.trim();
        if let Some(tag) = t.strip_prefix("tag:") {
            let tag = tag.trim();
            let mut any = false;
            for (name, tags) in hosts {
                if tags.iter().any(|x| x.eq_ignore_ascii_case(tag)) {
                    push(name, &mut out);
                    any = true;
                }
            }
            if !any {
                return Err(invalid(format!("no saved host has the tag `{tag}`")));
            }
        } else if hosts.iter().any(|(name, _)| name == t) {
            push(t, &mut out);
        } else {
            return Err(invalid(format!("there is no saved host called `{t}`")));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Workflow {
        let mut w = Workflow::new("deploy", "Deploy");
        w.steps = vec![
            Step::command("a", "A", "uptime", &["web-01"]),
            Step::approval("b", "B", "ok?"),
            Step::wait("c", "C", 5),
        ];
        w.links = vec![
            Link::new("a", "b", When::Success),
            Link::new("b", "c", When::Always),
        ];
        w
    }

    #[test]
    fn sample_is_valid() {
        sample().validate().expect("valid");
    }

    #[test]
    fn toml_round_trip_is_short_and_readable() {
        let w = sample();
        let text = toml::to_string(&w).expect("toml");
        assert!(text.contains("[[step]]"), "{text}");
        assert!(text.contains("[[link]]"), "{text}");
        // Defaults stay out of the file.
        assert!(!text.contains("when = \"success\""), "{text}");
        assert!(text.contains("when = \"always\""), "{text}");
        let back: Workflow = toml::from_str(&text).expect("parse");
        assert_eq!(back, w);
    }

    #[test]
    fn rejects_ids_that_could_escape_the_folder() {
        for id in ["../x", "a/b", "a.b", "", "a b", "a\\b"] {
            let mut w = sample();
            w.id = id.into();
            assert!(w.validate().is_err(), "{id:?} accepted");
        }
    }

    #[test]
    fn rejects_loops() {
        let mut w = sample();
        w.links.push(Link::new("c", "a", When::Success));
        let err = w.validate().expect_err("loop").to_string();
        assert!(err.contains("loop"), "{err}");
    }

    #[test]
    fn rejects_bad_links_and_steps() {
        let mut w = sample();
        w.links.push(Link::new("a", "zzz", When::Success));
        assert!(w.validate().is_err());

        let mut w = sample();
        w.links.push(Link::new("a", "a", When::Success));
        assert!(w.validate().is_err());

        let mut w = sample();
        w.links.push(Link::new("a", "b", When::Failure));
        assert!(w.validate().is_err(), "duplicate link");

        let mut w = sample();
        w.steps[0].targets.clear();
        assert!(w.validate().is_err(), "no targets");

        let mut w = sample();
        w.steps[0].command = "  ".into();
        assert!(w.validate().is_err(), "no command");

        let mut w = sample();
        w.steps[2].seconds = 0;
        assert!(w.validate().is_err(), "zero wait");

        let mut w = sample();
        w.steps[1].id = "a".into();
        assert!(w.validate().is_err(), "duplicate id");

        let mut w = sample();
        w.steps[0].command = "x".repeat(LIMITS.command + 1);
        assert!(w.validate().is_err(), "huge command");
    }

    #[test]
    fn slugify_makes_safe_ids() {
        assert_eq!(slugify("Deploy web (prod)!"), "deploy-web-prod");
        assert_eq!(slugify("../../etc"), "etc");
        assert_eq!(slugify("***"), "workflow");
        assert!(is_id(&slugify(&"é".repeat(200))));
    }

    #[test]
    fn targets_expand_tags_and_keep_order() {
        let hosts = vec![
            ("web-01".to_string(), vec!["web".to_string()]),
            ("web-02".to_string(), vec!["Web".to_string()]),
            ("db".to_string(), vec![]),
        ];
        let got = resolve_targets(&["db".into(), "tag:web".into(), "web-01".into()], &hosts)
            .expect("resolve");
        assert_eq!(got, ["db", "web-01", "web-02"]);
        assert!(resolve_targets(&["nope".into()], &hosts).is_err());
        assert!(resolve_targets(&["tag:none".into()], &hosts).is_err());
    }
}
