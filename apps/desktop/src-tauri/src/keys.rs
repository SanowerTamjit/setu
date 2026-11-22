//! The SSH key manager: lists the keys in `~/.ssh`, makes new ones and
//! hands them to ssh-agent. Private key material never reaches the webview.

use serde::{Deserialize, Serialize};
use setu_ssh::keygen::{self, KeyKind};
use setu_ssh::keys::{self, AgentState, KeyState};
use std::collections::HashSet;
use std::path::PathBuf;

fn ssh_dir() -> PathBuf {
    crate::files::home_dir().join(".ssh")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyView {
    path: String,
    name: String,
    /// `plain`, `encrypted` or `unreadable`.
    state: &'static str,
    problem: Option<String>,
    algorithm: Option<String>,
    fingerprint: Option<String>,
    comment: Option<String>,
    public_line: Option<String>,
    in_agent: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyList {
    dir: String,
    keys: Vec<KeyView>,
    /// `none`, `unreachable` or `running`.
    agent: &'static str,
    /// Keys the agent holds that are not files here, by fingerprint.
    agent_only: Vec<String>,
}

#[tauri::command]
pub async fn ssh_keys_list() -> KeyList {
    let dir = ssh_dir();
    let files = tauri::async_runtime::spawn_blocking({
        let dir = dir.clone();
        move || keygen::list(&dir)
    })
    .await
    .unwrap_or_default();
    let (agent, held): (&'static str, Vec<keys::AgentKey>) = match keys::agent_state().await {
        AgentState::NotConfigured => ("none", Vec::new()),
        AgentState::Unreachable(_) => ("unreachable", Vec::new()),
        AgentState::Keys(k) => ("running", k),
    };
    let held_prints: HashSet<&str> = held.iter().map(|k| k.fingerprint.as_str()).collect();
    let file_prints: HashSet<String> = files
        .iter()
        .filter_map(|f| f.public.as_ref().map(|p| p.fingerprint.clone()))
        .collect();
    let keys = files
        .into_iter()
        .map(|f| {
            let (state, problem) = match f.state {
                KeyState::Plain => ("plain", None),
                KeyState::Encrypted => ("encrypted", None),
                KeyState::Unreadable(why) => ("unreadable", Some(why)),
            };
            let in_agent = f
                .public
                .as_ref()
                .is_some_and(|p| held_prints.contains(p.fingerprint.as_str()));
            KeyView {
                path: f.path.to_string_lossy().into_owned(),
                name: f.name,
                state,
                problem,
                algorithm: f.public.as_ref().map(|p| p.algorithm.clone()),
                fingerprint: f.public.as_ref().map(|p| p.fingerprint.clone()),
                comment: f.public.as_ref().map(|p| p.comment.clone()),
                public_line: f.public.map(|p| p.line),
                in_agent,
            }
        })
        .collect();
    KeyList {
        dir: dir.to_string_lossy().into_owned(),
        keys,
        agent,
        agent_only: held
            .iter()
            .filter(|k| !file_prints.contains(&k.fingerprint))
            .map(|k| {
                if k.comment.is_empty() {
                    k.fingerprint.clone()
                } else {
                    format!("{} ({})", k.comment, k.fingerprint)
                }
            })
            .collect(),
    }
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum KindArg {
    Ed25519,
    Ecdsa,
    Rsa,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedKey {
    path: String,
    fingerprint: String,
    public_line: String,
}

#[tauri::command]
pub async fn ssh_key_generate(
    name: String,
    kind: KindArg,
    comment: String,
    passphrase: Option<String>,
) -> Result<GeneratedKey, String> {
    let kind = match kind {
        KindArg::Ed25519 => KeyKind::Ed25519,
        KindArg::Ecdsa => KeyKind::EcdsaP256,
        KindArg::Rsa => KeyKind::Rsa4096,
    };
    let passphrase = passphrase.map(zeroize::Zeroizing::new);
    // RSA takes a few seconds, so keep it off the main thread.
    tauri::async_runtime::spawn_blocking(move || {
        keygen::generate(
            &ssh_dir(),
            name.trim(),
            kind,
            comment.trim(),
            passphrase.as_deref().map(String::as_str),
        )
    })
    .await
    .map_err(|e| e.to_string())?
    .map(|k| GeneratedKey {
        path: k.private_path.to_string_lossy().into_owned(),
        fingerprint: k.fingerprint,
        public_line: k.public_line,
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn ssh_key_add_to_agent(path: String, passphrase: Option<String>) -> Result<(), String> {
    let path = PathBuf::from(path);
    // Only keys in ~/.ssh: the webview cannot point this at other files.
    if path.parent() != Some(ssh_dir().as_path()) {
        return Err("only keys in your .ssh folder can be added here".into());
    }
    let passphrase = passphrase.map(zeroize::Zeroizing::new);
    keygen::add_to_agent(&path, passphrase.as_deref().map(String::as_str))
        .await
        .map_err(|e| e.to_string())
}
