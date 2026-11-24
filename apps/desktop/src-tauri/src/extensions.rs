//! The settings page a build with extensions may add (see
//! `docs/extending.md`). The web view only draws the plain data it gets
//! here; button presses come back by id.

use serde::Serialize;
use setu_core::extend::{ActionOutcome, Extensions, PanelSection};
use std::io::Write;
use std::path::{Path, PathBuf};
use tauri::State;

/// Saved exports are text; anything bigger is a bug.
const MAX_SAVE: usize = 32 * 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelView {
    title: String,
    sections: Vec<PanelSection>,
}

/// The build's settings page, or `None` in the open-source build.
#[tauri::command]
pub fn extension_panel(extensions: State<'_, Extensions>) -> Option<PanelView> {
    let panel = extensions.panel()?;
    Some(PanelView {
        title: panel.title(),
        sections: panel.sections(),
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResult {
    message: String,
}

/// Presses button `id` of the build's settings page.
#[tauri::command]
pub async fn extension_action(
    app: tauri::AppHandle,
    extensions: State<'_, Extensions>,
    id: String,
) -> Result<ActionResult, String> {
    let panel = extensions
        .panel()
        .cloned()
        .ok_or("this build has no settings page of its own")?;
    if id.is_empty() || id.len() > 64 {
        return Err("unknown action".into());
    }
    // Actions may read or verify files: keep them off the main thread.
    let outcome = tauri::async_runtime::spawn_blocking(move || panel.action(&id))
        .await
        .map_err(|_| "the action stopped unexpectedly".to_string())??;
    match outcome {
        ActionOutcome::Message(message) => Ok(ActionResult { message }),
        ActionOutcome::Save {
            file_name,
            contents,
        } => {
            let path = save_new(&downloads_dir(), &file_name, contents.as_bytes())?;
            Ok(ActionResult {
                message: format!("Saved to {}", path.display()),
            })
        }
        ActionOutcome::OpenUrl { url, message } => {
            let url =
                browser_url(&url).ok_or("the page asked to open an address Setu does not open")?;
            use tauri_plugin_opener::OpenerExt;
            app.opener()
                .open_url(url, None::<&str>)
                .map_err(|_| "could not open your web browser".to_string())?;
            Ok(ActionResult { message })
        }
    }
}

/// An address an extension may open in the browser: `https://`, or
/// `http://` to this computer. No other schemes (no `file:`, no custom
/// handlers), no user name or password, no control characters, at most
/// 2048 bytes. Returns it unchanged when it passes.
fn browser_url(url: &str) -> Option<&str> {
    if url.len() > 2048 || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://").filter(|r| loopback_host(r)))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') || authority.contains('\\') {
        return None;
    }
    Some(url)
}

fn loopback_host(rest: &str) -> bool {
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = match authority.strip_prefix("[::1]") {
        Some(port) => return port.is_empty() || port.starts_with(':'),
        None => authority.rsplit_once(':').map_or(authority, |(h, _)| h),
    };
    host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost")
}

fn downloads_dir() -> PathBuf {
    let home = crate::files::home_dir();
    let downloads = home.join("Downloads");
    if downloads.is_dir() {
        downloads
    } else {
        home
    }
}

/// A file name an extension asked for, made safe: one path part, plain
/// characters, not hidden, at most 100 characters.
fn safe_name(name: &str) -> Option<String> {
    let cleaned: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .take(100)
        .collect();
    let cleaned = cleaned.trim_start_matches('.').to_string();
    (!cleaned.is_empty() && cleaned != "." && cleaned != "..").then_some(cleaned)
}

/// Writes `bytes` to a new file in `dir`, never over an existing one:
/// `name`, then `name-1`, `name-2`, … File mode 0600.
fn save_new(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    if bytes.len() > MAX_SAVE {
        return Err("that export is too large to save".into());
    }
    let name = safe_name(name).ok_or("the export has no usable file name")?;
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    for n in 0..100 {
        let candidate = if n == 0 {
            dir.join(&name)
        } else {
            dir.join(format!("{stem}-{n}{ext}"))
        };
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        match opts.open(&candidate) {
            Ok(mut f) => {
                f.write_all(bytes)
                    .and_then(|_| f.sync_all())
                    .map_err(|e| format!("could not save {}: {e}", candidate.display()))?;
                return Ok(candidate);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("could not save {}: {e}", candidate.display())),
        }
    }
    Err("too many files with that name already".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_and_loopback_addresses_open() {
        for ok in [
            "https://setu.example.com/sso/start?login=abc",
            "http://127.0.0.1:7440/sso/start",
            "http://localhost:7440/x",
            "http://[::1]:7440/",
        ] {
            assert_eq!(browser_url(ok), Some(ok), "{ok}");
        }
        for bad in [
            "file:///etc/passwd",
            "http://setu.example.com/",
            "http://127.0.0.1.evil.example/",
            "http://localhost.evil.example:80/",
            "https://user:pw@setu.example.com/",
            "https://",
            "javascript:alert(1)",
            "smb://share/x",
            "https://a.example/\nx",
            "https://a.example/ space",
            "HTTPS://a.example/",
        ] {
            assert_eq!(browser_url(bad), None, "{bad}");
        }
        assert_eq!(
            browser_url(&format!("https://a.example/{}", "x".repeat(2100))),
            None
        );
    }

    #[test]
    fn names_stay_in_the_folder() {
        assert_eq!(
            safe_name("audit-2026.jsonl").as_deref(),
            Some("audit-2026.jsonl")
        );
        assert_eq!(safe_name("../../.bashrc").as_deref(), Some("bashrc"));
        assert_eq!(safe_name("/etc/passwd").as_deref(), Some("etcpasswd"));
        assert_eq!(safe_name("..").as_deref(), None);
        assert_eq!(safe_name("").as_deref(), None);
    }

    #[test]
    fn saving_never_overwrites() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = save_new(dir.path(), "audit.log", b"one").expect("save");
        let b = save_new(dir.path(), "audit.log", b"two").expect("save");
        assert_eq!(a, dir.path().join("audit.log"));
        assert_eq!(b, dir.path().join("audit-1.log"));
        assert_eq!(std::fs::read(&a).expect("read"), b"one");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&b).expect("meta").permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
