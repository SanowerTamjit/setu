//! Runs the real `setu` binary against a throwaway home folder.
//!
//! The doctor tests that need OpenSSH (`ssh-keygen`, `ssh-agent`) run on
//! Unix only and are skipped with a message when the tools are missing.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn new() -> Self {
        let home = Home {
            dir: tempfile::tempdir().expect("tempdir"),
        };
        std::fs::create_dir_all(home.ssh()).expect("mkdir .ssh");
        std::fs::create_dir_all(home.config()).expect("mkdir config");
        home
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn ssh(&self) -> PathBuf {
        self.path().join(".ssh")
    }

    fn config(&self) -> PathBuf {
        self.path().join("setu-home").join("config")
    }

    fn write(&self, rel: &str, text: &str) -> PathBuf {
        let p = self.path().join(rel);
        std::fs::write(&p, text).expect("write");
        p
    }

    fn setu(&self, args: &[&str], agent: Option<&Path>) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_setu"));
        cmd.args(args)
            .env("HOME", self.path())
            .env("USERPROFILE", self.path())
            .env("SETU_HOME", self.path().join("setu-home"))
            .env("NO_COLOR", "1")
            .env_remove("SSH_AUTH_SOCK");
        if let Some(sock) = agent {
            cmd.env("SSH_AUTH_SOCK", sock);
        }
        cmd.output().expect("run setu")
    }
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn hosts_list_search_and_json() {
    let home = Home::new();
    std::fs::write(
        home.config().join("hosts.toml"),
        "version = 1\n\n[[host]]\nname = \"web-01\"\nhost = \"10.0.0.5\"\nuser = \"ops\"\nfolder = \"Prod\"\ntags = [\"nginx\"]\n\n[[host]]\nname = \"db-01\"\nhost = \"10.0.0.9\"\nport = 2200\n",
    )
    .expect("write hosts");

    let all = home.setu(&["hosts"], None);
    assert!(all.status.success());
    let text = stdout(&all);
    assert!(text.contains("web-01") && text.contains("ops@10.0.0.5"));
    assert!(text.contains("10.0.0.9:2200"));

    let some = stdout(&home.setu(&["hosts", "ls", "NGINX"], None));
    assert!(some.contains("web-01") && !some.contains("db-01"));

    let json: serde_json::Value =
        serde_json::from_slice(&home.setu(&["hosts", "list", "--json"], None).stdout)
            .expect("json");
    assert_eq!(json.as_array().map(Vec::len), Some(2));
}

#[test]
fn import_previews_then_writes() {
    let home = Home::new();
    home.write(
        ".ssh/config",
        "Host bastion\n  HostName 192.0.2.1\n  User ops\n\nHost app\n  HostName 192.0.2.7\n  ProxyJump bastion\n",
    );
    let preview = home.setu(&["hosts", "import"], None);
    assert!(preview.status.success());
    let text = stdout(&preview);
    assert!(text.contains("new") && text.contains("bastion"), "{text}");
    assert!(text.contains("Nothing saved yet"));
    assert!(!home.config().join("hosts.toml").exists());

    let write = home.setu(&["hosts", "import", "--write"], None);
    assert!(write.status.success());
    assert!(stdout(&write).contains("2 added"));
    let saved = std::fs::read_to_string(home.config().join("hosts.toml")).expect("saved");
    assert!(saved.contains("jump = \"ops@192.0.2.1\""), "{saved}");
    assert!(saved.contains("folder = \"SSH config\""));

    let again = stdout(&home.setu(&["hosts", "import"], None));
    assert!(again.contains("already saved"));
}

#[test]
fn doctor_reports_broken_hosts_file_and_fails() {
    let home = Home::new();
    std::fs::write(home.config().join("hosts.toml"), "version = 1\n[[host]\n").expect("write");
    let out = home.setu(&["doctor"], None);
    assert_eq!(out.status.code(), Some(1));
    let text = stdout(&out);
    assert!(
        text.contains("✗ The saved hosts file cannot be read"),
        "{text}"
    );
    assert!(text.contains("1 problem"));
}

#[test]
fn doctor_json_on_empty_home() {
    let home = Home::new();
    let out = home.setu(&["doctor", "--json"], None);
    assert!(out.status.success(), "{}", stdout(&out));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(json["totals"]["fail"], 0);
    let titles: Vec<&str> = json["checks"]
        .as_array()
        .expect("checks")
        .iter()
        .filter_map(|c| c["title"].as_str())
        .collect();
    assert!(titles.contains(&"No saved hosts yet"), "{titles:?}");
    assert!(titles.contains(&"No private keys found"));
}

#[cfg(unix)]
mod openssh {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn have(tool: &str) -> bool {
        Command::new(tool).arg("-?").output().is_ok()
    }

    fn keygen(path: &Path, passphrase: &str) {
        let ok = Command::new("ssh-keygen")
            .args([
                "-q", "-t", "ed25519", "-C", "test-key", "-N", passphrase, "-f",
            ])
            .arg(path)
            .status()
            .expect("ssh-keygen")
            .success();
        assert!(ok);
    }

    /// A private ssh-agent on a socket in `dir`, killed on drop.
    struct Agent {
        sock: PathBuf,
        pid: String,
    }

    impl Agent {
        fn start(dir: &Path) -> Self {
            let sock = dir.join("agent.sock");
            let out = Command::new("ssh-agent")
                .arg("-s")
                .arg("-a")
                .arg(&sock)
                .output()
                .expect("ssh-agent");
            let text = String::from_utf8_lossy(&out.stdout);
            let pid = text
                .split(';')
                .find_map(|part| part.trim().strip_prefix("SSH_AGENT_PID="))
                .expect("agent pid")
                .to_string();
            Agent { sock, pid }
        }

        fn add(&self, key: &Path) {
            let ok = Command::new("ssh-add")
                .arg(key)
                .env("SSH_AUTH_SOCK", &self.sock)
                .stderr(std::process::Stdio::null())
                .status()
                .expect("ssh-add")
                .success();
            assert!(ok);
        }
    }

    impl Drop for Agent {
        fn drop(&mut self) {
            let _ = Command::new("kill").arg(&self.pid).status();
        }
    }

    #[test]
    fn doctor_finds_key_permission_and_config_problems() {
        if !have("ssh-keygen") {
            eprintln!("skipping: ssh-keygen is not installed");
            return;
        }
        let home = Home::new();
        let plain = home.ssh().join("id_ed25519");
        keygen(&plain, "");
        let locked = home.ssh().join("id_locked");
        keygen(&locked, "a passphrase");
        let open = home.ssh().join("id_open");
        keygen(&open, "");
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        let config = home.write(
            ".ssh/config",
            "Host web\n  HostName 192.0.2.5\n  IdentityFile ~/.ssh/id_missing\n",
        );
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o666)).expect("chmod");
        std::fs::write(
            home.config().join("hosts.toml"),
            "version = 1\n[[host]]\nname = \"db\"\nhost = \"192.0.2.9\"\nauth = \"key\"\nkey_file = \"~/.ssh/nope\"\n",
        )
        .expect("hosts");

        let out = home.setu(&["doctor"], None);
        let text = stdout(&out);
        assert_eq!(out.status.code(), Some(1), "{text}");
        assert!(
            text.contains("✓ ~/.ssh/id_ed25519: usable, no passphrase"),
            "{text}"
        );
        assert!(text.contains("✓ ~/.ssh/id_locked: protected by a passphrase"));
        assert!(text.contains("✗ ~/.ssh/id_open: other users can read it"));
        assert!(text.contains("fix: chmod 600 ~/.ssh/id_open"));
        assert!(text.contains("✗ Host db: key file ~/.ssh/nope not found"));
        assert!(text.contains("Bad owner or permissions"));
        assert!(text.contains("! Host web: IdentityFile ~/.ssh/id_missing not found"));
        assert!(text.contains("! No ssh-agent: you will type key passphrases"));
        assert!(text.contains("· 1 host is not saved in Setu yet"));
    }

    #[test]
    fn doctor_lists_agent_keys_and_marks_loaded_ones() {
        if !have("ssh-keygen") || !have("ssh-agent") || !have("ssh-add") {
            eprintln!("skipping: OpenSSH client tools are not installed");
            return;
        }
        let home = Home::new();
        let key = home.ssh().join("id_ed25519");
        keygen(&key, "");
        let agent = Agent::start(home.path());

        let empty = stdout(&home.setu(&["doctor"], Some(&agent.sock)));
        assert!(
            empty.contains("! ssh-agent is running but holds no keys"),
            "{empty}"
        );

        agent.add(&key);
        let loaded = stdout(&home.setu(&["doctor"], Some(&agent.sock)));
        assert!(loaded.contains("✓ ssh-agent holds 1 key"), "{loaded}");
        assert!(loaded.contains("ssh-ed25519 SHA256:"));
        assert!(loaded.contains("test-key"));
        assert!(loaded.contains("usable, no passphrase, loaded in ssh-agent"));

        let gone = home.path().join("gone.sock");
        let dead = home.setu(&["doctor"], Some(&gone));
        assert_eq!(dead.status.code(), Some(1));
        assert!(stdout(&dead).contains("✗ An ssh-agent is configured but does not answer"));
    }
}
