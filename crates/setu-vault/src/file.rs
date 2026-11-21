//! The on-disk vault format.
//!
//! ```json
//! {
//!   "format": "setu-vault",
//!   "version": 1,
//!   "kdf": { "algorithm": "argon2id", "memory_kib": 65536, "iterations": 3, "parallelism": 1 },
//!   "salt": "<base64, 16 bytes>",
//!   "cipher": "xchacha20poly1305",
//!   "nonce": "<base64, 24 bytes>",
//!   "ciphertext": "<base64>"
//! }
//! ```
//!
//! Everything except `nonce` and `ciphertext` is authenticated as associated
//! data (see [`VaultFile::aad`]).

use crate::{KdfParams, VaultError};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "setu-vault";
pub const VERSION: u32 = 1;
pub const CIPHER: &str = "xchacha20poly1305";
pub const SALT_LEN: usize = 16;
pub const NONCE_LEN: usize = 24;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultFile {
    pub format: String,
    pub version: u32,
    pub kdf: KdfParams,
    pub salt: String,
    pub cipher: String,
    pub nonce: String,
    pub ciphertext: String,
}

/// The authenticated part of the header, serialized in a fixed field order.
#[derive(Serialize)]
struct Aad<'a> {
    format: &'a str,
    version: u32,
    kdf: &'a KdfParams,
    salt: &'a str,
    cipher: &'a str,
}

impl VaultFile {
    pub fn aad(&self) -> Vec<u8> {
        // Serializing a plain struct of strings and integers cannot fail.
        serde_json::to_vec(&Aad {
            format: &self.format,
            version: self.version,
            kdf: &self.kdf,
            salt: &self.salt,
            cipher: &self.cipher,
        })
        .unwrap_or_default()
    }

    pub fn read(path: &Path) -> Result<Self, VaultError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(VaultError::NotFound(path.to_path_buf()))
            }
            Err(source) => {
                return Err(VaultError::Read {
                    path: path.to_path_buf(),
                    source,
                })
            }
        };
        let format_err = |message: String| VaultError::Format {
            path: path.to_path_buf(),
            message,
        };
        let file: VaultFile = serde_json::from_str(&text).map_err(|e| format_err(e.to_string()))?;
        if file.format != FORMAT {
            return Err(format_err(format!("unknown format `{}`", file.format)));
        }
        if file.version > VERSION {
            return Err(VaultError::NewerFormat {
                path: path.to_path_buf(),
                found: file.version,
            });
        }
        if file.cipher != CIPHER {
            return Err(format_err(format!("unknown cipher `{}`", file.cipher)));
        }
        file.kdf.validate()?;
        Ok(file)
    }

    pub fn salt(&self, path: &Path) -> Result<[u8; SALT_LEN], VaultError> {
        decode_fixed(&self.salt, "salt", path)
    }

    pub fn nonce(&self, path: &Path) -> Result<[u8; NONCE_LEN], VaultError> {
        decode_fixed(&self.nonce, "nonce", path)
    }

    pub fn ciphertext(&self, path: &Path) -> Result<Vec<u8>, VaultError> {
        B64.decode(&self.ciphertext)
            .map_err(|e| VaultError::Format {
                path: path.to_path_buf(),
                message: format!("ciphertext: {e}"),
            })
    }

    /// Writes the file atomically (temp file, then rename), readable only by
    /// the owner on Unix.
    pub fn write(&self, path: &Path) -> Result<(), VaultError> {
        let write_err = |source| VaultError::Write {
            path: path.to_path_buf(),
            source,
        };
        let body =
            serde_json::to_vec_pretty(self).map_err(|e| write_err(std::io::Error::other(e)))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(write_err)?;
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
            file.write_all(&body)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&tmp, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result.map_err(write_err)
    }
}

fn decode_fixed<const N: usize>(
    value: &str,
    what: &str,
    path: &Path,
) -> Result<[u8; N], VaultError> {
    let bytes = B64.decode(value).map_err(|e| VaultError::Format {
        path: path.to_path_buf(),
        message: format!("{what}: {e}"),
    })?;
    bytes.try_into().map_err(|_| VaultError::Format {
        path: path.to_path_buf(),
        message: format!("{what} must be {N} bytes"),
    })
}

pub fn encode(bytes: &[u8]) -> String {
    B64.encode(bytes)
}

pub fn decode(text: &str) -> Option<Vec<u8>> {
    B64.decode(text).ok()
}
