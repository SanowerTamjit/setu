//! Text output: aligned, with colour only on a terminal that wants it.

use crate::doctor::{Check, Level, Totals};
use std::fmt::Write;
use std::io::IsTerminal;

/// Colour when stdout is a terminal and `NO_COLOR` is not set.
pub fn use_color() -> bool {
    std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty()) && std::io::stdout().is_terminal()
}

pub struct Paint {
    on: bool,
}

impl Paint {
    pub fn new(on: bool) -> Self {
        Paint { on }
    }

    fn wrap(&self, code: &str, text: &str) -> String {
        if self.on {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    pub fn bold(&self, t: &str) -> String {
        self.wrap("1", t)
    }
    pub fn dim(&self, t: &str) -> String {
        self.wrap("2", t)
    }
    pub fn green(&self, t: &str) -> String {
        self.wrap("32", t)
    }
    pub fn yellow(&self, t: &str) -> String {
        self.wrap("33", t)
    }
    pub fn red(&self, t: &str) -> String {
        self.wrap("31", t)
    }
    pub fn cyan(&self, t: &str) -> String {
        self.wrap("36", t)
    }
}

fn symbol(p: &Paint, level: Level) -> String {
    match level {
        Level::Ok => p.green("✓"),
        Level::Info => p.cyan("·"),
        Level::Warn => p.yellow("!"),
        Level::Fail => p.red("✗"),
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The doctor report, grouped by section in the order the checks ran.
pub fn doctor(checks: &[Check], totals: Totals, color: bool) -> String {
    let p = Paint::new(color);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} {}",
        p.bold("Setu doctor"),
        p.dim(&format!(
            "{} · {}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS
        ))
    );
    let mut section = "";
    for check in checks {
        if check.section != section {
            section = check.section;
            let _ = writeln!(out, "\n{}", p.bold(section));
        }
        let _ = writeln!(out, "  {} {}", symbol(&p, check.level), check.title);
        for d in &check.details {
            let _ = writeln!(out, "      {}", p.dim(d));
        }
        if let Some(fix) = &check.fix {
            if check.level >= Level::Warn {
                let _ = writeln!(out, "      {} {fix}", p.yellow("fix:"));
            } else {
                let _ = writeln!(out, "      {}", p.dim(&format!("tip: {fix}")));
            }
        }
    }
    let mut parts = vec![p.green(&format!("{} ok", totals.ok))];
    if totals.warn > 0 {
        parts.push(p.yellow(&plural(totals.warn, "warning", "warnings")));
    }
    if totals.fail > 0 {
        parts.push(p.red(&plural(totals.fail, "problem", "problems")));
    }
    let verdict = if totals.fail > 0 {
        "Fix the problems marked ✗; SSH will fail until then."
    } else if totals.warn > 0 {
        "Nothing blocks SSH; the warnings are worth a look."
    } else {
        "All good."
    };
    let _ = writeln!(out, "\n{}  {}", parts.join(", "), verdict);
    out
}

/// Pads `s` to `width` characters (not bytes), for tables.
pub fn pad(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - n))
    }
}
