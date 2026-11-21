use crate::file::{self, VaultFile, NONCE_LEN, SALT_LEN};
use crate::VaultError;
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// File name of the vault inside Setu's config folder.
pub const VAULT_FILE: &str = "vault.json";

/// Shortest master password Setu accepts.
pub const MIN_PASSWORD_CHARS: usize = 8;

const KEY_LEN: usize = 32;

/// Argon2id cost settings, stored in the vault header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    pub algorithm: KdfAlgorithm,
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KdfAlgorithm {
    Argon2id,
}

impl Default for KdfParams {
    /// 64 MiB, 3 passes, 1 lane: above OWASP's Argon2id recommendation and
    /// about half a second on a laptop.
    fn default() -> Self {
        KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            memory_kib: 64 * 1024,
            iterations: 3,
            parallelism: 1,
        }
    }
}

impl KdfParams {
    /// Cheap settings for tests only. Never use them for a real vault.
    #[doc(hidden)]
    pub fn insecure_fast() -> Self {
        KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            memory_kib: 64,
            iterations: 1,
            parallelism: 1,
        }
    }

    /// Rejects settings that would make unlocking hang or eat all memory
    /// (a damaged or hostile file must not be able to do that).
    pub(crate) fn validate(&self) -> Result<(), VaultError> {
        if self.memory_kib > 4 * 1024 * 1024 || self.iterations > 64 || self.parallelism > 16 {
            return Err(VaultError::KdfParams(format!(
                "{} KiB, {} passes, {} lanes is beyond what Setu allows",
                self.memory_kib, self.iterations, self.parallelism
            )));
        }
        self.argon2().map(|_| ())
    }

    fn argon2(&self) -> Result<Argon2<'static>, VaultError> {
        let params = Params::new(
            self.memory_kib,
            self.iterations,
            self.parallelism,
            Some(KEY_LEN),
        )
        .map_err(|e| VaultError::KdfParams(e.to_string()))?;
        Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
    }

    fn derive(&self, password: &str, salt: &[u8]) -> Result<Zeroizing<[u8; KEY_LEN]>, VaultError> {
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        self.argon2()?
            .hash_password_into(password.as_bytes(), salt, key.as_mut())
            .map_err(|e| VaultError::KdfParams(e.to_string()))?;
        Ok(key)
    }
}

/// What a secret unlocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecretKind {
    /// An SSH login password; the target is `user@host:port`.
    SshPassword,
    /// A private key's passphrase; the target is the key file's path.
    KeyPassphrase,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    kind: SecretKind,
    target: String,
    secret: Zeroizing<String>,
    /// Seconds since the Unix epoch.
    updated: u64,
}

#[derive(Serialize, Deserialize, Default)]
struct Contents {
    entries: Vec<Entry>,
}

/// A saved secret, without the secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntryInfo {
    pub kind: SecretKind,
    pub target: String,
    pub updated: u64,
}

/// An unlocked vault. Dropping it zeroizes the key and every secret.
pub struct Vault {
    path: PathBuf,
    kdf: KdfParams,
    salt: [u8; SALT_LEN],
    key: Zeroizing<[u8; KEY_LEN]>,
    contents: Contents,
}

impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vault")
            .field("path", &self.path)
            .field("entries", &self.contents.entries.len())
            .finish_non_exhaustive()
    }
}

fn random<const N: usize>() -> Result<[u8; N], VaultError> {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).map_err(|e| VaultError::Random(e.to_string()))?;
    Ok(buf)
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn check_strength(password: &str) -> Result<(), VaultError> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(VaultError::WeakPassword(MIN_PASSWORD_CHARS));
    }
    Ok(())
}

impl Vault {
    /// Whether a vault file exists at `path`.
    pub fn exists(path: &Path) -> bool {
        path.is_file()
    }

    /// Checks that the file at `path` is a vault this version can open
    /// (format, version, cipher, KDF settings and encodings), without the
    /// master password. It cannot tell whether the ciphertext is intact.
    pub fn check_file(path: &Path) -> Result<(), VaultError> {
        let file = VaultFile::read(path)?;
        file.salt(path)?;
        file.nonce(path)?;
        file.ciphertext(path)?;
        Ok(())
    }

    /// Creates an empty vault. Refuses to replace an existing one.
    pub fn create(path: &Path, password: &str, kdf: KdfParams) -> Result<Self, VaultError> {
        check_strength(password)?;
        if Self::exists(path) {
            return Err(VaultError::AlreadyExists(path.to_path_buf()));
        }
        kdf.validate()?;
        let salt = random::<SALT_LEN>()?;
        let key = kdf.derive(password, &salt)?;
        let vault = Vault {
            path: path.to_path_buf(),
            kdf,
            salt,
            key,
            contents: Contents::default(),
        };
        vault.save()?;
        Ok(vault)
    }

    /// Unlocks the vault at `path` with the master password.
    pub fn open(path: &Path, password: &str) -> Result<Self, VaultError> {
        let file = VaultFile::read(path)?;
        let salt = file.salt(path)?;
        let nonce = file.nonce(path)?;
        let ciphertext = file.ciphertext(path)?;
        let key = file.kdf.derive(password, &salt)?;
        let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref())
            .map_err(|_| VaultError::WrongPasswordOrTampered)?;
        let aad = file.aad();
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(
                    &XNonce::from(nonce),
                    Payload {
                        msg: &ciphertext,
                        aad: &aad,
                    },
                )
                .map_err(|_| VaultError::WrongPasswordOrTampered)?,
        );
        let contents: Contents =
            serde_json::from_slice(&plaintext).map_err(|e| VaultError::Format {
                path: path.to_path_buf(),
                message: format!("decrypted contents: {e}"),
            })?;
        Ok(Vault {
            path: path.to_path_buf(),
            kdf: file.kdf,
            salt,
            key,
            contents,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Encrypts and writes the vault with a fresh nonce.
    fn save(&self) -> Result<(), VaultError> {
        let nonce = random::<NONCE_LEN>()?;
        let mut file = VaultFile {
            format: file::FORMAT.into(),
            version: file::VERSION,
            kdf: self.kdf,
            salt: file::encode(&self.salt),
            cipher: file::CIPHER.into(),
            nonce: file::encode(&nonce),
            ciphertext: String::new(),
        };
        let plaintext =
            Zeroizing::new(serde_json::to_vec(&self.contents).map_err(|_| VaultError::Encrypt)?);
        let cipher = XChaCha20Poly1305::new_from_slice(self.key.as_ref())
            .map_err(|_| VaultError::Encrypt)?;
        let aad = file.aad();
        let ciphertext = cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: &plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| VaultError::Encrypt)?;
        file.ciphertext = file::encode(&ciphertext);
        file.write(&self.path)
    }

    fn position(&self, kind: SecretKind, target: &str) -> Option<usize> {
        self.contents
            .entries
            .iter()
            .position(|e| e.kind == kind && e.target == target)
    }

    /// The secret saved for `target`, if any.
    pub fn get(&self, kind: SecretKind, target: &str) -> Option<Zeroizing<String>> {
        self.position(kind, target)
            .map(|i| self.contents.entries[i].secret.clone())
    }

    /// Saves (or replaces) a secret and writes the vault.
    pub fn set(
        &mut self,
        kind: SecretKind,
        target: &str,
        secret: Zeroizing<String>,
    ) -> Result<(), VaultError> {
        let entry = Entry {
            kind,
            target: target.to_string(),
            secret,
            updated: now(),
        };
        match self.position(kind, target) {
            Some(i) => self.contents.entries[i] = entry,
            None => self.contents.entries.push(entry),
        }
        self.save()
    }

    /// Removes a secret. Returns whether one was saved.
    pub fn remove(&mut self, kind: SecretKind, target: &str) -> Result<bool, VaultError> {
        match self.position(kind, target) {
            Some(i) => {
                self.contents.entries.remove(i);
                self.save()?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Every saved secret, without the secrets, sorted by target.
    pub fn entries(&self) -> Vec<EntryInfo> {
        let mut out: Vec<EntryInfo> = self
            .contents
            .entries
            .iter()
            .map(|e| EntryInfo {
                kind: e.kind,
                target: e.target.clone(),
                updated: e.updated,
            })
            .collect();
        out.sort_by(|a, b| a.target.cmp(&b.target));
        out
    }

    /// Re-encrypts the vault under a new master password and a new salt.
    /// The current password must be given again, so an unattended unlocked
    /// app cannot be used to take over the vault.
    pub fn change_password(&mut self, current: &str, new: &str) -> Result<(), VaultError> {
        check_strength(new)?;
        let check = self.kdf.derive(current, &self.salt)?;
        if !constant_time_eq(check.as_ref(), self.key.as_ref()) {
            return Err(VaultError::WrongPasswordOrTampered);
        }
        let salt = random::<SALT_LEN>()?;
        let kdf = KdfParams::default().max_with(self.kdf);
        let key = kdf.derive(new, &salt)?;
        let (old_salt, old_key, old_kdf) = (self.salt, self.key.clone(), self.kdf);
        self.salt = salt;
        self.key = key;
        self.kdf = kdf;
        if let Err(e) = self.save() {
            self.salt = old_salt;
            self.key = old_key;
            self.kdf = old_kdf;
            return Err(e);
        }
        Ok(())
    }
}

impl KdfParams {
    /// The stronger of each setting, so changing the password never weakens
    /// a vault (test vaults stay cheap because tests pass their own).
    fn max_with(self, other: KdfParams) -> KdfParams {
        if other.memory_kib < 1024 {
            // A test vault: keep it cheap.
            return other;
        }
        KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            memory_kib: self.memory_kib.max(other.memory_kib),
            iterations: self.iterations.max(other.iterations),
            parallelism: self.parallelism.max(other.parallelism),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PW: &str = "correct horse battery";

    fn new_vault(dir: &Path) -> Vault {
        Vault::create(&dir.join(VAULT_FILE), PW, KdfParams::insecure_fast()).expect("create")
    }

    fn secret(s: &str) -> Zeroizing<String> {
        Zeroizing::new(s.to_string())
    }

    #[test]
    fn check_file_needs_no_password() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(VAULT_FILE);
        assert!(matches!(
            Vault::check_file(&path),
            Err(VaultError::NotFound(_))
        ));
        new_vault(dir.path());
        Vault::check_file(&path).expect("valid vault");
        std::fs::write(&path, "{\"format\":\"other\"}").expect("write");
        assert!(Vault::check_file(&path).is_err());
    }

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut vault = new_vault(dir.path());
        vault
            .set(SecretKind::SshPassword, "ops@web-01:22", secret("hunter2"))
            .expect("set");
        vault
            .set(
                SecretKind::KeyPassphrase,
                "/home/a/.ssh/id_ed25519",
                secret("pp"),
            )
            .expect("set");
        vault
            .set(SecretKind::SshPassword, "ops@web-01:22", secret("hunter3"))
            .expect("replace");
        drop(vault);

        let vault = Vault::open(&dir.path().join(VAULT_FILE), PW).expect("open");
        assert_eq!(
            vault
                .get(SecretKind::SshPassword, "ops@web-01:22")
                .as_deref()
                .map(String::as_str),
            Some("hunter3")
        );
        assert_eq!(vault.entries().len(), 2);
        assert!(vault
            .get(SecretKind::KeyPassphrase, "ops@web-01:22")
            .is_none());
    }

    #[test]
    fn file_holds_no_plaintext() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut vault = new_vault(dir.path());
        vault
            .set(
                SecretKind::SshPassword,
                "ops@web-01:22",
                secret("hunter2-unique"),
            )
            .expect("set");
        let text = std::fs::read_to_string(vault.path()).expect("read");
        assert!(!text.contains("hunter2-unique"));
        assert!(!text.contains("web-01"));
        assert!(text.contains("\"format\": \"setu-vault\""));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(vault.path())
                .expect("meta")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn wrong_password_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = new_vault(dir.path()).path().to_path_buf();
        assert!(matches!(
            Vault::open(&path, "not the password"),
            Err(VaultError::WrongPasswordOrTampered)
        ));
    }

    #[test]
    fn tampering_is_detected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut vault = new_vault(dir.path());
        vault
            .set(SecretKind::SshPassword, "a@b:22", secret("x"))
            .expect("set");
        let path = vault.path().to_path_buf();
        let original = std::fs::read_to_string(&path).expect("read");
        let mut file: VaultFile = serde_json::from_str(&original).expect("json");

        // A changed header field (here the KDF cost) is caught by the AAD.
        let mut weaker = file.clone();
        weaker.kdf.iterations += 1;
        std::fs::write(&path, serde_json::to_string(&weaker).expect("json")).expect("write");
        assert!(matches!(
            Vault::open(&path, PW),
            Err(VaultError::WrongPasswordOrTampered)
        ));

        // A flipped ciphertext bit is caught by the tag.
        let mut bytes = file.ciphertext(&path).expect("ct");
        bytes[0] ^= 1;
        file.ciphertext = file::encode(&bytes);
        std::fs::write(&path, serde_json::to_string(&file).expect("json")).expect("write");
        assert!(matches!(
            Vault::open(&path, PW),
            Err(VaultError::WrongPasswordOrTampered)
        ));
    }

    #[test]
    fn each_save_uses_a_new_nonce() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut vault = new_vault(dir.path());
        let read_nonce = |p: &Path| {
            serde_json::from_str::<VaultFile>(&std::fs::read_to_string(p).expect("read"))
                .expect("json")
                .nonce
        };
        let first = read_nonce(vault.path());
        vault
            .set(SecretKind::SshPassword, "a@b:22", secret("x"))
            .expect("set");
        assert_ne!(first, read_nonce(vault.path()));
    }

    #[test]
    fn create_rules() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(VAULT_FILE);
        assert!(matches!(
            Vault::create(&path, "short", KdfParams::insecure_fast()),
            Err(VaultError::WeakPassword(_))
        ));
        new_vault(dir.path());
        assert!(matches!(
            Vault::create(&path, PW, KdfParams::insecure_fast()),
            Err(VaultError::AlreadyExists(_))
        ));
        assert!(matches!(
            Vault::open(&dir.path().join("missing.json"), PW),
            Err(VaultError::NotFound(_))
        ));
    }

    #[test]
    fn hostile_kdf_settings_are_refused_before_hashing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = new_vault(dir.path()).path().to_path_buf();
        let mut file: VaultFile =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("json");
        file.kdf.memory_kib = u32::MAX;
        std::fs::write(&path, serde_json::to_string(&file).expect("json")).expect("write");
        assert!(matches!(
            Vault::open(&path, PW),
            Err(VaultError::KdfParams(_))
        ));
    }

    #[test]
    fn change_password() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut vault = new_vault(dir.path());
        vault
            .set(SecretKind::SshPassword, "a@b:22", secret("x"))
            .expect("set");
        assert!(vault
            .change_password("wrong one!", "new password 1")
            .is_err());
        assert!(vault.change_password(PW, "short").is_err());
        vault.change_password(PW, "new password 1").expect("change");
        let path = vault.path().to_path_buf();
        drop(vault);
        assert!(Vault::open(&path, PW).is_err());
        let vault = Vault::open(&path, "new password 1").expect("open");
        assert!(vault.get(SecretKind::SshPassword, "a@b:22").is_some());
    }

    #[test]
    fn remove_and_debug_hides_secrets() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut vault = new_vault(dir.path());
        vault
            .set(SecretKind::SshPassword, "a@b:22", secret("topsecret"))
            .expect("set");
        assert!(!format!("{vault:?}").contains("topsecret"));
        assert!(vault
            .remove(SecretKind::SshPassword, "a@b:22")
            .expect("remove"));
        assert!(!vault
            .remove(SecretKind::SshPassword, "a@b:22")
            .expect("remove"));
        assert!(vault.entries().is_empty());
    }

    #[test]
    fn default_params_are_valid() {
        KdfParams::default().validate().expect("valid");
    }
}
