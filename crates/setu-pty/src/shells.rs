//! Detects the shells available on this machine.
//!
//! Detection never runs a shell or asks the user. On Unix it reads `$SHELL`
//! and `/etc/shells` and checks which well-known shells exist on `PATH`. On
//! Windows it looks for PowerShell 7 (`pwsh`), Windows PowerShell, `cmd` and
//! the installed WSL distros.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The family a shell belongs to, used for icons and sensible defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShellKind {
    Bash,
    Zsh,
    Fish,
    Sh,
    /// PowerShell 7+ (`pwsh`).
    Pwsh,
    /// Windows PowerShell 5.x (`powershell.exe`).
    WindowsPowerShell,
    Cmd,
    /// A WSL distribution, launched through `wsl.exe -d <name>`.
    Wsl,
    Other,
}

impl ShellKind {
    /// Guesses the kind from a program path such as `/usr/bin/zsh`.
    pub fn from_program(program: &Path) -> Self {
        match program_stem(program).to_ascii_lowercase().as_str() {
            "bash" => ShellKind::Bash,
            "zsh" => ShellKind::Zsh,
            "fish" => ShellKind::Fish,
            "sh" | "dash" | "ash" => ShellKind::Sh,
            "pwsh" => ShellKind::Pwsh,
            "powershell" => ShellKind::WindowsPowerShell,
            "cmd" => ShellKind::Cmd,
            _ => ShellKind::Other,
        }
    }
}

/// A shell Setu can open in a terminal tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellProfile {
    /// Stable identifier, for example `zsh` or `wsl:Ubuntu`.
    pub id: String,
    /// Name shown in the UI.
    pub name: String,
    pub kind: ShellKind,
    /// Program to run.
    pub program: PathBuf,
    /// Arguments passed to the program.
    pub args: Vec<String>,
}

impl ShellProfile {
    fn from_program(program: PathBuf) -> Self {
        let kind = ShellKind::from_program(&program);
        let stem = program_stem(&program);
        let name = match kind {
            ShellKind::Pwsh => "PowerShell".to_string(),
            ShellKind::WindowsPowerShell => "Windows PowerShell".to_string(),
            ShellKind::Cmd => "Command Prompt".to_string(),
            _ => stem.clone(),
        };
        // Unix shells start as login shells so the user's profile is loaded,
        // matching what macOS Terminal and most terminal emulators do.
        let args = match kind {
            ShellKind::Bash | ShellKind::Zsh | ShellKind::Fish | ShellKind::Sh => {
                vec!["-l".to_string()]
            }
            ShellKind::Pwsh | ShellKind::WindowsPowerShell => vec!["-NoLogo".to_string()],
            _ => Vec::new(),
        };
        ShellProfile {
            id: stem.to_ascii_lowercase(),
            name,
            kind,
            program,
            args,
        }
    }

    fn wsl(wsl_exe: &Path, distro: &str) -> Self {
        ShellProfile {
            id: format!("wsl:{distro}"),
            name: format!("{distro} (WSL)"),
            kind: ShellKind::Wsl,
            program: wsl_exe.to_path_buf(),
            args: vec!["-d".to_string(), distro.to_string()],
        }
    }
}

/// The file name without extension, treating both `/` and `\\` as
/// separators so Windows paths are understood on every host (and in tests).
fn program_stem(program: &Path) -> String {
    let full = program.to_string_lossy();
    let file = full.rsplit(['/', '\\']).next().unwrap_or_default();
    let stem = match file.rfind('.') {
        Some(i) if i > 0 => &file[..i],
        _ => file,
    };
    if stem.is_empty() {
        "shell".to_string()
    } else {
        stem.to_string()
    }
}

/// Joins path parts with the separator of the target OS, not the host's.
fn join(base: &str, parts: &[&str], windows: bool) -> PathBuf {
    let sep = if windows { '\\' } else { '/' };
    let mut out = base.trim_end_matches(['/', '\\']).to_string();
    for part in parts {
        out.push(sep);
        out.push_str(part);
    }
    PathBuf::from(out)
}

/// What detection needs from the system, so it can be tested with fakes.
pub trait System {
    /// An environment variable, `None` if unset or empty.
    fn env(&self, key: &str) -> Option<String>;
    /// Whether `path` is an existing file.
    fn is_file(&self, path: &Path) -> bool;
    /// Contents of a text file, `None` if it cannot be read.
    fn read_to_string(&self, path: &Path) -> Option<String>;
    /// Runs `program args` and returns its raw stdout, `None` on failure.
    fn command_output(&self, program: &Path, args: &[&str]) -> Option<Vec<u8>>;
}

/// The real system.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealSystem;

impl System for RealSystem {
    fn env(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|v| !v.is_empty())
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn read_to_string(&self, path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    fn command_output(&self, program: &Path, args: &[&str]) -> Option<Vec<u8>> {
        let mut cmd = std::process::Command::new(program);
        cmd.args(args).stdin(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let out = cmd.output().ok()?;
        out.status.success().then_some(out.stdout)
    }
}

/// Finds the shells on this machine. The first entry is the default shell.
pub fn detect_shells() -> Vec<ShellProfile> {
    if cfg!(windows) {
        detect_windows(&RealSystem)
    } else {
        detect_unix(&RealSystem)
    }
}

/// The shell to open on first launch.
pub fn default_shell() -> Option<ShellProfile> {
    detect_shells().into_iter().next()
}

/// Searches `PATH` for an executable named `name` (with `.exe` on Windows).
fn find_on_path(sys: &impl System, name: &str, windows: bool) -> Option<PathBuf> {
    let path = sys.env("PATH")?;
    let sep = if windows { ';' } else { ':' };
    let file = if windows {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    path.split(sep)
        .filter(|dir| !dir.is_empty())
        .map(|dir| join(dir, &[&file], windows))
        .find(|candidate| sys.is_file(candidate))
}

/// Unix detection: `$SHELL` first, then `/etc/shells`, then well-known names on `PATH`.
pub fn detect_unix(sys: &impl System) -> Vec<ShellProfile> {
    let mut found: Vec<ShellProfile> = Vec::new();
    let push = |program: PathBuf, found: &mut Vec<ShellProfile>| {
        let profile = ShellProfile::from_program(program);
        // One entry per shell name: /bin/bash and /usr/bin/bash are the same to a user.
        if !found.iter().any(|p| p.id == profile.id) {
            found.push(profile);
        }
    };

    if let Some(shell) = sys.env("SHELL") {
        let path = PathBuf::from(shell);
        if sys.is_file(&path) {
            push(path, &mut found);
        }
    }

    if let Some(list) = sys.read_to_string(Path::new("/etc/shells")) {
        for line in list.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let path = PathBuf::from(line);
            let kind = ShellKind::from_program(&path);
            // Skip entries like /usr/bin/false, git-shell and screen.
            if kind != ShellKind::Other && sys.is_file(&path) {
                push(path, &mut found);
            }
        }
    }

    for name in ["zsh", "bash", "fish", "sh"] {
        if let Some(path) = find_on_path(sys, name, false) {
            push(path, &mut found);
        }
    }

    if found.is_empty() && sys.is_file(Path::new("/bin/sh")) {
        push(PathBuf::from("/bin/sh"), &mut found);
    }
    found
}

/// Windows detection: PowerShell 7, Windows PowerShell, cmd, then WSL distros.
pub fn detect_windows(sys: &impl System) -> Vec<ShellProfile> {
    let mut found = Vec::new();
    let system_root = sys
        .env("SystemRoot")
        .unwrap_or_else(|| r"C:\Windows".to_string());
    let system32 = join(&system_root, &["System32"], true);
    let system32 = system32.to_string_lossy();

    if let Some(pwsh) = find_on_path(sys, "pwsh", true) {
        found.push(ShellProfile::from_program(pwsh));
    }

    let winps = join(
        &system32,
        &["WindowsPowerShell", "v1.0", "powershell.exe"],
        true,
    );
    if sys.is_file(&winps) {
        found.push(ShellProfile::from_program(winps));
    }

    let cmd = sys
        .env("ComSpec")
        .map(PathBuf::from)
        .unwrap_or_else(|| join(&system32, &["cmd.exe"], true));
    if sys.is_file(&cmd) {
        found.push(ShellProfile::from_program(cmd));
    }

    let wsl = join(&system32, &["wsl.exe"], true);
    if sys.is_file(&wsl) {
        if let Some(out) = sys.command_output(&wsl, &["--list", "--quiet"]) {
            for distro in parse_wsl_list(&out) {
                found.push(ShellProfile::wsl(&wsl, &distro));
            }
        }
    }
    found
}

/// Parses `wsl.exe --list --quiet` output.
///
/// `wsl.exe` writes UTF-16LE (often with a BOM) unless `WSL_UTF8=1` is set,
/// so both encodings are handled. Docker Desktop's internal distros are hidden.
pub fn parse_wsl_list(raw: &[u8]) -> Vec<String> {
    let looks_utf16 = raw.len() >= 2 && (raw.starts_with(&[0xFF, 0xFE]) || raw[1] == 0);
    let text = if looks_utf16 {
        let units: Vec<u16> = raw
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(raw).into_owned()
    };
    text.lines()
        .map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}' || c == '\0'))
        .filter(|l| !l.is_empty() && !l.starts_with("docker-desktop"))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    #[derive(Default)]
    struct FakeSystem {
        env: HashMap<String, String>,
        files: HashSet<PathBuf>,
        texts: HashMap<PathBuf, String>,
        outputs: HashMap<PathBuf, Vec<u8>>,
    }

    impl FakeSystem {
        fn env(mut self, k: &str, v: &str) -> Self {
            self.env.insert(k.into(), v.into());
            self
        }
        fn file(mut self, p: impl Into<PathBuf>) -> Self {
            self.files.insert(p.into());
            self
        }
        fn text(mut self, p: &str, t: &str) -> Self {
            self.texts.insert(p.into(), t.into());
            self
        }
        fn output(mut self, p: impl Into<PathBuf>, o: Vec<u8>) -> Self {
            self.outputs.insert(p.into(), o);
            self
        }
    }

    impl System for FakeSystem {
        fn env(&self, key: &str) -> Option<String> {
            self.env.get(key).cloned().filter(|v| !v.is_empty())
        }
        fn is_file(&self, path: &Path) -> bool {
            self.files.contains(path)
        }
        fn read_to_string(&self, path: &Path) -> Option<String> {
            self.texts.get(path).cloned()
        }
        fn command_output(&self, program: &Path, _args: &[&str]) -> Option<Vec<u8>> {
            self.outputs.get(program).cloned()
        }
    }

    fn ids(shells: &[ShellProfile]) -> Vec<&str> {
        shells.iter().map(|s| s.id.as_str()).collect()
    }

    fn utf16le(s: &str, bom: bool) -> Vec<u8> {
        let mut out = if bom { vec![0xFF, 0xFE] } else { Vec::new() };
        for unit in s.encode_utf16() {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out
    }

    #[test]
    fn kind_from_program() {
        assert_eq!(
            ShellKind::from_program(Path::new("/bin/zsh")),
            ShellKind::Zsh
        );
        assert_eq!(
            ShellKind::from_program(Path::new("/usr/bin/dash")),
            ShellKind::Sh
        );
        assert_eq!(
            ShellKind::from_program(Path::new(r"C:\Program Files\PowerShell\7\pwsh.exe")),
            ShellKind::Pwsh
        );
        assert_eq!(
            ShellKind::from_program(Path::new("/usr/bin/git-shell")),
            ShellKind::Other
        );
    }

    #[test]
    fn unix_puts_login_shell_first_and_dedupes() {
        let sys = FakeSystem::default()
            .env("SHELL", "/usr/bin/zsh")
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .file("/usr/bin/zsh")
            .file("/bin/bash")
            .file("/usr/bin/bash")
            .file("/usr/bin/false")
            .file("/bin/sh")
            .text(
                "/etc/shells",
                "# comment\n/bin/sh\n/bin/bash\n/usr/bin/bash\n/usr/bin/zsh\n/usr/bin/false\n",
            );
        let shells = detect_unix(&sys);
        assert_eq!(ids(&shells), ["zsh", "sh", "bash"]);
        assert_eq!(shells[0].args, ["-l"]);
        assert_eq!(shells[0].kind, ShellKind::Zsh);
    }

    #[test]
    fn unix_finds_shells_on_path_without_etc_shells() {
        let sys = FakeSystem::default()
            .env("PATH", "/opt/homebrew/bin:/usr/bin")
            .file("/opt/homebrew/bin/fish")
            .file("/usr/bin/bash");
        let shells = detect_unix(&sys);
        assert_eq!(ids(&shells), ["bash", "fish"]);
        assert_eq!(shells[1].program, PathBuf::from("/opt/homebrew/bin/fish"));
    }

    #[test]
    fn unix_ignores_missing_login_shell() {
        let sys = FakeSystem::default()
            .env("SHELL", "/usr/bin/zsh")
            .file("/bin/sh");
        assert_eq!(ids(&detect_unix(&sys)), ["sh"]);
    }

    #[test]
    fn windows_order_and_wsl() {
        let sys = FakeSystem::default()
            .env("SystemRoot", r"C:\Windows")
            .env("ComSpec", r"C:\Windows\System32\cmd.exe")
            .env("PATH", r"C:\Windows\System32;C:\Program Files\PowerShell\7")
            .file(r"C:\Program Files\PowerShell\7\pwsh.exe")
            .file(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe")
            .file(r"C:\Windows\System32\cmd.exe")
            .file(r"C:\Windows\System32\wsl.exe")
            .output(
                r"C:\Windows\System32\wsl.exe",
                utf16le("Ubuntu-24.04\r\ndocker-desktop\r\nDebian\r\n", true),
            );
        let shells = detect_windows(&sys);
        assert_eq!(
            ids(&shells),
            [
                "pwsh",
                "powershell",
                "cmd",
                "wsl:Ubuntu-24.04",
                "wsl:Debian"
            ]
        );
        assert_eq!(shells[0].name, "PowerShell");
        assert_eq!(shells[3].args, ["-d", "Ubuntu-24.04"]);
        assert_eq!(shells[3].kind, ShellKind::Wsl);
    }

    #[test]
    fn windows_without_wsl() {
        let sys = FakeSystem::default()
            .env("SystemRoot", r"C:\Windows")
            .file(r"C:\Windows\System32\cmd.exe");
        assert_eq!(ids(&detect_windows(&sys)), ["cmd"]);
    }

    #[test]
    fn wsl_list_parsing_handles_encodings() {
        let expected = ["Ubuntu", "kali-linux"];
        assert_eq!(
            parse_wsl_list(&utf16le("Ubuntu\r\nkali-linux\r\n", true)),
            expected
        );
        assert_eq!(
            parse_wsl_list(&utf16le("Ubuntu\r\nkali-linux\r\n", false)),
            expected
        );
        assert_eq!(parse_wsl_list(b"Ubuntu\nkali-linux\n"), expected);
        assert!(parse_wsl_list(b"").is_empty());
    }
}
