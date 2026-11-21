//! The encrypted vault in the desktop app: create, unlock, lock, auto-lock,
//! "unlock automatically on this computer" (OS keychain), and lookups for
//! SSH connections.
//!
//! Secrets never cross to the web view. The UI sends a password or passphrase
//! once (to connect or to save it) and only ever gets back the list of
//! targets, never a stored secret. The vault key goes only to the OS keychain.

use serde::Serialize;
use setu_core::paths::{self, Os, ProcessEnv};
use setu_vault::keychain::{KeyStore, OsKeychain};
use setu_vault::{EntryInfo, KdfParams, SecretKind, Vault, VaultError, Zeroizing, VAULT_FILE};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

/// Lock after this long without the vault being used.
pub const AUTO_LOCK: Duration = Duration::from_secs(15 * 60);

/// Event sent to the UI when the vault locks or unlocks by itself.
pub const STATUS_EVENT: &str = "vault-status";

#[derive(Debug)]
struct Inner {
    vault: Option<Vault>,
    last_used: Instant,
    /// The OS keychain holds this vault's key.
    remembered: bool,
    /// "Lock now" was used: do not reopen from the keychain until the user
    /// unlocks with the password again.
    manually_locked: bool,
    /// Why the keychain could not be used, for the dialog.
    keychain_problem: Option<String>,
}

#[derive(Clone)]
pub struct VaultState {
    path: Result<PathBuf, String>,
    inner: Arc<Mutex<Inner>>,
    keychain: Arc<dyn KeyStore>,
}

impl std::fmt::Debug for VaultState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultState")
            .field("path", &self.path)
            .field("keychain", &self.keychain.name())
            .finish_non_exhaustive()
    }
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
pub struct Remember {
    /// The OS keychain can be used on this computer.
    available: bool,
    /// Its name, such as "macOS Keychain".
    store: &'static str,
    /// The vault unlocks automatically on this computer.
    enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    problem: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    exists: bool,
    unlocked: bool,
    path: String,
    entries: Vec<EntryInfo>,
    auto_lock_minutes: u64,
    remember: Remember,
}

impl VaultState {
    pub fn from_env() -> Self {
        let path = paths::config_dir(&ProcessEnv, Os::current())
            .map(|dir| dir.join(VAULT_FILE))
            .map_err(|e| e.to_string());
        Self::new(path, Arc::new(OsKeychain))
    }

    pub fn new(path: Result<PathBuf, String>, keychain: Arc<dyn KeyStore>) -> Self {
        VaultState {
            path,
            inner: Arc::new(Mutex::new(Inner {
                vault: None,
                last_used: Instant::now(),
                remembered: false,
                manually_locked: false,
                keychain_problem: None,
            })),
            keychain,
        }
    }

    fn path(&self) -> Result<PathBuf, String> {
        self.path.clone()
    }

    /// The keychain account for this vault: its path, so separate Setu
    /// homes (`SETU_HOME`) keep separate entries.
    fn account(&self) -> Option<String> {
        self.path.as_ref().ok().map(|p| p.display().to_string())
    }

    fn lock_inner(&self) -> MutexGuard<'_, Inner> {
        // A panic while holding the lock cannot leave the vault half-written
        // (saves are atomic), so recover the guard rather than fail forever.
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn status(&self) -> Status {
        let path = self.path.clone().unwrap_or_default();
        let inner = self.lock_inner();
        let keychain_ok = self.keychain.available();
        Status {
            exists: Vault::exists(&path),
            unlocked: inner.vault.is_some(),
            path: path.display().to_string(),
            entries: inner.vault.as_ref().map(Vault::entries).unwrap_or_default(),
            auto_lock_minutes: AUTO_LOCK.as_secs() / 60,
            remember: Remember {
                available: keychain_ok.is_ok(),
                store: self.keychain.name(),
                enabled: inner.remembered,
                problem: inner
                    .keychain_problem
                    .clone()
                    .or_else(|| keychain_ok.err().map(|e| e.to_string())),
            },
        }
    }

    /// At startup: learns whether the keychain holds this vault's key and,
    /// if it does, unlocks with it. A key that no longer fits (the master
    /// password was changed on another copy of the file) is removed.
    pub fn startup_unlock(&self) {
        let (Ok(path), Some(account)) = (self.path(), self.account()) else {
            return;
        };
        if !Vault::exists(&path) || self.keychain.available().is_err() {
            return;
        }
        let loaded = self.keychain.load(&account);
        let mut inner = self.lock_inner();
        match loaded {
            Ok(Some(key)) => match Vault::open_with_key(&path, &key) {
                Ok(vault) => {
                    inner.vault = Some(vault);
                    inner.last_used = Instant::now();
                    inner.remembered = true;
                }
                Err(VaultError::WrongPasswordOrTampered) => {
                    let _ = self.keychain.delete(&account);
                    inner.remembered = false;
                    inner.keychain_problem = Some(
                        "The saved key no longer matches the vault (its master password changed). Unlock with the password to save it again.".into(),
                    );
                }
                Err(e) => inner.keychain_problem = Some(e.to_string()),
            },
            Ok(None) => inner.remembered = false,
            Err(e) => inner.keychain_problem = Some(e.to_string()),
        }
    }

    /// Reopens a vault that auto-locked, from the keychain, when the user
    /// chose that and did not lock it by hand.
    fn reopen_from_keychain(&self, inner: &mut Inner) {
        if inner.vault.is_some() || !inner.remembered || inner.manually_locked {
            return;
        }
        let (Ok(path), Some(account)) = (self.path(), self.account()) else {
            return;
        };
        match self.keychain.load(&account) {
            Ok(Some(key)) => match Vault::open_with_key(&path, &key) {
                Ok(vault) => {
                    inner.vault = Some(vault);
                    inner.last_used = Instant::now();
                }
                Err(e) => {
                    inner.remembered = false;
                    inner.keychain_problem = Some(e.to_string());
                }
            },
            Ok(None) => inner.remembered = false,
            Err(e) => inner.keychain_problem = Some(e.to_string()),
        }
    }

    pub fn availability(&self) -> Availability {
        let mut inner = self.lock_inner();
        self.reopen_from_keychain(&mut inner);
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

    /// A stored secret, if the vault is unlocked (or can reopen from the
    /// keychain) and has one. Counts as use.
    pub fn get(&self, kind: SecretKind, target: &str) -> Option<Zeroizing<String>> {
        let mut inner = self.lock_inner();
        self.reopen_from_keychain(&mut inner);
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
        self.reopen_from_keychain(&mut inner);
        let Some(vault) = inner.vault.as_mut() else {
            return Ok(false);
        };
        vault.set(kind, target, secret).map_err(|e| e.to_string())?;
        inner.last_used = Instant::now();
        Ok(true)
    }

    /// Unlocked with the password: keep it, and save or clear the keychain
    /// entry when the user ticked or unticked "unlock automatically".
    fn set_unlocked(&self, vault: Vault, remember: Option<bool>) {
        let mut inner = self.lock_inner();
        inner.manually_locked = false;
        if let Some(enable) = remember {
            self.apply_remember(&mut inner, &vault, enable);
        } else if inner.remembered {
            // Keep the keychain in step (the key changes with the password).
            self.apply_remember(&mut inner, &vault, true);
        }
        inner.vault = Some(vault);
        inner.last_used = Instant::now();
    }

    fn apply_remember(&self, inner: &mut Inner, vault: &Vault, enable: bool) {
        let Some(account) = self.account() else {
            return;
        };
        let result = if enable {
            self.keychain.save(&account, &vault.unlock_key())
        } else {
            self.keychain.delete(&account)
        };
        match result {
            Ok(()) => {
                inner.remembered = enable;
                inner.keychain_problem = None;
            }
            Err(e) => {
                inner.remembered = false;
                inner.keychain_problem = Some(e.to_string());
            }
        }
    }

    fn lock(&self, by_hand: bool) {
        let mut inner = self.lock_inner();
        // Dropping the vault zeroizes its key and secrets.
        inner.vault = None;
        if by_hand {
            inner.manually_locked = true;
        }
    }

    /// Locks the vault after [`AUTO_LOCK`] of inactivity and tells the UI.
    /// With "unlock automatically" on, the next use reopens it from the
    /// keychain; meanwhile its secrets are out of memory.
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
                    state.lock(false);
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
    remember: Option<bool>,
) -> Result<Status, String> {
    let path = state.path()?;
    let password = Zeroizing::new(password);
    let vault = blocking(move || Vault::create(&path, &password, KdfParams::default())).await?;
    state.set_unlocked(vault, remember);
    Ok(state.status())
}

#[tauri::command]
pub async fn vault_unlock(
    state: State<'_, VaultState>,
    password: String,
    remember: Option<bool>,
) -> Result<Status, String> {
    let path = state.path()?;
    let password = Zeroizing::new(password);
    let vault = blocking(move || Vault::open(&path, &password)).await?;
    state.set_unlocked(vault, remember);
    Ok(state.status())
}

#[tauri::command]
pub fn vault_lock(state: State<'_, VaultState>) -> Status {
    state.lock(true);
    state.status()
}

/// Turns "unlock automatically on this computer" on or off. The vault must
/// be unlocked: turning it on stores the current key.
#[tauri::command]
pub fn vault_remember(state: State<'_, VaultState>, enabled: bool) -> Result<Status, String> {
    {
        let mut inner = state.lock_inner();
        let vault = inner.vault.take().ok_or("unlock the vault first")?;
        state.apply_remember(&mut inner, &vault, enabled);
        inner.vault = Some(vault);
        if let Some(problem) = inner.keychain_problem.clone() {
            return Err(problem);
        }
    }
    Ok(state.status())
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
    // Puts the vault back and, if remembered, stores the new key.
    state.set_unlocked(vault, None);
    result.map_err(message)?;
    Ok(state.status())
}

#[cfg(test)]
mod tests {
    use super::*;
    use setu_vault::keychain::MemoryKeyStore;

    const PW: &str = "correct horse battery";

    fn setup() -> (tempfile::TempDir, Arc<MemoryKeyStore>, VaultState) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(VAULT_FILE);
        let mut vault = Vault::create(&path, PW, KdfParams::insecure_fast()).expect("create");
        vault
            .set(
                SecretKind::SshPassword,
                "ops@a:22",
                Zeroizing::new("pw".into()),
            )
            .expect("set");
        let store = Arc::new(MemoryKeyStore::default());
        let state = VaultState::new(Ok(path), store.clone());
        (dir, store, state)
    }

    fn unlock(state: &VaultState, remember: Option<bool>) {
        let vault = Vault::open(&state.path().expect("path"), PW).expect("open");
        state.set_unlocked(vault, remember);
    }

    #[test]
    fn remembered_vault_unlocks_at_startup_and_after_auto_lock() {
        let (_dir, store, state) = setup();
        unlock(&state, Some(true));
        assert!(state.status().remember.enabled);

        // A new app start finds the key in the keychain.
        let fresh = VaultState::new(state.path(), store.clone());
        fresh.startup_unlock();
        let status = fresh.status();
        assert!(status.unlocked && status.remember.enabled);

        // Auto-lock clears memory; the next use reopens from the keychain.
        fresh.lock(false);
        assert!(!fresh.status().unlocked);
        assert_eq!(
            fresh
                .get(SecretKind::SshPassword, "ops@a:22")
                .as_deref()
                .map(String::as_str),
            Some("pw")
        );
    }

    #[test]
    fn lock_now_is_not_undone_by_the_keychain() {
        let (_dir, _store, state) = setup();
        unlock(&state, Some(true));
        state.lock(true);
        assert_eq!(state.availability(), Availability::Locked);
        assert!(state.get(SecretKind::SshPassword, "ops@a:22").is_none());
        unlock(&state, None);
        assert_eq!(state.availability(), Availability::Unlocked);
    }

    #[test]
    fn turning_it_off_removes_the_key() {
        let (_dir, store, state) = setup();
        unlock(&state, Some(true));
        unlock(&state, Some(false));
        assert!(!state.status().remember.enabled);
        let account = state.account().expect("account");
        assert_eq!(store.load(&account).expect("load"), None);

        let fresh = VaultState::new(state.path(), store);
        fresh.startup_unlock();
        assert!(!fresh.status().unlocked);
    }

    #[test]
    fn stale_key_is_dropped_with_a_message() {
        let (_dir, store, state) = setup();
        unlock(&state, Some(true));
        // Change the password on the file without this app knowing.
        let mut other = Vault::open(&state.path().expect("path"), PW).expect("open");
        other
            .change_password(PW, "another long password")
            .expect("change");

        let fresh = VaultState::new(state.path(), store.clone());
        fresh.startup_unlock();
        let status = fresh.status();
        assert!(!status.unlocked && !status.remember.enabled);
        assert!(status
            .remember
            .problem
            .is_some_and(|p| p.contains("no longer matches")));
        assert_eq!(
            store
                .load(&fresh.account().expect("account"))
                .expect("load"),
            None
        );
    }

    #[test]
    fn keychain_failure_falls_back_to_the_password() {
        let (_dir, store, state) = setup();
        *store.fail.lock().expect("lock") = Some("keyring is locked".into());
        unlock(&state, Some(true));
        let status = state.status();
        assert!(status.unlocked, "the password unlock itself still works");
        assert!(!status.remember.enabled && !status.remember.available);
        assert_eq!(
            status.remember.problem.as_deref(),
            Some("keyring is locked")
        );
    }

    #[test]
    fn password_change_updates_the_stored_key() {
        let (_dir, store, state) = setup();
        unlock(&state, Some(true));
        {
            let mut inner = state.lock_inner();
            let vault = inner.vault.as_mut().expect("unlocked");
            vault
                .change_password(PW, "another long password")
                .expect("change");
        }
        let vault = state.lock_inner().vault.take().expect("vault");
        state.set_unlocked(vault, None);
        let fresh = VaultState::new(state.path(), store);
        fresh.startup_unlock();
        assert!(fresh.status().unlocked);
    }
}
