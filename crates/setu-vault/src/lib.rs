//! Setu's encrypted local vault.
//!
//! The vault keeps SSH passwords and key passphrases in one file
//! (`vault.json` in Setu's config folder), encrypted with a key derived
//! from a master password:
//!
//! - **Key derivation:** Argon2id (64 MiB, 3 passes, 1 lane by default) over the
//!   master password and a random 16-byte salt gives a 256-bit key.
//! - **Encryption:** XChaCha20-Poly1305 with a fresh random 24-byte nonce on
//!   every save. The file header (format, KDF parameters, salt, cipher) is the
//!   associated data, so changing any of it makes decryption fail.
//! - **In memory:** the derived key and every secret are zeroized on drop. The
//!   master password itself is never kept.
//!
//! Nothing is ever recoverable without the master password; there is no
//! backdoor and no reset.

mod error;
mod file;
mod vault;

pub use error::VaultError;
pub use vault::{EntryInfo, KdfParams, SecretKind, Vault, MIN_PASSWORD_CHARS, VAULT_FILE};
pub use zeroize::Zeroizing;
