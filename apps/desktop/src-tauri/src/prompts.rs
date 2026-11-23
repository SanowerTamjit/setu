//! Which third-party prompt themes (Powerlevel10k, Oh My Zsh, Starship, Oh My
//! Posh) this computer's shells already use, so Settings can say what is set
//! up and what is missing.
//!
//! It only looks: it checks a few files in the home folder, reads the shell
//! start-up files (capped) for the lines these tools add, and looks for the
//! programs on `PATH`. Nothing is run, installed or changed.

use serde::Serialize;
use std::path::{Path, PathBuf};

/// Start-up files bigger than this are not read; real ones are a few KB.
const MAX_RC: u64 = 512 * 1024;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PromptTool {
    /// `powerlevel10k`, `ohmyzsh`, `starship` or `ohmyposh`.
    pub id: &'static str,
    /// Installed: its files or program were found.
    pub installed: bool,
    /// A shell start-up file loads it.
    pub enabled: bool,
    /// Where it was found (a file or the start-up file that loads it), for the UI.
    pub detail: Option<String>,
}

#[tauri::command]
pub fn prompt_detect() -> Vec<PromptTool> {
    match home() {
        Some(home) => detect(&home, std::env::var_os("PATH").as_deref()),
        None => Vec::new(),
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

/// Reads a start-up file, or nothing if it is missing, not a file or too big.
fn read_rc(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_RC {
        return None;
    }
    std::fs::read(path)
        .ok()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// Lines that are not comments, so a commented-out `eval` does not count.
fn active_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines().map(str::trim).filter(|l| !l.starts_with('#'))
}

fn on_path(path_var: Option<&std::ffi::OsStr>, program: &str) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ""]
    } else {
        &[""]
    };
    std::env::split_paths(path_var?).find_map(|dir| {
        exts.iter()
            .map(|e| dir.join(format!("{program}{e}")))
            .find(|p| p.is_file())
    })
}

fn tilde(home: &Path, p: &Path) -> String {
    match p.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

pub fn detect(home: &Path, path_var: Option<&std::ffi::OsStr>) -> Vec<PromptTool> {
    let rc_names = [
        ".zshrc",
        ".bashrc",
        ".bash_profile",
        ".config/fish/config.fish",
        "Documents/PowerShell/Microsoft.PowerShell_profile.ps1",
        ".config/powershell/Microsoft.PowerShell_profile.ps1",
    ];
    let rcs: Vec<(PathBuf, String)> = rc_names
        .iter()
        .map(|n| home.join(n))
        .filter_map(|p| read_rc(&p).map(|t| (p, t)))
        .collect();
    // The first start-up file with an active line matching `test`.
    let loaded_by = |test: &dyn Fn(&str) -> bool| -> Option<String> {
        rcs.iter()
            .find(|(_, text)| active_lines(text).any(test))
            .map(|(p, _)| tilde(home, p))
    };

    let mut out = Vec::new();

    // Powerlevel10k: its config file, a clone in the usual places, or a
    // theme line in .zshrc.
    let p10k_dirs = [
        ".oh-my-zsh/custom/themes/powerlevel10k",
        "powerlevel10k",
        ".powerlevel10k",
        ".zsh/powerlevel10k",
        ".local/share/powerlevel10k",
    ];
    let p10k_found = p10k_dirs.iter().map(|d| home.join(d)).find(|p| p.is_dir());
    let p10k_loaded = loaded_by(&|l| l.contains("powerlevel10k"));
    let p10k_config = home.join(".p10k.zsh");
    out.push(PromptTool {
        id: "powerlevel10k",
        installed: p10k_found.is_some() || p10k_config.is_file(),
        enabled: p10k_loaded.is_some(),
        detail: p10k_loaded.or_else(|| p10k_found.map(|p| tilde(home, &p))),
    });

    let omz = home.join(".oh-my-zsh");
    let omz_loaded = loaded_by(&|l| l.contains("oh-my-zsh.sh"));
    out.push(PromptTool {
        id: "ohmyzsh",
        installed: omz.join("oh-my-zsh.sh").is_file(),
        enabled: omz_loaded.is_some(),
        detail: omz_loaded.or_else(|| omz.is_dir().then(|| tilde(home, &omz))),
    });

    let starship = on_path(path_var, "starship");
    let starship_loaded = loaded_by(&|l| l.contains("starship init"));
    out.push(PromptTool {
        id: "starship",
        installed: starship.is_some(),
        enabled: starship_loaded.is_some(),
        detail: starship_loaded.or_else(|| starship.map(|p| p.display().to_string())),
    });

    let posh = on_path(path_var, "oh-my-posh");
    let posh_loaded =
        loaded_by(&|l| l.contains("oh-my-posh init") || l.contains("oh-my-posh --init"));
    out.push(PromptTool {
        id: "ohmyposh",
        installed: posh.is_some(),
        enabled: posh_loaded.is_some(),
        detail: posh_loaded.or_else(|| posh.map(|p| p.display().to_string())),
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool<'a>(tools: &'a [PromptTool], id: &str) -> &'a PromptTool {
        tools.iter().find(|t| t.id == id).expect("listed")
    }

    #[test]
    fn nothing_set_up() {
        let home = tempfile::tempdir().expect("tempdir");
        let tools = detect(home.path(), None);
        assert_eq!(tools.len(), 4);
        assert!(tools
            .iter()
            .all(|t| !t.installed && !t.enabled && t.detail.is_none()));
    }

    #[test]
    fn finds_p10k_on_oh_my_zsh_and_starship_in_bash() {
        let home = tempfile::tempdir().expect("tempdir");
        let h = home.path();
        std::fs::create_dir_all(h.join(".oh-my-zsh/custom/themes/powerlevel10k")).expect("mkdir");
        std::fs::write(h.join(".oh-my-zsh/oh-my-zsh.sh"), "").expect("write");
        std::fs::write(
            h.join(".zshrc"),
            "ZSH_THEME=\"powerlevel10k/powerlevel10k\"\nsource $ZSH/oh-my-zsh.sh\n",
        )
        .expect("write");
        // A commented-out line does not count.
        std::fs::write(
            h.join(".bashrc"),
            "# eval \"$(oh-my-posh init bash)\"\neval \"$(starship init bash)\"\n",
        )
        .expect("write");
        let bin = h.join("bin");
        std::fs::create_dir_all(&bin).expect("mkdir");
        std::fs::write(bin.join("starship"), "").expect("write");
        let path = std::env::join_paths([&bin]).expect("path");

        let tools = detect(h, Some(&path));
        let p10k = tool(&tools, "powerlevel10k");
        assert!(p10k.installed && p10k.enabled);
        assert_eq!(p10k.detail.as_deref(), Some("~/.zshrc"));
        assert!(tool(&tools, "ohmyzsh").enabled);
        let starship = tool(&tools, "starship");
        assert!(starship.installed && starship.enabled);
        assert_eq!(starship.detail.as_deref(), Some("~/.bashrc"));
        let posh = tool(&tools, "ohmyposh");
        assert!(!posh.installed && !posh.enabled);
    }

    #[test]
    fn huge_startup_files_are_not_read() {
        let home = tempfile::tempdir().expect("tempdir");
        let mut big = "x\n".repeat((MAX_RC as usize) / 2 + 10);
        big.push_str("source ~/powerlevel10k/powerlevel10k.zsh-theme\n");
        std::fs::write(home.path().join(".zshrc"), big).expect("write");
        assert!(!tool(&detect(home.path(), None), "powerlevel10k").enabled);
    }
}
