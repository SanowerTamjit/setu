//! "Unlock automatically on this computer": keeping a vault's
//! [`UnlockKey`] in the operating system's credential store, so the vault
//! opens without the master password while you are logged in.
//!
//! | OS      | Store                                              |
//! |---------|----------------------------------------------------|
//! | macOS   | Keychain (login keychain)                          |
//! | Windows | Credential Manager                                 |
//! | Linux   | Secret Service (GNOME Keyring, KWallet, KeePassXC)  |
//!
//! The trade-off, which the UI states: anyone who can use your unlocked
//! login session can then open the vault. It is off by default.

use crate::UnlockKey;
use std::sync::Mutex;

/// The keychain service name entries are saved under.
pub const SERVICE: &str = "dev.setu.vault";

/// Errors from the OS credential store, worded for the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct KeychainError(pub String);

/// Where an [`UnlockKey`] is kept. One entry per vault file.
pub trait KeyStore: Send + Sync {
    /// The store's name for the UI, such as "macOS Keychain".
    fn name(&self) -> &'static str;
    /// Whether the store can be used here (a Linux desktop without a
    /// Secret Service, for example, has none).
    fn available(&self) -> Result<(), KeychainError>;
    fn load(&self, vault: &str) -> Result<Option<UnlockKey>, KeychainError>;
    fn save(&self, vault: &str, key: &UnlockKey) -> Result<(), KeychainError>;
    /// Removes the entry; removing one that is not there is not an error.
    fn delete(&self, vault: &str) -> Result<(), KeychainError>;
}

/// An in-memory store for tests.
#[derive(Debug, Default)]
pub struct MemoryKeyStore {
    entries: Mutex<Vec<(String, UnlockKey)>>,
    /// When set, every call fails with this message (to test fallbacks).
    pub fail: Mutex<Option<String>>,
}

impl MemoryKeyStore {
    fn check(&self) -> Result<(), KeychainError> {
        match self.fail.lock().map(|f| f.clone()) {
            Ok(Some(message)) => Err(KeychainError(message)),
            _ => Ok(()),
        }
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, Vec<(String, UnlockKey)>> {
        self.entries.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl KeyStore for MemoryKeyStore {
    fn name(&self) -> &'static str {
        "test keychain"
    }

    fn available(&self) -> Result<(), KeychainError> {
        self.check()
    }

    fn load(&self, vault: &str) -> Result<Option<UnlockKey>, KeychainError> {
        self.check()?;
        Ok(self
            .entries()
            .iter()
            .find(|(v, _)| v == vault)
            .map(|(_, k)| k.clone()))
    }

    fn save(&self, vault: &str, key: &UnlockKey) -> Result<(), KeychainError> {
        self.check()?;
        let mut entries = self.entries();
        entries.retain(|(v, _)| v != vault);
        entries.push((vault.to_string(), key.clone()));
        Ok(())
    }

    fn delete(&self, vault: &str) -> Result<(), KeychainError> {
        self.check()?;
        self.entries().retain(|(v, _)| v != vault);
        Ok(())
    }
}

/// The operating system's credential store.
#[cfg(feature = "os-keychain")]
#[derive(Debug, Default, Clone, Copy)]
pub struct OsKeychain;

#[cfg(feature = "os-keychain")]
impl OsKeychain {
    fn entry(vault: &str) -> Result<keyring::Entry, KeychainError> {
        keyring::Entry::new(SERVICE, vault).map_err(os_error)
    }
}

#[cfg(feature = "os-keychain")]
fn os_error(e: keyring::Error) -> KeychainError {
    KeychainError(match e {
        keyring::Error::NoDefaultStore | keyring::Error::NoStorageAccess(_) => {
            if cfg!(target_os = "linux") {
                "no Secret Service is running (install or start GNOME Keyring, KWallet or KeePassXC)".into()
            } else {
                format!("the system keychain is not available: {e}")
            }
        }
        other => other.to_string(),
    })
}

#[cfg(feature = "os-keychain")]
impl KeyStore for OsKeychain {
    fn name(&self) -> &'static str {
        if cfg!(target_os = "macos") {
            "macOS Keychain"
        } else if cfg!(windows) {
            "Windows Credential Manager"
        } else {
            "the system keyring (Secret Service)"
        }
    }

    fn available(&self) -> Result<(), KeychainError> {
        match keyring::Entry::store_status() {
            Ok(()) => Ok(()),
            Err(_) => Err(os_error(keyring::Error::NoDefaultStore)),
        }
    }

    fn load(&self, vault: &str) -> Result<Option<UnlockKey>, KeychainError> {
        match Self::entry(vault)?.get_password() {
            // Something else under our name is treated as no entry.
            Ok(text) => Ok(UnlockKey::from_text(&zeroize::Zeroizing::new(text))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(os_error(e)),
        }
    }

    fn save(&self, vault: &str, key: &UnlockKey) -> Result<(), KeychainError> {
        Self::entry(vault)?
            .set_password(&key.to_text())
            .map_err(os_error)
    }

    fn delete(&self, vault: &str) -> Result<(), KeychainError> {
        match Self::entry(vault)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(os_error(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KdfParams, Vault, VAULT_FILE};

    #[test]
    fn memory_store_round_trip_and_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let vault = Vault::create(
            &dir.path().join(VAULT_FILE),
            "correct horse battery",
            KdfParams::insecure_fast(),
        )
        .expect("create");
        let store = MemoryKeyStore::default();
        assert_eq!(store.load("a").expect("load"), None);
        store.save("a", &vault.unlock_key()).expect("save");
        assert_eq!(store.load("a").expect("load"), Some(vault.unlock_key()));
        assert_eq!(store.load("b").expect("load"), None);
        store.delete("a").expect("delete");
        store.delete("a").expect("delete twice");
        assert_eq!(store.load("a").expect("load"), None);

        *store.fail.lock().expect("lock") = Some("locked keyring".into());
        assert_eq!(store.load("a"), Err(KeychainError("locked keyring".into())));
    }
}
