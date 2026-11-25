//! `setu`: the Setu command line.
//!
//! It works on the same files as the desktop app (saved hosts, the vault,
//! `~/.ssh`) through the same crates, so both always agree.

// A command line prints to stdout; that is its job.
#![allow(clippy::print_stdout)]

mod doctor;
mod hosts;
mod output;

use clap::{Parser, Subcommand};
use setu_core::paths::{self, Os, ProcessEnv};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, Parser)]
#[command(
    name = "setu",
    version,
    about = "Setu, the open-source ops IDE, on the command line",
    after_help = "Docs: https://github.com/SanowerTamjit/setu/blob/master/docs/user/cli.md"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check SSH keys, ssh-agent, ~/.ssh/config and Setu's own files
    Doctor {
        /// Print the findings as JSON
        #[arg(long)]
        json: bool,
    },
    /// List or import saved hosts
    Hosts {
        #[command(subcommand)]
        command: Option<HostsCommand>,
    },
    /// Show where Setu keeps its files
    Paths,
}

#[derive(Debug, Subcommand)]
enum HostsCommand {
    /// List saved hosts, optionally only those matching SEARCH
    #[command(visible_alias = "ls")]
    List {
        /// Matches name, address, user, folder and tags (case-insensitive)
        search: Option<String>,
        /// Print the hosts as JSON
        #[arg(long)]
        json: bool,
    },
    /// Import hosts from ~/.ssh/config (shows what would change unless --write)
    Import {
        /// Save the new and changed hosts
        #[arg(long)]
        write: bool,
        /// Folder for newly imported hosts
        #[arg(long, default_value = "SSH config")]
        folder: String,
    },
}

fn home_dir() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn context() -> doctor::Context {
    doctor::Context {
        home: home_dir(),
        config_dir: paths::config_dir(&ProcessEnv, Os::current()).map_err(|e| e.to_string()),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let ctx = context();
    let result = match cli.command {
        Command::Doctor { json } => run_doctor(&ctx, json),
        Command::Hosts { command } => match command.unwrap_or(HostsCommand::List {
            search: None,
            json: false,
        }) {
            HostsCommand::List { search, json } => hosts::list(&ctx, search.as_deref(), json),
            HostsCommand::Import { write, folder } => hosts::import(&ctx, write, &folder),
        },
        Command::Paths => paths_command(&ctx),
    };
    match result {
        Ok(code) => code,
        Err(message) => {
            eprintln!("setu: {message}");
            ExitCode::from(2)
        }
    }
}

fn run_doctor(ctx: &doctor::Context, json: bool) -> Result<ExitCode, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let agent = runtime.block_on(setu_ssh::keys::agent_state());
    let checks = doctor::run(ctx, &agent);
    let totals = doctor::totals(&checks);
    if json {
        let report = serde_json::json!({ "checks": checks, "totals": totals });
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
    } else {
        print!("{}", output::doctor(&checks, totals, output::use_color()));
    }
    Ok(if totals.fail > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn paths_command(ctx: &doctor::Context) -> Result<ExitCode, String> {
    let config = ctx.config_dir.clone()?;
    let rows = [
        ("config", config.clone()),
        ("hosts", config.join(setu_hosts::HOSTS_FILE)),
        ("vault", config.join(setu_vault::VAULT_FILE)),
        ("session", config.join("session.json")),
        ("work memory", config.join("work-memory.json")),
        ("ssh config", ctx.home.join(".ssh").join("config")),
        ("known_hosts", ctx.home.join(".ssh").join("known_hosts")),
    ];
    for (name, path) in rows {
        println!("{name:<12} {}", path.display());
    }
    Ok(ExitCode::SUCCESS)
}
