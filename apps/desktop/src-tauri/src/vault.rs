//! The encrypted vault in the desktop app: create, unlock, lock, auto-lock,
//! and lookups for SSH connections.
//!
//! Secrets never cross to the web view. The UI sends a password or passphrase
//! once (to connect or to save it) and only ever gets back the list of
//! targets, never a stored secret.

use serde::Serialize;
use setu_core::paths::{self, Os, ProcessEnv};
use setu_vault::{EntryInfo, KdfParams, SecretKind, Vault, VaultError, Zeroizing, VAULT_FILE};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

/// Lock after this long without the vault being used.
pub const AUTO_LOCK: Duration = Duration::from_secs(15 * 60);

/// Event sent to the UI when the vault locks by itself.
pub const STATUS_EVENT: &str = "vault-status";

#[derive(Debug)]
struct Inner {
    vault: Option<Vault>,
    last_used: Instant,
}

#[derive(Debug, Clone)]
pub struct VaultState {
    path: Result<PathBuf, String>,
    inner: Arc<Mutex<Inner>>,
}

/// Whether a stored secret could be looked up, for the UI's prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Availability {
    /// No vault yet.
    None,
    Locked,
    Unlocked,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    exists: bool,
    unlocked: bool,
    path: String,
    entries: Vec<EntryInfo>,
    auto_lock_minutes: u64,
}

impl VaultState {
    pub fn from_env() -> Self {
        VaultState {
            path: paths::config_dir(&ProcessEnv, Os::current())
                .map(|dir| dir.join(VAULT_FILE))
                .map_err(|e| e.to_string()),
            inner: Arc::new(Mutex::new(Inner {
                vault: None,
                last_used: Instant::now(),
            })),
        }
    }

    fn path(&self) -> Result<PathBuf, String> {
        self.path.clone()
    }

    fn lock_inner(&self) -> MutexGuard<'_, Inner> {
        // A panic while holding the lock cannot leave the vault half-written
        // (saves are atomic), so recover the guard rather than fail forever.
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn status(&self) -> Status {
        let path = self.path.clone().unwrap_or_default();
        let inner = self.lock_inner();
        Status {
            exists: Vault::exists(&path),
            unlocked: inner.vault.is_some(),
            path: path.display().to_string(),
            entries: inner.vault.as_ref().map(Vault::entries).unwrap_or_default(),
            auto_lock_minutes: AUTO_LOCK.as_secs() / 60,
        }
    }

    pub fn availability(&self) -> Availability {
        let inner = self.lock_inner();
        if inner.vault.is_some() {
            Availability::Unlocked
        } else if self
            .path
            .as_ref()
            .map(|p| Vault::exists(p))
            .unwrap_or(false)
        {
            Availability::Locked
        } else {
            Availability::None
        }
    }

    /// A stored secret, if the vault is unlocked and has one. Counts as use.
    pub fn get(&self, kind: SecretKind, target: &str) -> Option<Zeroizing<String>> {
        let mut inner = self.lock_inner();
        let secret = inner.vault.as_ref()?.get(kind, target);
        if secret.is_some() {
            inner.last_used = Instant::now();
        }
        secret
    }

    /// Saves a secret if the vault is unlocked. Returns whether it was saved.
    pub fn save(
        &self,
        kind: SecretKind,
        target: &str,
        secret: Zeroizing<String>,
    ) -> Result<bool, String> {
        let mut inner = self.lock_inner();
        let Some(vault) = inner.vault.as_mut() else {
            return Ok(false);
        };
        vault.set(kind, target, secret).map_err(|e| e.to_string())?;
        inner.last_used = Instant::now();
        Ok(true)
    }

    fn set_unlocked(&self, vault: Vault) {
        let mut inner = self.lock_inner();
        inner.vault = Some(vault);
        inner.last_used = Instant::now();
    }

    fn lock(&self) {
        // Dropping the vault zeroizes its key and secrets.
        self.lock_inner().vault = None;
    }

    /// Locks the vault after [`AUTO_LOCK`] of inactivity and tells the UI.
    pub fn spawn_auto_lock(&self, app: AppHandle) {
        let state = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(20)).await;
                let expired = {
                    let inner = state.lock_inner();
                    inner.vault.is_some() && inner.last_used.elapsed() >= AUTO_LOCK
                };
                if expired {
                    state.lock();
                    let _ = app.emit(STATUS_EVENT, state.status());
                }
            }
        });
    }
}

fn message(e: VaultError) -> String {
    e.to_string()
}

/// Runs Argon2 off the main thread so the window stays responsive.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, VaultError> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
        .map_err(message)
}

#[tauri::command]
pub fn vault_status(state: State<'_, VaultState>) -> Status {
    state.status()
}

#[tauri::command]
pub async fn vault_create(
    state: State<'_, VaultState>,
    password: String,
) -> Result<Status, String> {
    let path = state.path()?;
    let password = Zeroizing::new(password);
    let vault = blocking(move || Vault::create(&path, &password, KdfParams::default())).await?;
    state.set_unlocked(vault);
    Ok(state.status())
}

#[tauri::command]
pub async fn vault_unlock(
    state: State<'_, VaultState>,
    password: String,
) -> Result<Status, String> {
    let path = state.path()?;
    let password = Zeroizing::new(password);
    let vault = blocking(move || Vault::open(&path, &password)).await?;
    state.set_unlocked(vault);
    Ok(state.status())
}

#[tauri::command]
pub fn vault_lock(state: State<'_, VaultState>) -> Status {
    state.lock();
    state.status()
}

#[tauri::command]
pub fn vault_remove(
    state: State<'_, VaultState>,
    kind: SecretKind,
    target: String,
) -> Result<Status, String> {
    {
        let mut inner = state.lock_inner();
        let vault = inner.vault.as_mut().ok_or("the vault is locked")?;
        vault.remove(kind, &target).map_err(message)?;
        inner.last_used = Instant::now();
    }
    Ok(state.status())
}

#[tauri::command]
pub async fn vault_change_password(
    state: State<'_, VaultState>,
    current: String,
    new: String,
) -> Result<Status, String> {
    let current = Zeroizing::new(current);
    let new = Zeroizing::new(new);
    // Take the vault out while Argon2 runs, and always put it back.
    let vault = state
        .lock_inner()
        .vault
        .take()
        .ok_or("unlock the vault first")?;
    let (vault, result) = tauri::async_runtime::spawn_blocking(move || {
        let mut vault = vault;
        let result = vault.change_password(&current, &new);
        (vault, result)
    })
    .await
    .map_err(|e| e.to_string())?;
    state.set_unlocked(vault);
    result.map_err(message)?;
    Ok(state.status())
}
