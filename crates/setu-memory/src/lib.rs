//! Work memory for Setu.
//!
//! Setu can remember the commands you type in its terminals, so it can
//! notice work you do again and again and offer to automate it:
//!
//! - a long command you keep typing becomes a **snippet** in the palette;
//! - the same few commands in the same order become a **workflow**;
//! - one command run on several saved hosts becomes a **fleet workflow**
//!   that runs it on all of them (or on their shared tag).
//!
//! Everything stays on this device, in one file (`work-memory.json`, 0600)
//! in Setu's config folder. Nothing is sent anywhere. Remembering is off
//! until the person turns it on, and they can pause it, look through what
//! is kept, delete entries or wipe it.
//!
//! Secrets are kept out in two layers: the terminal skips lines typed at a
//! password prompt or without echo, and [`redact`] masks secrets in what
//! is left (password flags, tokens, `user:pass@` URLs, `TOKEN=` variables).
//! A command that needed masking is kept masked and is never suggested.
//!
//! A suggestion never runs anything: it opens a workflow draft in the
//! builder, or saves a snippet that types a command without pressing Enter.

pub mod redact;
pub mod store;
pub mod suggest;

pub use redact::{redact, Redacted};
pub use store::Recorded;
pub use store::{Entry, MemoryError, MemoryFile, MemoryStore, Settings, Snippet};
pub use suggest::{suggestions, summary, to_workflow, Suggestion, SuggestionKind};
