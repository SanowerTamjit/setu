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
    /// Ed25519 on a FIDO2/U2F security key such as a YubiKey. Made by
    /// OpenSSH's `ssh-keygen`, which talks to the device.
    Ed25519Sk,
    /// ECDSA P-256 on a security key, for keys that cannot do Ed25519.
    EcdsaSk,
}

impl KeyKind {
    /// The usual file name for this kind, as `ssh-keygen` picks it.
    pub fn default_file_name(self) -> &'static str {
        match self {
            KeyKind::Ed25519 => "id_ed25519",
            KeyKind::EcdsaP256 => "id_ecdsa",
            KeyKind::Rsa4096 => "id_rsa",
            KeyKind::Ed25519Sk => "id_ed25519_sk",
            KeyKind::EcdsaSk => "id_ecdsa_sk",
        }
    }

    /// Whether the key lives on a hardware security key.
    pub fn is_security_key(self) -> bool {
        matches!(self, KeyKind::Ed25519Sk | KeyKind::EcdsaSk)
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
    #[error("{0}")]
    SecurityKey(String),
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
    if kind.is_security_key() {
        return generate_security_key(
            dir,
            name,
            kind,
            comment,
            passphrase,
            &SecurityKeyOptions::default(),
        );
    }
    let (private_path, public_path) = prepare(dir, name)?;

    let crypto = |e: russh::keys::ssh_key::Error| KeyGenError::Crypto(e.to_string());
    let mut rng = rand::rng();
    let algorithm = match kind {
        KeyKind::Ed25519 => Algorithm::Ed25519,
        KeyKind::EcdsaP256 => Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP256,
        },
        KeyKind::Rsa4096 => Algorithm::Rsa { hash: None },
        KeyKind::Ed25519Sk | KeyKind::EcdsaSk => unreachable!("made by ssh-keygen above"),
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

/// Checks `name`, makes sure neither file exists, and creates `dir`
/// (private to you) if it is missing. Returns the private and public paths.
fn prepare(dir: &Path, name: &str) -> Result<(PathBuf, PathBuf), KeyGenError> {
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
    ensure_dir(dir)?;
    Ok((private_path, public_path))
}

/// Creates `dir`, private to you, if it is missing.
fn ensure_dir(dir: &Path) -> Result<(), KeyGenError> {
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
    Ok(())
}

/// Choices for a key made on a security key.
#[derive(Debug, Clone, Default)]
pub struct SecurityKeyOptions {
    /// Store the key on the device too (`-O resident`), so it can be loaded
    /// onto another computer with [`load_resident_keys`]. Needs a FIDO2 key
    /// with a PIN.
    pub resident: bool,
    /// Ask for the device PIN (or fingerprint) on every use, not only a touch
    /// (`-O verify-required`). The server must allow it.
    pub verify_required: bool,
    /// The device PIN, for keys that need it while the key is made.
    pub pin: Option<zeroize::Zeroizing<String>>,
}

/// The `ssh-keygen` to run: the one on `PATH`, or Windows' built-in OpenSSH.
fn ssh_keygen() -> std::process::Command {
    #[cfg(windows)]
    {
        let builtin = Path::new(r"C:\Windows\System32\OpenSSH\ssh-keygen.exe");
        if builtin.is_file() {
            return std::process::Command::new(builtin);
        }
    }
    std::process::Command::new("ssh-keygen")
}

/// A private folder holding an askpass script that prints `pin`, so
/// `ssh-keygen` can get the PIN without a terminal. Unix only: on Windows,
/// OpenSSH asks for the PIN and touch in the system's own security key dialog.
struct PinAskpass {
    dir: tempfile::TempDir,
}

impl PinAskpass {
    #[cfg(unix)]
    fn new() -> Result<Self, KeyGenError> {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::Builder::new()
            .prefix("setu-askpass")
            .tempdir()
            .map_err(|e| {
                KeyGenError::SecurityKey(format!("could not make a temporary folder: {e}"))
            })?;
        let script = dir.path().join("askpass");
        std::fs::write(&script, "#!/bin/sh\nprintf '%s\\n' \"$SETU_SK_PIN\"\n")
            .and_then(|()| {
                std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
            })
            .map_err(io(&script))?;
        Ok(Self { dir })
    }

    fn apply(&self, cmd: &mut std::process::Command, pin: &str) {
        cmd.env("SSH_ASKPASS", self.dir.path().join("askpass"))
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("SETU_SK_PIN", pin);
    }
}

/// Runs `cmd` (an `ssh-keygen` call that talks to the security key) with the
/// PIN, if any, and turns its failure into a sentence for people.
fn run_with_device(mut cmd: std::process::Command, pin: Option<&str>) -> Result<(), KeyGenError> {
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(unix)]
    let askpass = match pin.filter(|p| !p.is_empty()) {
        Some(pin) => {
            let a = PinAskpass::new()?;
            a.apply(&mut cmd, pin);
            Some(a)
        }
        None => None,
    };
    #[cfg(not(unix))]
    let _ = pin;
    let out = cmd.output().map_err(|e| {
        KeyGenError::SecurityKey(if e.kind() == std::io::ErrorKind::NotFound {
            "security keys are made with OpenSSH's ssh-keygen, which is not installed. Install OpenSSH 8.2 or newer".into()
        } else {
            format!("could not run ssh-keygen: {e}")
        })
    })?;
    #[cfg(unix)]
    drop(askpass);
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(KeyGenError::SecurityKey(explain_device_failure(&stderr)))
}

/// Turns what `ssh-keygen` printed when talking to a security key failed
/// into one plain sentence.
pub fn explain_device_failure(stderr: &str) -> String {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("device not found") || lower.contains("no device") {
        "No security key found. Plug in your YubiKey or other FIDO2 key and try again.".into()
    } else if lower.contains("pin incorrect")
        || lower.contains("invalid pin")
        || lower.contains("wrong pin")
    {
        "The security key's PIN is wrong. After several wrong tries the key locks, so check it before trying again.".into()
    } else if lower.contains("pin required") || lower.contains("pin not set") {
        "This needs the security key's PIN. Enter it and try again (a key without a PIN cannot store keys on the device).".into()
    } else if lower.contains("unknown key type")
        || lower.contains("not supported") && lower.contains("-sk")
    {
        "This version of OpenSSH cannot make security keys. Install OpenSSH 8.2 or newer.".into()
    } else if lower.contains("enrollment failed: invalid format")
        || lower.contains("no fido sk provider")
        || lower.contains("built without")
    {
        // macOS's own OpenSSH is built without security key support.
        "This OpenSSH cannot make security keys (it was built without FIDO support, as macOS's own is). Install OpenSSH from Homebrew (brew install openssh) and try again.".into()
    } else if lower.contains("timeout")
        || lower.contains("action timeout")
        || lower.contains("user presence")
    {
        "The security key was not touched in time. Try again and touch it when it blinks.".into()
    } else {
        let last = stderr
            .lines()
            .map(str::trim)
            .rfind(|l| !l.is_empty() && !l.starts_with("You may need to touch"))
            .unwrap_or("ssh-keygen failed without saying why");
        format!("The security key did not make the key: {last}")
    }
}

/// Makes a key on a FIDO2/U2F security key with `ssh-keygen -t
/// ed25519-sk` (or `ecdsa-sk`). The device must be plugged in and touched
/// when it blinks. The files written hold only a handle to the key on the
/// device, so they are useless without it; a passphrase still protects them.
pub fn generate_security_key(
    dir: &Path,
    name: &str,
    kind: KeyKind,
    comment: &str,
    passphrase: Option<&str>,
    options: &SecurityKeyOptions,
) -> Result<NewKey, KeyGenError> {
    let key_type = match kind {
        KeyKind::Ed25519Sk => "ed25519-sk",
        KeyKind::EcdsaSk => "ecdsa-sk",
        _ => return Err(KeyGenError::SecurityKey("not a security key type".into())),
    };
    let (private_path, public_path) = prepare(dir, name)?;
    let mut cmd = ssh_keygen();
    cmd.arg("-q")
        .args(["-t", key_type])
        .arg("-f")
        .arg(&private_path)
        .args(["-N", "", "-C", comment]);
    if options.resident {
        cmd.args(["-O", "resident"]);
    }
    if options.verify_required {
        cmd.args(["-O", "verify-required"]);
    }
    let result = run_with_device(cmd, options.pin.as_deref().map(String::as_str));
    if let Err(e) = result {
        // ssh-keygen writes nothing on failure, but be sure no half key is left.
        for p in [&private_path, &public_path] {
            let _ = std::fs::remove_file(p);
        }
        return Err(e);
    }
    finish_security_key(&private_path, &public_path, passphrase)
}

/// Encrypts the key `ssh-keygen` wrote (it was made without a passphrase so
/// none had to pass on its command line) and reads its public half.
fn finish_security_key(
    private_path: &Path,
    public_path: &Path,
    passphrase: Option<&str>,
) -> Result<NewKey, KeyGenError> {
    let crypto = |e: russh::keys::ssh_key::Error| KeyGenError::Crypto(e.to_string());
    let text = std::fs::read_to_string(private_path).map_err(io(private_path))?;
    let key = PrivateKey::from_openssh(&text).map_err(crypto)?;
    let public = key.public_key().clone();
    if let Some(pass) = passphrase.filter(|p| !p.is_empty()) {
        let encrypted = key.encrypt(&mut rand::rng(), pass).map_err(crypto)?;
        let text = encrypted
            .to_openssh(russh::keys::ssh_key::LineEnding::LF)
            .map_err(crypto)?;
        // Write beside it and rename, so a crash never leaves a broken key.
        let tmp = private_path.with_extension("setu-tmp");
        let _ = std::fs::remove_file(&tmp);
        let mut file = create_new(&tmp, 0o600)?;
        file.write_all(text.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(io(&tmp))?;
        std::fs::rename(&tmp, private_path).map_err(io(private_path))?;
    }
    let public_line = public.to_openssh().map_err(crypto)?;
    Ok(NewKey {
        private_path: private_path.to_path_buf(),
        public_path: public_path.to_path_buf(),
        fingerprint: public.fingerprint(HashAlg::Sha256).to_string(),
        public_line,
    })
}

/// Copies the keys stored on the plugged-in security key (made with
/// "store on the key") into `dir`, like `ssh-keygen -K`. Needs the PIN.
/// Names already taken get a number added; nothing is overwritten. Returns
/// the private key paths written.
pub fn load_resident_keys(dir: &Path, pin: Option<&str>) -> Result<Vec<PathBuf>, KeyGenError> {
    let work = tempfile::Builder::new()
        .prefix("setu-resident")
        .tempdir()
        .map_err(|e| KeyGenError::SecurityKey(format!("could not make a temporary folder: {e}")))?;
    let mut cmd = ssh_keygen();
    cmd.arg("-K").current_dir(work.path());
    run_with_device(cmd, pin)?;
    ensure_dir(dir)?;
    let mut written = Vec::new();
    let mut names: Vec<String> = std::fs::read_dir(work.path())
        .map_err(io(work.path()))?
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.ends_with(".pub"))
        .collect();
    names.sort();
    for name in names {
        let from = work.path().join(&name);
        let pub_from = work.path().join(format!("{name}.pub"));
        let mut target = name.clone();
        let mut n = 2;
        while dir.join(&target).exists() || dir.join(format!("{target}.pub")).exists() {
            if same_file_contents(&from, &dir.join(&target)) {
                break;
            }
            target = format!("{name}_{n}");
            n += 1;
        }
        let to = dir.join(&target);
        if to.exists() {
            continue; // already here, unchanged
        }
        copy_new(&from, &to, 0o600)?;
        if pub_from.is_file() {
            copy_new(&pub_from, &dir.join(format!("{target}.pub")), 0o644)?;
        }
        written.push(to);
    }
    Ok(written)
}

fn same_file_contents(a: &Path, b: &Path) -> bool {
    matches!((std::fs::read(a), std::fs::read(b)), (Ok(x), Ok(y)) if x == y)
}

fn copy_new(from: &Path, to: &Path, mode: u32) -> Result<(), KeyGenError> {
    let data = std::fs::read(from).map_err(io(from))?;
    let mut file = create_new(to, mode)?;
    file.write_all(&data)
        .and_then(|()| file.sync_all())
        .map_err(io(to))
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

    /// A key file like `ssh-keygen -t ed25519-sk` writes, without a device.
    fn fake_security_key(dir: &Path, name: &str) -> (PathBuf, PathBuf) {
        use russh::keys::ssh_key::{private, public, LineEnding};
        let real = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("key");
        let point = *real.public_key().key_data().ed25519().expect("ed25519");
        let sk = private::SkEd25519::new(public::SkEd25519::new(point, "ssh:"), 1, vec![9u8; 48])
            .expect("sk");
        let key = PrivateKey::new(private::KeypairData::from(sk), "yubikey").expect("key");
        let private_path = dir.join(name);
        let public_path = dir.join(format!("{name}.pub"));
        std::fs::write(
            &private_path,
            key.to_openssh(LineEnding::LF).expect("encode").as_bytes(),
        )
        .expect("write");
        std::fs::write(
            &public_path,
            format!("{}\n", key.public_key().to_openssh().expect("pub")),
        )
        .expect("write");
        (private_path, public_path)
    }

    #[test]
    fn security_key_files_are_listed_and_can_get_a_passphrase() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (private_path, public_path) = fake_security_key(dir.path(), "id_ed25519_sk");
        let listed = list(dir.path());
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].state, KeyState::Plain);
        let public = listed[0].public.as_ref().expect("public");
        assert_eq!(public.algorithm, "sk-ssh-ed25519@openssh.com");

        let made = finish_security_key(&private_path, &public_path, Some("pw")).expect("finish");
        assert_eq!(made.fingerprint, public.fingerprint);
        assert!(made
            .public_line
            .starts_with("sk-ssh-ed25519@openssh.com AAAA"));
        assert_eq!(keys::key_state(&private_path), KeyState::Encrypted);
        let key = russh::keys::load_secret_key(&private_path, Some("pw")).expect("decrypt");
        assert!(keys::is_security_key(key.algorithm()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&private_path)
                .expect("meta")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn device_failures_read_plainly() {
        let cases = [
            ("Key enrollment failed: device not found\n", "No security key found"),
            ("Key enrollment failed: invalid PIN\n", "PIN is wrong"),
            ("Key enrollment failed: PIN required\n", "needs the security key's PIN"),
            ("unknown key type ed25519-sk\n", "cannot make security keys"),
            ("Key enrollment failed: invalid format\n", "Homebrew"),
            (
                "You may need to touch your authenticator to authorize key generation.\nsomething odd\n",
                "something odd",
            ),
        ];
        for (stderr, expected) in cases {
            let text = explain_device_failure(stderr);
            assert!(text.contains(expected), "{stderr:?} gave {text:?}");
        }
    }

    /// Runs the real ssh-keygen, which must find no device. Only on CI, so a
    /// developer's plugged-in key does not start blinking during tests.
    #[test]
    fn no_device_gives_a_clear_error_and_no_files() {
        if std::env::var_os("CI").is_none()
            || std::process::Command::new("ssh-keygen")
                .arg("-?")
                .output()
                .is_err()
        {
            eprintln!("skipping: only on CI with ssh-keygen");
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        match generate(dir.path(), "id_ed25519_sk", KeyKind::Ed25519Sk, "", None) {
            Err(KeyGenError::SecurityKey(text)) => {
                assert!(
                    text.contains("No security key found") || text.contains("cannot make"),
                    "{text}"
                )
            }
            other => panic!("expected a security key error, got {other:?}"),
        }
        assert!(list(dir.path()).is_empty());
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
