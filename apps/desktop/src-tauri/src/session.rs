//! Small JSON files the UI owns, in Setu's config folder:
//!
//! - `session.json`: the open tabs and panes, so the next start can reopen
//!   them (see `session.ts`). Host names, users, ports and key paths; never
//!   passwords or passphrases.
//! - `settings.json`: preferences such as the theme and keybindings (see
//!   `settings.ts`).
//!
//! Both are written atomically and readable only by you.

use setu_core::paths::{self, Os, ProcessEnv};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

pub const SESSION_FILE: &str = "session.json";
pub const SETTINGS_FILE: &str = "settings.json";

/// Larger files are not a session Setu wrote; ignore them.
const MAX_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub struct JsonFile {
    path: Result<PathBuf, String>,
    lock: Mutex<()>,
}

/// `session.json`.
#[derive(Debug)]
pub struct SessionFile(pub JsonFile);

/// `settings.json`.
#[derive(Debug)]
pub struct SettingsFile(pub JsonFile);

impl SessionFile {
    pub fn from_env() -> Self {
        SessionFile(JsonFile::in_config(SESSION_FILE))
    }
}

impl SettingsFile {
    pub fn from_env() -> Self {
        SettingsFile(JsonFile::in_config(SETTINGS_FILE))
    }
}

impl JsonFile {
    pub fn in_config(name: &str) -> Self {
        let path = paths::config_dir(&ProcessEnv, Os::current())
            .map(|dir| dir.join(name))
            .map_err(|e| e.to_string());
        JsonFile {
            path,
            lock: Mutex::new(()),
        }
    }

    #[cfg(test)]
    fn at(path: PathBuf) -> Self {
        JsonFile {
            path: Ok(path),
            lock: Mutex::new(()),
        }
    }

    /// The saved JSON, or `None` when there is none or it is unusable.
    fn load(&self) -> Option<String> {
        let path = self.path.as_ref().ok()?;
        let _guard = self.lock.lock().ok()?;
        let text = std::fs::read_to_string(path).ok()?;
        if text.len() > MAX_BYTES || serde_json::from_str::<serde_json::Value>(&text).is_err() {
            return None;
        }
        Some(text)
    }

    fn save(&self, json: &str) -> Result<(), String> {
        if json.len() > MAX_BYTES {
            return Err("too large to save".into());
        }
        serde_json::from_str::<serde_json::Value>(json).map_err(|e| e.to_string())?;
        let path = self.path.as_ref().map_err(Clone::clone)?;
        let _guard = self.lock.lock().map_err(|_| "file lock poisoned")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let mut tmp_name = path.as_os_str().to_owned();
        tmp_name.push(".tmp");
        let tmp = PathBuf::from(tmp_name);
        let result = (|| {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&tmp)?;
            file.write_all(json.as_bytes())?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&tmp, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result.map_err(|e| format!("{}: {e}", path.display()))
    }

    fn clear(&self) -> Result<(), String> {
        let path = self.path.as_ref().map_err(Clone::clone)?;
        let _guard = self.lock.lock().map_err(|_| "file lock poisoned")?;
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }
}

#[tauri::command]
pub fn session_load(state: State<'_, SessionFile>) -> Option<String> {
    state.0.load()
}

#[tauri::command]
pub fn session_save(state: State<'_, SessionFile>, json: String) -> Result<(), String> {
    state.0.save(&json)
}

#[tauri::command]
pub fn session_clear(state: State<'_, SessionFile>) -> Result<(), String> {
    state.0.clear()
}

#[tauri::command]
pub fn settings_load(state: State<'_, SettingsFile>) -> Option<String> {
    state.0.load()
}

#[tauri::command]
pub fn settings_save(state: State<'_, SettingsFile>, json: String) -> Result<(), String> {
    state.0.save(&json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_loads_and_clears() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = JsonFile::at(dir.path().join("setu").join(SESSION_FILE));
        assert_eq!(file.load(), None);
        file.save(r#"{"version":1,"tabs":[]}"#).expect("save");
        assert_eq!(file.load().as_deref(), Some(r#"{"version":1,"tabs":[]}"#));
        file.clear().expect("clear");
        assert_eq!(file.load(), None);
        file.clear().expect("clearing twice is fine");
    }

    #[test]
    fn refuses_and_ignores_non_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(SESSION_FILE);
        let file = JsonFile::at(path.clone());
        assert!(file.save("not json").is_err());
        std::fs::write(&path, "{broken").expect("write");
        assert_eq!(file.load(), None);
    }

    #[cfg(unix)]
    #[test]
    fn file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(SESSION_FILE);
        JsonFile::at(path.clone()).save("{}").expect("save");
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
