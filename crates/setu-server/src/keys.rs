//! `~/.ssh/authorized_keys` of the login user: reading it, adding a public
//! key and removing one. The file is read and written over SFTP by the
//! caller; this module only parses and builds text, so nothing typed or
//! read here ever reaches a shell.
//!
//! A key to add must be one well-formed public key line (a known type, a
//! blob whose inside names the same type, an optional plain comment, no
//! options). Removing names a line by its number *and* its key, so a file
//! that changed meanwhile is never edited at the wrong line.

use crate::field;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use base64::Engine;
use serde::Serialize;
use sha2::{Digest, Sha256};

/// Largest `authorized_keys` Setu reads or writes.
pub const MAX_FILE: u64 = 256 * 1024;
/// Most keys listed.
pub const MAX_KEYS: usize = 500;
/// Longest key line accepted when adding (RSA 16384 fits).
const MAX_LINE: usize = 16 * 1024;
const MAX_COMMENT: usize = 200;

/// Key types OpenSSH accepts in `authorized_keys`.
const TYPES: &[&str] = &[
    "ssh-ed25519",
    "ssh-rsa",
    "ecdsa-sha2-nistp256",
    "ecdsa-sha2-nistp384",
    "ecdsa-sha2-nistp521",
    "sk-ssh-ed25519@openssh.com",
    "sk-ecdsa-sha2-nistp256@openssh.com",
    "ssh-dss",
];

/// One key in the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizedKey {
    /// The line it is on, from 1.
    pub line: usize,
    pub kind: String,
    /// `SHA256:…`, as `ssh-keygen -l` prints it.
    pub fingerprint: String,
    pub comment: String,
    /// Options before the key (`from="…"`, `command="…"`), cleaned and cut.
    pub options: String,
    /// The key itself (base64), to name it when removing.
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyList {
    pub path: String,
    pub keys: Vec<AuthorizedKey>,
    /// Lines that are neither keys, comments nor blank.
    pub unreadable: usize,
    /// The file does not exist yet.
    pub missing: bool,
}

/// `SHA256:` and the unpadded base64 of the key's hash.
pub fn fingerprint(blob: &[u8]) -> String {
    format!("SHA256:{}", STANDARD_NO_PAD.encode(Sha256::digest(blob)))
}

/// The type named inside a key blob (its first length-prefixed string).
fn inner_type(blob: &[u8]) -> Option<&str> {
    let len = u32::from_be_bytes(blob.get(..4)?.try_into().ok()?) as usize;
    std::str::from_utf8(blob.get(4..4 + len)?).ok()
}

/// Splits a line into options, type, key and comment. Options may hold
/// quoted spaces, so the type is found as the first word that is one.
fn split(line: &str) -> Option<(&str, &str, &str, &str)> {
    let mut in_quote = false;
    let mut start = 0;
    let bytes = line.as_bytes();
    let mut i = 0;
    while i <= bytes.len() {
        let end = i == bytes.len() || (!in_quote && (bytes[i] == b' ' || bytes[i] == b'\t'));
        if i < bytes.len() && bytes[i] == b'"' {
            in_quote = !in_quote;
        }
        if end {
            let word = &line[start..i];
            if TYPES.contains(&word) {
                let options = line[..start].trim();
                let rest = line[i..].trim_start();
                let (key, comment) = rest.split_once([' ', '\t']).unwrap_or((rest, ""));
                return Some((options, word, key, comment.trim()));
            }
            start = i + 1;
        }
        i += 1;
    }
    None
}

/// Reads the file's text. Comments and blank lines are skipped.
pub fn parse(path: &str, text: &str) -> KeyList {
    let mut keys = Vec::new();
    let mut unreadable = 0;
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((options, kind, key, comment)) = split(line) else {
            unreadable += 1;
            continue;
        };
        let Ok(blob) = STANDARD.decode(key) else {
            unreadable += 1;
            continue;
        };
        if keys.len() >= MAX_KEYS {
            break;
        }
        keys.push(AuthorizedKey {
            line: n + 1,
            kind: kind.to_string(),
            fingerprint: fingerprint(&blob),
            comment: field(comment, MAX_COMMENT),
            options: field(options, 120),
            key: key.to_string(),
        });
    }
    KeyList {
        path: path.to_string(),
        keys,
        unreadable,
        missing: false,
    }
}

/// A public key someone wants to add, checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewKey {
    pub kind: String,
    pub fingerprint: String,
    pub comment: String,
    /// The line that will be written.
    pub line: String,
}

/// Checks a pasted public key: one line, `type base64 [comment]`, the
/// blob decodes and names the same type, the comment is plain text.
pub fn check_new_key(text: &str) -> Result<NewKey, String> {
    let text = text.trim();
    let bad = |why: &str| {
        Err(format!("This is not an SSH public key: {why}. Paste one line like \"ssh-ed25519 AAAA… you@laptop\" (the .pub file)."))
    };
    if text.is_empty() {
        return bad("nothing was pasted");
    }
    if text.len() > MAX_LINE || text.contains(['\n', '\r']) {
        return bad("it must be a single line");
    }
    if text.contains("PRIVATE KEY") {
        return Err("That is a PRIVATE key. Never paste it anywhere: paste the public key (the .pub file) instead.".into());
    }
    let mut words = text.splitn(3, [' ', '\t']);
    let (Some(kind), Some(key)) = (words.next(), words.next()) else {
        return bad("it needs a key type and the key");
    };
    if !TYPES.contains(&kind) {
        return bad("the key type is not one OpenSSH knows, or options come first (Setu adds keys without options)");
    }
    let Ok(blob) = STANDARD.decode(key) else {
        return bad("the key part is not valid base64");
    };
    if inner_type(&blob) != Some(kind) {
        return bad("the key does not match its type");
    }
    let comment_raw = words.next().unwrap_or("").trim();
    if comment_raw.chars().any(|c| c.is_control()) {
        return bad("the comment has control characters");
    }
    let comment = field(comment_raw, MAX_COMMENT);
    let line = if comment.is_empty() {
        format!("{kind} {key}")
    } else {
        format!("{kind} {key} {comment}")
    };
    Ok(NewKey {
        kind: kind.to_string(),
        fingerprint: fingerprint(&blob),
        comment,
        line,
    })
}

/// The file with `key` added at the end. Refuses a key that is already
/// there.
pub fn with_key(text: &str, key: &NewKey) -> Result<String, String> {
    let blob = key.line.split(' ').nth(1).unwrap_or_default();
    if parse("", text).keys.iter().any(|k| k.key == blob) {
        return Err("This key is already in authorized_keys.".into());
    }
    let mut out = text.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&key.line);
    out.push('\n');
    Ok(out)
}

/// The file without the key on `line`, which must still hold `key`. Every
/// other line is kept exactly as it was.
pub fn without_key(text: &str, line: usize, key: &str) -> Result<String, String> {
    let current = parse("", text);
    if !current.keys.iter().any(|k| k.line == line && k.key == key) {
        return Err(
            "authorized_keys changed since it was read. Read it again and try once more.".into(),
        );
    }
    let mut out = String::with_capacity(text.len());
    for (n, raw) in text.split_inclusive('\n').enumerate() {
        if n + 1 != line {
            out.push_str(raw);
        }
    }
    Ok(out)
}

/// What a change does, in words, for the confirmation, the policy and the
/// audit trail. Public keys are not secret.
pub fn describe(adding: bool, path: &str, kind: &str, fingerprint: &str, comment: &str) -> String {
    let what = if comment.is_empty() {
        format!("{kind} {fingerprint}")
    } else {
        format!("{kind} {fingerprint} ({comment})")
    };
    if adding {
        format!("add to {path}: {what}")
    } else {
        format!("remove from {path}: {what}")
    }
}

/// The mode a rewritten file gets: the old one without group or other
/// write (sshd's `StrictModes` refuses those) and always readable and
/// writable by its owner; `0600` for a new file.
pub fn keep_mode(old: Option<u32>) -> u32 {
    old.map_or(0o600, |m| (m & 0o644) | 0o600)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real-shaped ed25519 public key blob (32 zero bytes as the key).
    fn ed25519(seed: u8) -> String {
        let mut blob = Vec::new();
        blob.extend_from_slice(&11u32.to_be_bytes());
        blob.extend_from_slice(b"ssh-ed25519");
        blob.extend_from_slice(&32u32.to_be_bytes());
        blob.extend_from_slice(&[seed; 32]);
        STANDARD.encode(blob)
    }

    #[test]
    fn the_file_is_read() {
        let a = ed25519(1);
        let b = ed25519(2);
        let text = format!(
            "# admins\nssh-ed25519 {a} alice@laptop\n\nfrom=\"10.0.0.0/8\",command=\"echo hi there\" ssh-ed25519 {b} backup bot\nnot a key\nssh-rsa !!!notbase64\n"
        );
        let list = parse("/home/ops/.ssh/authorized_keys", &text);
        assert_eq!(list.keys.len(), 2);
        assert_eq!(list.unreadable, 2);
        assert_eq!(
            (list.keys[0].line, list.keys[0].comment.as_str()),
            (2, "alice@laptop")
        );
        assert!(list.keys[0].fingerprint.starts_with("SHA256:"));
        assert_ne!(list.keys[0].fingerprint, list.keys[1].fingerprint);
        assert_eq!(list.keys[1].line, 4);
        assert_eq!(
            list.keys[1].options,
            "from=\"10.0.0.0/8\",command=\"echo hi there\""
        );
        assert_eq!(list.keys[1].comment, "backup bot");
    }

    #[test]
    fn fingerprints_match_ssh_keygen() {
        // ssh-keygen -l of the key with 32 bytes of 0x01.
        let blob = STANDARD.decode(ed25519(1)).unwrap_or_default();
        let expected = format!("SHA256:{}", STANDARD_NO_PAD.encode(Sha256::digest(&blob)));
        assert_eq!(fingerprint(&blob), expected);
        assert!(!fingerprint(&blob).ends_with('='));
    }

    #[test]
    fn new_keys_are_checked() {
        let a = ed25519(1);
        let k = check_new_key(&format!("  ssh-ed25519 {a} me@laptop  \n"))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(k.line, format!("ssh-ed25519 {a} me@laptop"));
        assert_eq!(k.comment, "me@laptop");
        let bare = check_new_key(&format!("ssh-ed25519 {a}")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(bare.line, format!("ssh-ed25519 {a}"));
        for bad in [
            String::new(),
            format!("ssh-ed25519 {a} x\nssh-ed25519 {a} y"),
            format!("ssh-rsa {a} wrong type inside"),
            format!("command=\"rm -rf /\" ssh-ed25519 {a}"),
            "ssh-ed25519 not*base64".to_string(),
            format!("ssh-foo {a}"),
            format!("ssh-ed25519 {a} bell\u{7}"),
        ] {
            assert!(check_new_key(&bad).is_err(), "{bad:?}");
        }
        assert!(check_new_key("-----BEGIN OPENSSH PRIVATE KEY-----")
            .is_err_and(|e| e.contains("PRIVATE")));
    }

    #[test]
    fn adding_appends_once() {
        let a = ed25519(1);
        let b = ed25519(2);
        let key = check_new_key(&format!("ssh-ed25519 {b} new")).unwrap_or_else(|e| panic!("{e}"));
        let text = format!("ssh-ed25519 {a} old");
        assert_eq!(
            with_key(&text, &key),
            Ok(format!("ssh-ed25519 {a} old\nssh-ed25519 {b} new\n"))
        );
        assert_eq!(with_key("", &key), Ok(format!("ssh-ed25519 {b} new\n")));
        let twice = with_key(&with_key(&text, &key).unwrap_or_default(), &key);
        assert!(twice.is_err_and(|e| e.contains("already")));
    }

    #[test]
    fn removing_checks_the_line_still_holds_the_key() {
        let a = ed25519(1);
        let b = ed25519(2);
        let text = format!("# keep me\nssh-ed25519 {a} a\nssh-ed25519 {b} b\n");
        assert_eq!(
            without_key(&text, 2, &a),
            Ok(format!("# keep me\nssh-ed25519 {b} b\n"))
        );
        // Wrong line for that key, or a line that moved.
        assert!(without_key(&text, 3, &a).is_err());
        assert!(without_key(&text, 1, &a).is_err());
        // No trailing newline on the last line.
        let text = format!("ssh-ed25519 {a} a\nssh-ed25519 {b} b");
        assert_eq!(
            without_key(&text, 2, &b),
            Ok(format!("ssh-ed25519 {a} a\n"))
        );
    }

    #[test]
    fn changes_are_described() {
        assert_eq!(
            describe(
                true,
                "~/.ssh/authorized_keys",
                "ssh-ed25519",
                "SHA256:x",
                "me"
            ),
            "add to ~/.ssh/authorized_keys: ssh-ed25519 SHA256:x (me)"
        );
        assert_eq!(
            describe(false, "p", "ssh-rsa", "SHA256:y", ""),
            "remove from p: ssh-rsa SHA256:y"
        );
    }

    #[test]
    fn rewritten_files_stay_strict() {
        assert_eq!(keep_mode(None), 0o600);
        assert_eq!(keep_mode(Some(0o644)), 0o644);
        assert_eq!(keep_mode(Some(0o666)), 0o644);
        assert_eq!(keep_mode(Some(0o4777)), 0o644);
        assert_eq!(keep_mode(Some(0o400)), 0o600);
    }
}
