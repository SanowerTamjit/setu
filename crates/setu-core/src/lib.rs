//! Shared types and configuration for Setu.
//!
//! `setu-core` is the dependency every other Setu crate and client builds on.
//! It holds no protocol or UI code: only identifiers, configuration paths and
//! common error types, so it stays small and fast to compile.

pub mod extend;
pub mod paths;

/// The Setu version, taken from the workspace `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Human-readable product name.
pub const PRODUCT_NAME: &str = "Setu";
