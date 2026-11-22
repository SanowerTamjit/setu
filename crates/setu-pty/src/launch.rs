//! Shell profiles the user defines on top of a detected shell: extra
//! arguments, a starting folder and environment variables.
//!
//! For a WSL distro the folder and variables have to cross into Linux, so
//! they go through `wsl.exe --cd` and `WSLENV` instead of the Windows process.

use crate::session::SpawnOptions;
use crate::shells::{ShellKind, ShellProfile};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// What a profile changes about its base shell.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Launch {
    /// Added after the shell's own arguments.
    pub args: Vec<String>,
    /// Starting folder; `~` and `~/…` mean your home folder. Empty: home.
    pub cwd: String,
    /// Extra environment variables.
    pub env: Vec<(String, String)>,
}

/// Why a profile cannot start.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LaunchError {
    #[error("`{0}` is not a valid environment variable name (use letters, digits and _)")]
    BadName(String),
    #[error("the starting folder `{0}` does not exist")]
    NoFolder(String),
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn expand_home(path: &str, home: &Path) -> PathBuf {
    if path == "~" {
        home.to_path_buf()
    } else if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        home.join(rest)
    } else {
        PathBuf::from(path)
    }
}

/// The spawn options for `shell` changed by `launch`. `is_dir` checks the
/// starting folder (a parameter so tests need no real folders).
pub fn spawn_options(
    shell: &ShellProfile,
    launch: &Launch,
    home: &Path,
    is_dir: impl Fn(&Path) -> bool,
) -> Result<SpawnOptions, LaunchError> {
    for (name, _) in &launch.env {
        if !valid_name(name) {
            return Err(LaunchError::BadName(name.clone()));
        }
    }
    let mut opts = SpawnOptions::from(shell);
    let cwd = launch.cwd.trim();
    if shell.kind == ShellKind::Wsl {
        // Inside Linux: wsl.exe resolves the folder itself (`~` is the Linux home).
        if !cwd.is_empty() {
            opts.args.push("--cd".into());
            opts.args.push(cwd.into());
        }
        if !launch.env.is_empty() {
            let mut names: Vec<String> = std::env::var("WSLENV")
                .ok()
                .filter(|v| !v.is_empty())
                .map(|v| v.split(':').map(String::from).collect())
                .unwrap_or_default();
            for (name, value) in &launch.env {
                names.push(format!("{name}/u"));
                opts = opts.env(name.clone(), value.clone());
            }
            opts = opts.env("WSLENV", names.join(":"));
        }
        if !launch.args.is_empty() {
            opts.args.push("--".into());
            opts.args.extend(launch.args.iter().cloned());
        }
        return Ok(opts);
    }
    if !cwd.is_empty() {
        let dir = expand_home(cwd, home);
        if !is_dir(&dir) {
            return Err(LaunchError::NoFolder(cwd.to_string()));
        }
        opts = opts.cwd(dir);
    }
    for (name, value) in &launch.env {
        opts = opts.env(name.clone(), value.clone());
    }
    opts.args.extend(launch.args.iter().cloned());
    Ok(opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bash() -> ShellProfile {
        ShellProfile {
            id: "bash".into(),
            name: "bash".into(),
            kind: ShellKind::Bash,
            program: "/bin/bash".into(),
            args: vec!["-l".into()],
        }
    }

    fn ubuntu() -> ShellProfile {
        ShellProfile {
            id: "wsl:Ubuntu".into(),
            name: "Ubuntu (WSL)".into(),
            kind: ShellKind::Wsl,
            program: r"C:\Windows\System32\wsl.exe".into(),
            args: vec!["-d".into(), "Ubuntu".into()],
        }
    }

    fn launch(cwd: &str, env: &[(&str, &str)], args: &[&str]) -> Launch {
        Launch {
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: cwd.into(),
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn local_profile_adds_folder_env_and_args() {
        let opts = spawn_options(
            &bash(),
            &launch("~/work", &[("AWS_PROFILE", "prod")], &["--noprofile"]),
            Path::new("/home/me"),
            |p| p == Path::new("/home/me/work"),
        )
        .expect("ok");
        assert_eq!(opts.args, vec!["-l", "--noprofile"]);
        assert_eq!(opts.cwd.as_deref(), Some(Path::new("/home/me/work")));
        assert_eq!(
            opts.env,
            vec![("AWS_PROFILE".to_string(), "prod".to_string())]
        );
    }

    #[test]
    fn missing_folder_and_bad_names_are_refused() {
        let home = Path::new("/home/me");
        assert_eq!(
            spawn_options(&bash(), &launch("/nope", &[], &[]), home, |_| false).err(),
            Some(LaunchError::NoFolder("/nope".into()))
        );
        assert_eq!(
            spawn_options(&bash(), &launch("", &[("A-B", "x")], &[]), home, |_| true).err(),
            Some(LaunchError::BadName("A-B".into()))
        );
    }

    #[test]
    fn wsl_profile_passes_folder_and_env_into_linux() {
        let opts = spawn_options(
            &ubuntu(),
            &launch(
                "~/src",
                &[("KUBECONFIG", "/home/me/.kube/lab")],
                &["-c", "htop"],
            ),
            Path::new(r"C:\Users\me"),
            |_| false,
        )
        .expect("ok");
        assert_eq!(
            opts.args,
            vec!["-d", "Ubuntu", "--cd", "~/src", "--", "-c", "htop"]
        );
        assert_eq!(opts.cwd, None);
        assert!(opts
            .env
            .contains(&("KUBECONFIG".into(), "/home/me/.kube/lab".into())));
        let wslenv = opts
            .env
            .iter()
            .find(|(k, _)| k == "WSLENV")
            .map(|(_, v)| v.clone());
        assert!(wslenv.expect("WSLENV").ends_with("KUBECONFIG/u"));
    }
}
