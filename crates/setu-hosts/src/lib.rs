//! Saved hosts for Setu.
//!
//! Hosts live in one plain TOML file (`hosts.toml` in Setu's config folder),
//! so they can be backed up, diffed and kept in Git. Nothing secret is stored
//! here: passwords are asked for at connect time and keys stay in `~/.ssh`.
//!
//! ```toml
//! version = 1
//!
//! [[host]]
//! name = "web-01"
//! host = "10.0.4.21"
//! user = "deploy"
//! folder = "Production/Web"
//! tags = ["nginx"]
//! auth = "key"
//! key_file = "~/.ssh/id_ed25519"
//! jump = "ops@bastion.example.com"
//! ```
//!
//! [`ssh_config`] reads OpenSSH's `~/.ssh/config` and [`import`] turns its
//! hosts into entries for the inventory.

mod error;
pub mod import;
mod model;
pub mod sources;
pub mod ssh_config;
mod store;
pub mod tunnels;

pub use error::HostsError;
pub use model::{Auth, Host, Inventory, FORMAT_VERSION};
pub use store::{HostStore, HOSTS_FILE};
