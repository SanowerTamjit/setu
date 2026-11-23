//! The task board: cards for ops work in three columns (to do, doing,
//! done), each optionally linked to hosts and to a workflow. Kept in
//! `board.toml` in Setu's config folder; order in the file is the order
//! on the board.

use crate::error::{invalid, WorkflowError};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const BOARD_VERSION: u32 = 1;
pub const MAX_CARDS: usize = 500;
pub const MAX_TITLE: usize = 200;
pub const MAX_NOTES: usize = 4000;
pub const MAX_HOSTS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Column {
    #[default]
    Todo,
    Doing,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Low,
    #[default]
    Normal,
    High,
}

impl Priority {
    fn is_normal(&self) -> bool {
        *self == Priority::Normal
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    /// Letters, digits, `-` and `_`.
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub column: Column,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    /// Saved host names or `tag:<tag>`, as in workflows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hosts: Vec<String>,
    /// The id of a workflow that does this job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<String>,
    #[serde(default, skip_serializing_if = "Priority::is_normal")]
    pub priority: Priority,
    /// A due date as `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Board {
    #[serde(default = "board_version")]
    pub version: u32,
    #[serde(default, rename = "card", alias = "cards")]
    pub cards: Vec<Card>,
}

impl Default for Board {
    fn default() -> Self {
        Board {
            version: BOARD_VERSION,
            cards: Vec::new(),
        }
    }
}

fn board_version() -> u32 {
    BOARD_VERSION
}

fn is_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// `YYYY-MM-DD` with a plausible month and day.
fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let num = |r: std::ops::Range<usize>| s.get(r).and_then(|x| x.parse::<u32>().ok());
    matches!(
        (num(0..4), num(5..7), num(8..10)),
        (Some(1970..=9999), Some(1..=12), Some(1..=31))
    )
}

fn text(what: &str, s: &str, max: usize) -> Result<(), WorkflowError> {
    if s.chars().count() > max {
        return Err(invalid(format!("{what} is longer than {max} characters")));
    }
    if s.chars().any(|c| c == '\0') {
        return Err(invalid(format!("{what} contains a NUL character")));
    }
    Ok(())
}

impl Board {
    /// Checks every card; the desktop app calls this on load and save.
    pub fn validate(&self) -> Result<(), WorkflowError> {
        if self.cards.len() > MAX_CARDS {
            return Err(invalid(format!(
                "the board can hold at most {MAX_CARDS} cards"
            )));
        }
        let mut ids = HashSet::new();
        for c in &self.cards {
            if !is_id(&c.id) {
                return Err(invalid("a card id must be 1 to 64 letters, digits, - or _"));
            }
            if !ids.insert(c.id.as_str()) {
                return Err(invalid(format!("two cards have the id `{}`", c.id)));
            }
            if c.title.trim().is_empty() {
                return Err(invalid("every card needs a title"));
            }
            text("a card title", &c.title, MAX_TITLE)?;
            text(&format!("the notes of `{}`", c.title), &c.notes, MAX_NOTES)?;
            if c.hosts.len() > MAX_HOSTS {
                return Err(invalid(format!(
                    "`{}` lists more than {MAX_HOSTS} hosts",
                    c.title
                )));
            }
            for h in &c.hosts {
                if h.trim().is_empty() {
                    return Err(invalid(format!("`{}` has an empty host name", c.title)));
                }
                text("a host name", h, 100)?;
            }
            if let Some(w) = &c.workflow {
                if !is_id(w) {
                    return Err(invalid(format!(
                        "`{}` links to an invalid workflow id",
                        c.title
                    )));
                }
            }
            if let Some(d) = &c.due {
                if !is_date(d) {
                    return Err(invalid(format!(
                        "the due date of `{}` must look like 2026-12-31",
                        c.title
                    )));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str, column: Column) -> Card {
        Card {
            id: id.into(),
            title: format!("Card {id}"),
            column,
            notes: String::new(),
            hosts: vec!["tag:web".into()],
            workflow: Some("deploy-web".into()),
            priority: Priority::Normal,
            due: Some("2026-12-01".into()),
        }
    }

    #[test]
    fn round_trip_keeps_order_and_skips_defaults() {
        let b = Board {
            version: 1,
            cards: vec![card("a", Column::Doing), card("b", Column::Todo)],
        };
        b.validate().expect("valid");
        let text = toml::to_string(&b).expect("toml");
        assert!(text.contains("[[card]]"), "{text}");
        assert!(!text.contains("priority"), "{text}");
        let back: Board = toml::from_str(&text).expect("parse");
        assert_eq!(back, b);
    }

    #[test]
    fn rejects_bad_cards() {
        let bad = |f: &dyn Fn(&mut Card)| {
            let mut c = card("a", Column::Todo);
            f(&mut c);
            Board {
                version: 1,
                cards: vec![c],
            }
            .validate()
            .is_err()
        };
        assert!(bad(&|c| c.id = "../x".into()));
        assert!(bad(&|c| c.title = "  ".into()));
        assert!(bad(&|c| c.title = "x".repeat(MAX_TITLE + 1)));
        assert!(bad(&|c| c.notes = "x".repeat(MAX_NOTES + 1)));
        assert!(bad(&|c| c.workflow = Some("a/b".into())));
        assert!(bad(&|c| c.due = Some("tomorrow".into())));
        assert!(bad(&|c| c.due = Some("2026-13-01".into())));
        assert!(bad(&|c| c.hosts = vec!["".into()]));
        let dup = Board {
            version: 1,
            cards: vec![card("a", Column::Todo), card("a", Column::Done)],
        };
        assert!(dup.validate().is_err());
    }
}
