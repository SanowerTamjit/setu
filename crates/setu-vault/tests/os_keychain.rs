//! Saves, loads and deletes a vault key in the real OS keychain.
//!
//! Opt-in, because CI machines have no unlocked keychain: run with
//! `SETU_TEST_OS_KEYCHAIN=1 cargo test -p setu-vault --features os-keychain`
//! inside a desktop session (on Linux, a running Secret Service).
#![cfg(feature = "os-keychain")]

use setu_vault::keychain::{KeyStore, OsKeychain};
use setu_vault::{KdfParams, Vault, VAULT_FILE};

#[test]
fn os_keychain_round_trip() {
    if std::env::var_os("SETU_TEST_OS_KEYCHAIN").is_none() {
        eprintln!("skipping: set SETU_TEST_OS_KEYCHAIN=1 to use the real keychain");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(VAULT_FILE);
    let vault =
        Vault::create(&path, "correct horse battery", KdfParams::insecure_fast()).expect("create");
    let account = path.display().to_string();
    let store = OsKeychain;
    store.available().expect("keychain available");
    assert_eq!(store.load(&account).expect("load"), None);
    store.save(&account, &vault.unlock_key()).expect("save");
    let key = store.load(&account).expect("load").expect("saved key");
    Vault::open_with_key(&path, &key).expect("opens with the stored key");
    store.delete(&account).expect("delete");
    assert_eq!(store.load(&account).expect("load"), None);
}
