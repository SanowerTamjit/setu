//! Making new SSH keys and listing the ones in a folder (usually `~/.ssh`).
//!
//! Keys are written in OpenSSH format, the private half readable by you
//! alone. An existing file is never overwritten.

use crate::keys::{self, KeyState};
use russh::keys::{Algorithm, EcdsaCurve, HashAlg, PrivateKey, PublicKey};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The kinds of key Setu makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    /// Short, fast and the usual choice today.
    Ed25519,
    /// ECDSA on NIST P-256, for servers that want a NIST curve.
    EcdsaP256,
    /// RSA, 4096 bits, for old servers that accept nothing else.
    Rsa4096,
}

impl KeyKind {
    /// The usual file name for this kind, as `ssh-keygen` picks it.
    pub fn default_file_name(self) -> &'static str {
        match self {
            KeyKind::Ed25519 => "id_ed25519",
            KeyKind::EcdsaP256 => "id_ecdsa",
            KeyKind::Rsa4096 => "id_rsa",
        }
    }
}

/// Why a key could not be made.
#[derive(Debug, thiserror::Error)]
pub enum KeyGenError {
    #[error("use a plain file name such as id_ed25519, without folders")]
    BadName,
    #[error("{0} already exists; pick another name")]
    Exists(PathBuf),
    #[error("could not make the key: {0}")]
    Crypto(String),
    #[error("could not write {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// A key that was just written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewKey {
    pub private_path: PathBuf,
    pub public_path: PathBuf,
    /// `SHA256:…`.
    pub fingerprint: String,
    /// The `.pub` line, ready to paste into `authorized_keys`.
    pub public_line: String,
}

fn valid_file_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.ends_with(".pub")
        && name.len() <= 100
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '@'))
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> KeyGenError + '_ {
    move |source| KeyGenError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Opens `path` for writing only if it does not exist yet, with `mode` on Unix.
fn create_new(path: &Path, mode: u32) -> Result<std::fs::File, KeyGenError> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    options.open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            KeyGenError::Exists(path.to_path_buf())
        } else {
            io(path)(e)
        }
    })
}

/// Makes a key of `kind` and writes `<dir>/<name>` and `<dir>/<name>.pub`.
/// `dir` is created (private to you) if it is missing. With a passphrase
/// the private key is encrypted the way `ssh-keygen` does it.
pub fn generate(
    dir: &Path,
    name: &str,
    kind: KeyKind,
    comment: &str,
    passphrase: Option<&str>,
) -> Result<NewKey, KeyGenError> {
    if !valid_file_name(name) {
        return Err(KeyGenError::BadName);
    }
    let private_path = dir.join(name);
    let public_path = dir.join(format!("{name}.pub"));
    for p in [&private_path, &public_path] {
        if p.exists() {
            return Err(KeyGenError::Exists(p.clone()));
        }
    }
    if !dir.exists() {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(dir).map_err(io(dir))?;
    }

    let crypto = |e: russh::keys::ssh_key::Error| KeyGenError::Crypto(e.to_string());
    let mut rng = rand::rng();
    let algorithm = match kind {
        KeyKind::Ed25519 => Algorithm::Ed25519,
        KeyKind::EcdsaP256 => Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP256,
        },
        KeyKind::Rsa4096 => Algorithm::Rsa { hash: None },
    };
    let mut key = match kind {
        KeyKind::Rsa4096 => {
            let pair = russh::keys::ssh_key::private::RsaKeypair::random(&mut rng, 4096)
                .map_err(crypto)?;
            PrivateKey::from(pair)
        }
        _ => PrivateKey::random(&mut rng, algorithm).map_err(crypto)?,
    };
    key.set_comment(comment);
    let public = key.public_key().clone();
    if let Some(pass) = passphrase.filter(|p| !p.is_empty()) {
        key = key.encrypt(&mut rng, pass).map_err(crypto)?;
    }
    let private_text = key
        .to_openssh(russh::keys::ssh_key::LineEnding::LF)
        .map_err(crypto)?;
    let public_line = public.to_openssh().map_err(crypto)?;

    let mut file = create_new(&private_path, 0o600)?;
    file.write_all(private_text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(io(&private_path))?;
    let mut file = create_new(&public_path, 0o644)?;
    file.write_all(format!("{public_line}\n").as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(io(&public_path))?;

    Ok(NewKey {
        private_path,
        public_path,
        fingerprint: public.fingerprint(HashAlg::Sha256).to_string(),
        public_line,
    })
}

/// A private key found in a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyFile {
    pub path: PathBuf,
    /// The file name, such as `id_ed25519`.
    pub name: String,
    pub state: KeyState,
    /// From the `.pub` file next to it, when there is one.
    pub public: Option<PublicInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicInfo {
    /// Such as `ssh-ed25519`.
    pub algorithm: String,
    pub fingerprint: String,
    pub comment: String,
    pub line: String,
}

fn public_info(private_key: &Path) -> Option<PublicInfo> {
    let mut pub_path = private_key.as_os_str().to_owned();
    pub_path.push(".pub");
    let text = std::fs::read_to_string(pub_path).ok()?;
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())?
        .trim()
        .to_string();
    let key = PublicKey::from_openssh(&line).ok()?;
    Some(PublicInfo {
        algorithm: key.algorithm().as_str().to_string(),
        fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
        comment: key.comment().to_string(),
        line,
    })
}

/// The private keys in `dir`, by name. A missing folder has none.
pub fn list(dir: &Path) -> Vec<KeyFile> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<KeyFile> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && keys::looks_like_private_key(p))
        .map(|path| KeyFile {
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            state: keys::key_state(&path),
            public: public_info(&path),
            path,
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Why a key could not be given to the agent.
#[derive(Debug, thiserror::Error)]
pub enum AgentAddError {
    #[error("no ssh-agent is running")]
    NoAgent,
    #[error("the passphrase is wrong")]
    WrongPassphrase,
    #[error("this key needs its passphrase")]
    NeedsPassphrase,
    #[error("{0}")]
    Other(String),
}

/// Unlocks the key at `path` and hands it to the running ssh-agent, so
/// connections can use it without asking again.
pub async fn add_to_agent(path: &Path, passphrase: Option<&str>) -> Result<(), AgentAddError> {
    let key = match russh::keys::load_secret_key(path, passphrase.filter(|p| !p.is_empty())) {
        Ok(k) => k,
        Err(russh::keys::Error::KeyIsEncrypted) => return Err(AgentAddError::NeedsPassphrase),
        Err(russh::keys::Error::SshKey(_) | russh::keys::Error::KeyIsCorrupt)
            if passphrase.is_some() =>
        {
            return Err(AgentAddError::WrongPassphrase)
        }
        Err(e) => return Err(AgentAddError::Other(e.to_string())),
    };
    #[cfg(unix)]
    {
        let sock = std::env::var_os("SSH_AUTH_SOCK")
            .filter(|v| !v.is_empty())
            .ok_or(AgentAddError::NoAgent)?;
        let mut agent = russh::keys::agent::client::AgentClient::connect_uds(sock)
            .await
            .map_err(|_| AgentAddError::NoAgent)?;
        agent
            .add_identity(&key, &[])
            .await
            .map_err(|e| AgentAddError::Other(e.to_string()))
    }
    #[cfg(windows)]
    {
        let mut agent = russh::keys::agent::client::AgentClient::connect_named_pipe(
            r"\\.\pipe\openssh-ssh-agent",
        )
        .await
        .map_err(|_| AgentAddError::NoAgent)?;
        agent
            .add_identity(&key, &[])
            .await
            .map_err(|e| AgentAddError::Other(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makes_an_ed25519_key_with_private_permissions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ssh = dir.path().join(".ssh");
        let key =
            generate(&ssh, "id_ed25519", KeyKind::Ed25519, "me@laptop", None).expect("generate");
        assert!(key.public_line.starts_with("ssh-ed25519 AAAA"));
        assert!(key.public_line.ends_with(" me@laptop"));
        assert!(key.fingerprint.starts_with("SHA256:"));
        assert_eq!(keys::key_state(&key.private_path), KeyState::Plain);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).expect("meta").permissions().mode() & 0o777;
            assert_eq!(mode(&key.private_path), 0o600);
            assert_eq!(mode(&ssh), 0o700);
        }
        let listed = list(&ssh);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "id_ed25519");
        assert_eq!(
            listed[0].public.as_ref().map(|p| p.fingerprint.clone()),
            Some(key.fingerprint)
        );
    }

    #[test]
    fn passphrase_encrypts_and_existing_files_are_kept() {
        let dir = tempfile::tempdir().expect("tempdir");
        let key = generate(dir.path(), "work", KeyKind::EcdsaP256, "", Some("hunter22"))
            .expect("generate");
        assert_eq!(keys::key_state(&key.private_path), KeyState::Encrypted);
        assert!(russh::keys::load_secret_key(&key.private_path, Some("hunter22")).is_ok());
        assert!(matches!(
            generate(dir.path(), "work", KeyKind::Ed25519, "", None),
            Err(KeyGenError::Exists(_))
        ));
    }

    #[tokio::test]
    async fn agent_add_checks_the_passphrase_first() {
        let dir = tempfile::tempdir().expect("tempdir");
        let key = generate(dir.path(), "k", KeyKind::Ed25519, "", Some("right")).expect("generate");
        assert!(matches!(
            add_to_agent(&key.private_path, Some("wrong")).await,
            Err(AgentAddError::WrongPassphrase)
        ));
        assert!(matches!(
            add_to_agent(&key.private_path, None).await,
            Err(AgentAddError::NeedsPassphrase)
        ));
    }

    #[test]
    fn refuses_paths_as_names() {
        let dir = tempfile::tempdir().expect("tempdir");
        for name in ["", "../x", "a/b", ".hidden", "x.pub"] {
            assert!(
                matches!(
                    generate(dir.path(), name, KeyKind::Ed25519, "", None),
                    Err(KeyGenError::BadName)
                ),
                "{name}"
            );
        }
    }
}
