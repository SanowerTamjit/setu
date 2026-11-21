use std::path::PathBuf;

/// Errors from creating, unlocking or saving the vault.
///
/// No variant ever carries a secret.
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    /// Wrong master password, or the file was changed or corrupted. AEAD cannot
    /// tell the two apart, and saying which would help an attacker.
    #[error("wrong master password, or the vault file is damaged")]
    WrongPasswordOrTampered,
    #[error("a vault already exists at {0}")]
    AlreadyExists(PathBuf),
    #[error("no vault at {0}")]
    NotFound(PathBuf),
    #[error("the master password must be at least {0} characters")]
    WeakPassword(usize),
    #[error("{path} is not a Setu vault: {message}")]
    Format { path: PathBuf, message: String },
    #[error("{path} was written by a newer version of Setu (vault format {found})")]
    NewerFormat { path: PathBuf, found: u32 },
    #[error("unsupported key derivation settings: {0}")]
    KdfParams(String),
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not save {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("the system random number generator failed: {0}")]
    Random(String),
    #[error("encryption failed")]
    Encrypt,
}
