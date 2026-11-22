//! Visual workflows for Setu.
//!
//! A workflow is a small graph of **steps** joined by **links**. A step runs
//! a command on one or more saved hosts, waits for a person to approve, or
//! pauses for a while. A link says when the next step runs: after the
//! previous one succeeded (the default), failed, or either way. Steps whose
//! links are all satisfied run at the same time, so a graph can fan out to
//! parallel branches and join again.
//!
//! Each workflow is one TOML file in `workflows/` in Setu's config folder:
//!
//! ```toml
//! version = 1
//! id = "deploy-web"
//! name = "Deploy web"
//!
//! [[step]]
//! id = "check"
//! name = "Check disk"
//! kind = "command"
//! command = "df -h /"
//! targets = ["tag:web"]
//!
//! [[step]]
//! id = "ok"
//! name = "Go ahead?"
//! kind = "approval"
//!
//! [[link]]
//! from = "check"
//! to = "ok"
//! ```
//!
//! [`plan`] decides which steps can run next, [`engine`] runs a workflow with
//! a [`engine::Runner`] that does the real work (SSH in the desktop app, a
//! fake in tests), and [`store`] keeps workflows and run history on disk.

pub mod engine;
mod error;
mod model;
pub mod plan;
pub mod store;
pub mod templates;

pub use error::WorkflowError;
pub use model::{
    resolve_targets, slugify, Kind, Link, Step, When, Workflow, FORMAT_VERSION, LIMITS,
};
