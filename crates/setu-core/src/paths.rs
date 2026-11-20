//! Where Setu keeps its files on each operating system.
//!
//! Setu stores everything as plain files so users can back them up or keep
//! them in Git. The layout follows each OS's conventions:
//!
//! | OS      | Config directory                         |
//! |---------|------------------------------------------|
//! | Linux   | `$XDG_CONFIG_HOME/setu` or `~/.config/setu` |
//! | macOS   | `~/Library/Application Support/setu`     |
//! | Windows | `%APPDATA%\setu`                         |
//!
//! Portable mode (`SETU_HOME` set) overrides all of them, so Setu can run from
//! a folder without being installed.

use std::path::PathBuf;

/// Environment variable that, when set, puts every Setu file under one folder.
pub const SETU_HOME_ENV: &str = "SETU_HOME";

const APP_DIR: &str = "setu";

/// Errors when resolving Setu's directories.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PathError {
    /// No home directory could be determined from the environment.
    #[error("could not determine the home directory; set {SETU_HOME_ENV} to choose a folder")]
    NoHome,
}

/// A source of environment values, so path resolution is testable without
/// touching the real process environment.
pub trait Env {
    /// Returns the value of `key`, or `None` if it is unset or empty.
    fn var(&self, key: &str) -> Option<String>;
}

/// The real process environment.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessEnv;

impl Env for ProcessEnv {
    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|v| !v.is_empty())
    }
}

/// The operating system families Setu supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
}

impl Os {
    /// The OS this binary was compiled for.
    pub const fn current() -> Self {
        if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Linux
        }
    }
}

/// Resolves the directory holding Setu's configuration (hosts, settings).
pub fn config_dir(env: &impl Env, os: Os) -> Result<PathBuf, PathError> {
    if let Some(home) = env.var(SETU_HOME_ENV) {
        return Ok(PathBuf::from(home).join("config"));
    }
    match os {
        Os::Linux => {
            if let Some(xdg) = env.var("XDG_CONFIG_HOME") {
                return Ok(PathBuf::from(xdg).join(APP_DIR));
            }
            let home = env.var("HOME").ok_or(PathError::NoHome)?;
            Ok(PathBuf::from(home).join(".config").join(APP_DIR))
        }
        Os::MacOs => {
            let home = env.var("HOME").ok_or(PathError::NoHome)?;
            Ok(PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join(APP_DIR))
        }
        Os::Windows => {
            let appdata = env.var("APPDATA").ok_or(PathError::NoHome)?;
            Ok(PathBuf::from(appdata).join(APP_DIR))
        }
    }
}

/// Resolves the user's SSH directory (`~/.ssh`), used for config and key import.
pub fn ssh_dir(env: &impl Env, os: Os) -> Result<PathBuf, PathError> {
    let home = match os {
        Os::Windows => env.var("USERPROFILE").or_else(|| env.var("HOME")),
        Os::Linux | Os::MacOs => env.var("HOME"),
    }
    .ok_or(PathError::NoHome)?;
    Ok(PathBuf::from(home).join(".ssh"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct FakeEnv(HashMap<&'static str, &'static str>);

    impl Env for FakeEnv {
        fn var(&self, key: &str) -> Option<String> {
            self.0
                .get(key)
                .map(|v| v.to_string())
                .filter(|v| !v.is_empty())
        }
    }

    fn env(pairs: &[(&'static str, &'static str)]) -> FakeEnv {
        FakeEnv(pairs.iter().copied().collect())
    }

    #[test]
    fn portable_mode_wins_everywhere() {
        let e = env(&[("SETU_HOME", "/opt/setu"), ("HOME", "/home/a")]);
        for os in [Os::Linux, Os::MacOs, Os::Windows] {
            assert_eq!(
                config_dir(&e, os),
                Ok(PathBuf::from("/opt/setu").join("config"))
            );
        }
    }

    #[test]
    fn linux_prefers_xdg() {
        let e = env(&[("XDG_CONFIG_HOME", "/x"), ("HOME", "/home/a")]);
        assert_eq!(config_dir(&e, Os::Linux), Ok(PathBuf::from("/x/setu")));
    }

    #[test]
    fn linux_falls_back_to_dot_config() {
        let e = env(&[("HOME", "/home/a")]);
        assert_eq!(
            config_dir(&e, Os::Linux),
            Ok(PathBuf::from("/home/a/.config/setu"))
        );
    }

    #[test]
    fn empty_values_count_as_unset() {
        let e = env(&[("XDG_CONFIG_HOME", ""), ("HOME", "/home/a")]);
        assert_eq!(
            config_dir(&e, Os::Linux),
            Ok(PathBuf::from("/home/a/.config/setu"))
        );
    }

    #[test]
    fn macos_uses_application_support() {
        let e = env(&[("HOME", "/Users/a")]);
        assert_eq!(
            config_dir(&e, Os::MacOs),
            Ok(PathBuf::from("/Users/a/Library/Application Support/setu"))
        );
    }

    #[test]
    fn windows_uses_appdata() {
        let e = env(&[("APPDATA", "C:/Users/a/AppData/Roaming")]);
        assert_eq!(
            config_dir(&e, Os::Windows),
            Ok(PathBuf::from("C:/Users/a/AppData/Roaming").join("setu"))
        );
    }

    #[test]
    fn missing_home_is_an_error() {
        let e = env(&[]);
        assert_eq!(config_dir(&e, Os::Linux), Err(PathError::NoHome));
        assert_eq!(ssh_dir(&e, Os::MacOs), Err(PathError::NoHome));
    }

    #[test]
    fn ssh_dir_uses_userprofile_on_windows() {
        let e = env(&[("USERPROFILE", "C:/Users/a")]);
        assert_eq!(
            ssh_dir(&e, Os::Windows),
            Ok(PathBuf::from("C:/Users/a").join(".ssh"))
        );
    }
}
