//! Integration tests against a real OpenSSH server.
//!
//! Each test starts its own `sshd` on a free localhost port with throwaway
//! keys. Tests are skipped (with a message) when `sshd` or `ssh-keygen` is not
//! installed, for example on Windows CI.

use setu_ssh::sftp::{EntryKind, TransferControl};
use setu_ssh::{connect, known_hosts, Auth, ConnectOptions, ShellSize, SshError, Zeroizing};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct Sshd {
    child: Child,
    dir: tempfile::TempDir,
    port: u16,
    user: String,
}

impl Drop for Sshd {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn find(candidates: &[&str]) -> Option<PathBuf> {
    candidates.iter().map(PathBuf::from).find(|p| p.is_file())
}

fn keygen(path: &Path, passphrase: &str) {
    let status = Command::new("ssh-keygen")
        .args([
            "-q",
            "-t",
            "ed25519",
            "-N",
            passphrase,
            "-C",
            "setu-test",
            "-f",
        ])
        .arg(path)
        .status()
        .expect("run ssh-keygen");
    assert!(status.success(), "ssh-keygen failed");
}

/// Writes an `sk-ssh-ed25519@openssh.com` key file like one `ssh-keygen -t
/// ed25519-sk` makes, with a made-up key handle, and returns its public line.
fn fake_security_key(path: &Path) -> String {
    use russh::keys::ssh_key::{private, public, LineEnding, PrivateKey};
    let real = PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519).expect("key");
    let point = *real.public_key().key_data().ed25519().expect("ed25519");
    let sk = private::SkEd25519::new(public::SkEd25519::new(point, "ssh:"), 0x01, vec![7u8; 64])
        .expect("sk key");
    let key = PrivateKey::new(private::KeypairData::from(sk), "setu-sk-test").expect("private key");
    std::fs::write(
        path,
        key.to_openssh(LineEnding::LF).expect("encode").as_bytes(),
    )
    .expect("write");
    let line = format!("{}\n", key.public_key().to_openssh().expect("public"));
    std::fs::write(format!("{}.pub", path.display()), &line).expect("write pub");
    line
}

/// A free localhost port that no other test in this run was given. The
/// listener is closed before sshd binds the port, so without this two tests
/// running in parallel could get the same port: the second sshd fails to
/// bind, its readiness check reaches the first test's sshd, and its first
/// connection is refused once that test ends.
fn free_port() -> u16 {
    static GIVEN: std::sync::Mutex<Vec<u16>> = std::sync::Mutex::new(Vec::new());
    loop {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .expect("free port");
        let mut given = GIVEN.lock().unwrap_or_else(|e| e.into_inner());
        if !given.contains(&port) {
            given.push(port);
            return port;
        }
    }
}

fn current_user() -> String {
    std::env::var("USER")
        .ok()
        .filter(|u| !u.is_empty())
        .or_else(|| {
            Command::new("whoami")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        })
        .expect("current user")
}

/// Whether CI requires these tests to run (set `SETU_REQUIRE_SSHD=1`).
fn required() -> bool {
    std::env::var("SETU_REQUIRE_SSHD").is_ok_and(|v| v == "1")
}

/// Skips the test with a message, or fails it when CI requires sshd.
fn skip(reason: &str) -> Option<Sshd> {
    assert!(!required(), "SETU_REQUIRE_SSHD=1 but {reason}");
    eprintln!("skipping: {reason}");
    None
}

/// Starts sshd, or returns None (skip) when OpenSSH is not available.
fn start_sshd() -> Option<Sshd> {
    if cfg!(windows) {
        return skip("sshd tests do not run on Windows");
    }
    let Some(sshd) = find(&["/usr/sbin/sshd", "/usr/bin/sshd", "/usr/local/sbin/sshd"]) else {
        return skip("sshd is not installed");
    };
    if Command::new("ssh-keygen").arg("-?").output().is_err() {
        return skip("ssh-keygen is not installed");
    }
    // Running as root needs the privilege separation directory.
    let _ = std::fs::create_dir_all("/run/sshd");

    let dir = tempfile::tempdir().expect("tempdir");
    let d = dir.path();
    keygen(&d.join("host_key"), "");
    keygen(&d.join("user_key"), "");
    keygen(&d.join("encrypted_key"), "s3cret");
    keygen(&d.join("other_key"), "");
    let authorized = [
        std::fs::read_to_string(d.join("user_key.pub")).expect("pub"),
        std::fs::read_to_string(d.join("encrypted_key.pub")).expect("pub"),
    ]
    .concat();
    std::fs::write(d.join("authorized_keys"), authorized).expect("authorized_keys");

    let port = free_port();
    let config = format!(
        "Port {port}\nListenAddress 127.0.0.1\nHostKey {host}\nAuthorizedKeysFile {auth}\n\
         PidFile {pid}\nStrictModes no\nUsePAM no\nPasswordAuthentication no\n\
         KbdInteractiveAuthentication no\nPubkeyAuthentication yes\nPermitRootLogin yes\n\
         AllowTcpForwarding yes\nSubsystem sftp internal-sftp\nLogLevel ERROR\n",
        host = d.join("host_key").display(),
        auth = d.join("authorized_keys").display(),
        pid = d.join("sshd.pid").display(),
    );
    std::fs::write(d.join("sshd_config"), config).expect("config");

    let mut child = Command::new(sshd)
        .arg("-D")
        .arg("-e")
        .arg("-f")
        .arg(d.join("sshd_config"))
        .stdin(Stdio::null())
        .spawn()
        .expect("start sshd");

    // sshd may refuse to run unprivileged on some systems (for example macOS).
    let deadline = Instant::now() + Duration::from_secs(10);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        if Instant::now() >= deadline || matches!(child.try_wait(), Ok(Some(_))) {
            let _ = child.kill();
            let _ = child.wait();
            return skip("sshd could not be started here");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // The port answered: make sure it is our sshd and not one that exited
    // after failing to bind (the port then belongs to someone else).
    if matches!(child.try_wait(), Ok(Some(_))) {
        panic!(
            "sshd exited after its port started answering; port {port} is in use by something else"
        );
    }
    Some(Sshd {
        child,
        dir,
        port,
        user: current_user(),
    })
}

impl Sshd {
    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn opts(&self) -> ConnectOptions {
        ConnectOptions::new("127.0.0.1", self.user.clone())
            .port(self.port)
            .known_hosts(self.path("known_hosts"))
            .auth(Auth::KeyFile {
                path: self.path("user_key"),
                passphrase: None,
            })
    }

    /// The host key fingerprint as ssh-keygen prints it.
    fn fingerprint(&self) -> String {
        let out = Command::new("ssh-keygen")
            .arg("-lf")
            .arg(self.path("host_key.pub"))
            .output()
            .expect("ssh-keygen -l");
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .nth(1)
            .expect("fingerprint")
            .to_string()
    }

    /// Accepts the host key, like a user clicking "Trust".
    async fn trust(&self) {
        match connect(&self.opts()).await {
            Err(SshError::UnknownHostKey { key }) => {
                known_hosts::trust(&self.path("known_hosts"), &key).expect("trust")
            }
            other => panic!("expected an unknown host key, got {other:?}"),
        }
    }
}

macro_rules! sshd_or_skip {
    () => {
        match start_sshd() {
            Some(s) => s,
            None => return,
        }
    };
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_host_key_is_refused_with_its_fingerprint() {
    let sshd = sshd_or_skip!();
    match connect(&sshd.opts()).await {
        Err(SshError::UnknownHostKey { key }) => {
            assert_eq!(key.fingerprint, sshd.fingerprint());
            assert_eq!(key.algorithm, "ssh-ed25519");
            assert_eq!(key.port, sshd.port);
        }
        other => panic!("expected UnknownHostKey, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn trusted_host_with_key_file_runs_a_command() {
    let sshd = sshd_or_skip!();
    sshd.trust().await;
    let client = connect(&sshd.opts()).await.expect("connect");
    let (out, code) = client.exec("echo setu-over-ssh").await.expect("exec");
    assert_eq!(String::from_utf8_lossy(&out), "setu-over-ssh\n");
    assert_eq!(code, Some(0));
    client.disconnect().await.expect("disconnect");
}

#[tokio::test(flavor = "multi_thread")]
async fn run_command_keeps_stderr_exit_code_and_only_the_tail() {
    let sshd = sshd_or_skip!();
    sshd.trust().await;
    let client = connect(&sshd.opts()).await.expect("connect");
    let (out, code) = client
        .run_command("echo out; echo err >&2; exit 3", 1024)
        .await
        .expect("run");
    let text = String::from_utf8_lossy(&out);
    assert!(text.contains("out") && text.contains("err"), "{text}");
    assert_eq!(code, Some(3));
    let (out, _) = client
        .run_command("seq 1 5000; echo the-end", 64)
        .await
        .expect("run");
    assert_eq!(out.len(), 64);
    assert!(String::from_utf8_lossy(&out).ends_with("the-end\n"));
    client.disconnect().await.expect("disconnect");
}

#[tokio::test(flavor = "multi_thread")]
async fn changed_host_key_is_refused() {
    let sshd = sshd_or_skip!();
    // Record a different ed25519 key for this host:port.
    let other = std::fs::read_to_string(sshd.path("other_key.pub")).expect("pub");
    let other = other
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ");
    std::fs::write(
        sshd.path("known_hosts"),
        format!("[127.0.0.1]:{} {other}\n", sshd.port),
    )
    .expect("write known_hosts");
    match connect(&sshd.opts()).await {
        Err(SshError::HostKeyChanged { key, line, .. }) => {
            assert_eq!(key.fingerprint, sshd.fingerprint());
            assert_eq!(line, 1);
        }
        other => panic!("expected HostKeyChanged, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_key_fails_authentication() {
    let sshd = sshd_or_skip!();
    sshd.trust().await;
    let mut opts = sshd.opts();
    opts.auth = vec![Auth::KeyFile {
        path: sshd.path("other_key"),
        passphrase: None,
    }];
    match connect(&opts).await {
        Err(SshError::AuthFailed { tried, .. }) => assert_eq!(tried.len(), 1),
        other => panic!("expected AuthFailed, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn encrypted_key_asks_for_a_passphrase_then_works() {
    let sshd = sshd_or_skip!();
    sshd.trust().await;
    let mut opts = sshd.opts();
    opts.auth = vec![Auth::KeyFile {
        path: sshd.path("encrypted_key"),
        passphrase: None,
    }];
    assert!(matches!(
        connect(&opts).await,
        Err(SshError::KeyNeedsPassphrase { .. })
    ));
    opts.auth = vec![Auth::KeyFile {
        path: sshd.path("encrypted_key"),
        passphrase: Some(Zeroizing::new("s3cret".to_string())),
    }];
    let client = connect(&opts).await.expect("connect with passphrase");
    let (out, _) = client.exec("echo ok").await.expect("exec");
    assert_eq!(String::from_utf8_lossy(&out), "ok\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn interactive_shell_echoes_and_resizes() {
    let sshd = sshd_or_skip!();
    sshd.trust().await;
    let client = connect(&sshd.opts()).await.expect("connect");
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let (exit_tx, exit_rx) = mpsc::channel::<Option<u32>>();
    let shell = client
        .open_shell(
            ShellSize {
                cols: 100,
                rows: 30,
            },
            move |b| {
                let _ = tx.send(b.to_vec());
            },
            move |code| {
                let _ = exit_tx.send(code);
            },
        )
        .await
        .expect("shell");

    let read_until = |needle: &str| {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut out = Vec::new();
        while Instant::now() < deadline {
            if let Ok(chunk) = rx.recv_timeout(Duration::from_millis(100)) {
                out.extend_from_slice(&chunk);
                if String::from_utf8_lossy(&out).contains(needle) {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    };

    shell.write(b"echo answer-$((6*7))\n").expect("write");
    let out = read_until("answer-42");
    assert!(out.contains("answer-42"), "output was: {out:?}");

    shell
        .resize(ShellSize {
            cols: 111,
            rows: 33,
        })
        .expect("resize");
    tokio::time::sleep(Duration::from_millis(200)).await;
    shell.write(b"stty size\n").expect("write");
    let out = read_until("33 111");
    assert!(out.contains("33 111"), "output was: {out:?}");

    shell.write(b"exit 5\n").expect("write");
    let code = exit_rx.recv_timeout(Duration::from_secs(10)).expect("exit");
    assert_eq!(code, Some(5));
}

#[tokio::test(flavor = "multi_thread")]
async fn proxy_jump_through_a_bastion() {
    let sshd = sshd_or_skip!();
    sshd.trust().await;
    // Jump through the server to itself: the inner session runs inside a
    // direct-tcpip channel opened by the outer one.
    let opts = sshd.opts().jump(sshd.opts());
    let client = connect(&opts).await.expect("connect via jump host");
    let (out, code) = client.exec("echo via-jump").await.expect("exec");
    assert_eq!(String::from_utf8_lossy(&out), "via-jump\n");
    assert_eq!(code, Some(0));
}

#[tokio::test(flavor = "multi_thread")]
async fn ssh_agent_authentication() {
    let sshd = sshd_or_skip!();
    if Command::new("ssh-agent").arg("-h").output().is_err() {
        eprintln!("skipping: ssh-agent is not installed");
        return;
    }
    sshd.trust().await;
    let sock = sshd.path("agent.sock");
    let mut agent = Command::new("ssh-agent")
        .arg("-D")
        .arg("-a")
        .arg(&sock)
        .stdout(Stdio::null())
        .spawn()
        .expect("start ssh-agent");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "ssh-agent did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    let added = Command::new("ssh-add")
        .arg(sshd.path("user_key"))
        .env("SSH_AUTH_SOCK", &sock)
        .stderr(Stdio::null())
        .status()
        .expect("ssh-add");
    assert!(added.success());

    // Only this test reads SSH_AUTH_SOCK; the others name a key file explicitly.
    std::env::set_var("SSH_AUTH_SOCK", &sock);

    // A security key file the server accepts. There is no device here, so
    // Setu must hand it to the agent and then report that it did not sign.
    let sk_path = sshd.path("id_ed25519_sk");
    let sk_line = fake_security_key(&sk_path);
    let mut authorized = std::fs::read_to_string(sshd.path("authorized_keys")).expect("read");
    authorized.push_str(&sk_line);
    std::fs::write(sshd.path("authorized_keys"), authorized).expect("write");
    let mut opts = sshd.opts();
    opts.auth = vec![Auth::KeyFile {
        path: sk_path,
        passphrase: None,
    }];
    let sk_result = connect(&opts).await;
    let listed = Command::new("ssh-add")
        .arg("-l")
        .env("SSH_AUTH_SOCK", &sock)
        .output()
        .expect("ssh-add -l");

    // The agent now holds the security key first; agent sign-in must skip
    // it (it cannot sign) and go on to the ordinary key.
    opts.auth = vec![Auth::Agent];
    let result = connect(&opts).await;
    let _ = agent.kill();
    let _ = agent.wait();
    match sk_result {
        Err(SshError::SecurityKey { reason, .. }) => {
            assert!(reason.contains("did not sign"), "{reason}")
        }
        other => panic!("expected a security key error, got {other:?}"),
    }
    assert!(
        String::from_utf8_lossy(&listed.stdout).contains("ED25519-SK"),
        "the agent should hold the security key: {}",
        String::from_utf8_lossy(&listed.stdout)
    );
    let client = result.expect("connect with ssh-agent");
    let (out, _) = client.exec("echo agent-ok").await.expect("exec");
    assert_eq!(String::from_utf8_lossy(&out), "agent-ok\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn sftp_browse_copy_rename_delete() {
    let sshd = sshd_or_skip!();
    sshd.trust().await;
    let client = connect(&sshd.opts()).await.expect("connect");
    let sftp = client.sftp().await.expect("sftp");
    assert!(sftp.home().await.expect("home").starts_with('/'));

    // The server is local, so a temp folder doubles as the "remote" side.
    let work = tempfile::tempdir().expect("tempdir");
    let local = work.path().join("local");
    let remote_root = work.path().join("remote");
    let back = work.path().join("back");
    for d in [&local, &remote_root, &back] {
        std::fs::create_dir_all(d).expect("mkdir");
    }
    let site = local.join("site");
    std::fs::create_dir_all(site.join("assets")).expect("mkdir");
    std::fs::write(site.join("index.html"), b"<h1>setu</h1>").expect("write");
    let big: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(site.join("assets/blob.bin"), &big).expect("write");

    let remote = remote_root.to_string_lossy().to_string();
    let control = TransferControl::default();
    let mut updates = 0;
    let done = sftp
        .upload(&site, &remote, false, &control, |_| updates += 1)
        .await
        .expect("upload");
    assert_eq!(done.files_done, 2);
    assert_eq!(done.done_bytes, done.total_bytes);
    assert_eq!(done.total_bytes, 13 + 700_000);
    assert!(updates >= 3);

    let listing = sftp.list(&format!("{remote}/site")).await.expect("list");
    let names: Vec<_> = listing.iter().map(|e| (e.name.as_str(), e.kind)).collect();
    assert_eq!(
        names,
        [("assets", EntryKind::Dir), ("index.html", EntryKind::File)]
    );

    sftp.download(&format!("{remote}/site"), &back, false, &control, |_| {})
        .await
        .expect("download");
    assert_eq!(
        std::fs::read(back.join("site/assets/blob.bin")).expect("read"),
        big
    );

    sftp.mkdir(&format!("{remote}/new")).await.expect("mkdir");
    sftp.rename(&format!("{remote}/new"), &format!("{remote}/renamed"))
        .await
        .expect("rename");
    assert!(remote_root.join("renamed").is_dir());

    sftp.remove(&format!("{remote}/site"))
        .await
        .expect("remove");
    assert!(!remote_root.join("site").exists());

    // A cancelled copy fails and leaves no partial file behind.
    let cancelled = TransferControl::default();
    cancelled.cancel();
    let err = sftp
        .upload(
            &site.join("assets/blob.bin"),
            &remote,
            false,
            &cancelled,
            |_| {},
        )
        .await
        .expect_err("cancelled");
    assert!(matches!(err, SshError::Cancelled));
    assert!(!remote_root.join("blob.bin").exists());
    assert!(!remote_root.join("blob.bin.setu-part").exists());

    // A paused copy keeps its partial file under the .setu-part name.
    let paused = TransferControl::default();
    paused.pause();
    let err = sftp
        .upload(
            &site.join("assets/blob.bin"),
            &remote,
            false,
            &paused,
            |_| {},
        )
        .await
        .expect_err("paused");
    assert!(matches!(err, SshError::Cancelled) && paused.paused());
    assert!(remote_root.join("blob.bin.setu-part").exists());
    assert!(!remote_root.join("blob.bin").exists());

    // Resuming an upload continues the partial file from where it stopped.
    std::fs::write(remote_root.join("blob.bin.setu-part"), &big[..300_000]).expect("write");
    let done = sftp
        .upload(
            &site.join("assets/blob.bin"),
            &remote,
            true,
            &control,
            |_| {},
        )
        .await
        .expect("resume upload");
    assert_eq!(done.resumed_bytes, 300_000);
    assert_eq!(done.done_bytes, 700_000);
    assert_eq!(
        std::fs::read(remote_root.join("blob.bin")).expect("read"),
        big
    );
    assert!(!remote_root.join("blob.bin.setu-part").exists());

    // Resuming again skips the file, which is already complete.
    let done = sftp
        .upload(
            &site.join("assets/blob.bin"),
            &remote,
            true,
            &control,
            |_| {},
        )
        .await
        .expect("skip");
    assert_eq!(done.resumed_bytes, 700_000);

    // Resuming a download does the same on the local side.
    let down = work.path().join("down");
    std::fs::create_dir_all(&down).expect("mkdir");
    std::fs::write(down.join("blob.bin.setu-part"), &big[..123_456]).expect("write");
    let done = sftp
        .download(&format!("{remote}/blob.bin"), &down, true, &control, |_| {})
        .await
        .expect("resume download");
    assert_eq!(done.resumed_bytes, 123_456);
    assert_eq!(std::fs::read(down.join("blob.bin")).expect("read"), big);
    assert!(!down.join("blob.bin.setu-part").exists());
}

// Unix only: it checks modes through std::os::unix.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn sftp_edit_in_place_and_permissions() {
    use setu_ssh::sftp::Change;
    use std::os::unix::fs::PermissionsExt;
    let sshd = sshd_or_skip!();
    sshd.trust().await;
    let client = connect(&sshd.opts()).await.expect("connect");
    let sftp = client.sftp().await.expect("sftp");
    let work = tempfile::tempdir().expect("tempdir");
    let conf = work.path().join("app.conf");
    std::fs::write(&conf, b"port = 80\n").expect("write");
    std::fs::set_permissions(&conf, std::fs::Permissions::from_mode(0o640)).expect("chmod");
    let path = conf.to_string_lossy().to_string();

    // Editing in place keeps the file's mode.
    assert_eq!(
        sftp.read_file(&path, 1_000_000).await.expect("read"),
        b"port = 80\n"
    );
    sftp.write_file(&path, b"port = 8080\n")
        .await
        .expect("write");
    assert_eq!(std::fs::read(&conf).expect("read"), b"port = 8080\n");
    let stat = sftp.stat(&path).await.expect("stat");
    assert_eq!(stat.permissions, Some(0o640));
    assert_eq!(stat.size, 12);
    assert!(sftp.read_file(&path, 4).await.is_err(), "over the limit");

    // One file, then a whole tree where folders stay enterable.
    let change = Change {
        mode: Some(0o600),
        ..Change::default()
    };
    sftp.set_attributes(&path, change, false)
        .await
        .expect("chmod");
    assert_eq!(
        std::fs::metadata(&conf).expect("meta").permissions().mode() & 0o7777,
        0o600
    );

    let site = work.path().join("site");
    std::fs::create_dir_all(site.join("css")).expect("mkdir");
    std::fs::write(site.join("index.html"), b"hi").expect("write");
    std::fs::write(site.join("css/app.css"), b"body{}").expect("write");
    let stat = sftp.stat(&path).await.expect("stat");
    let tree = Change {
        mode: Some(0o640),
        uid: None,
        gid: stat.gid,
    };
    let done = sftp
        .set_attributes(&site.to_string_lossy(), tree, true)
        .await
        .expect("chmod -R");
    assert_eq!(done.items, 4);
    let mode = |p: &Path| std::fs::metadata(p).expect("meta").permissions().mode() & 0o7777;
    assert_eq!(mode(&site), 0o750);
    assert_eq!(mode(&site.join("css")), 0o750);
    assert_eq!(mode(&site.join("index.html")), 0o640);
    assert_eq!(mode(&site.join("css/app.css")), 0o640);
}

// ---- Tunnels ----

/// A TCP server on localhost that echoes every line back with a prefix, so a
/// test can tell it reached this server and not something else.
async fn echo_server() -> u16 {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (read, mut write) = tcp.into_split();
                let mut lines = BufReader::new(read).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if write
                        .write_all(format!("echo:{line}\n").as_bytes())
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
    });
    port
}

/// Sends one line over `tcp` and returns the answer.
async fn round_trip(tcp: &mut tokio::net::TcpStream, line: &str) -> String {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    tcp.write_all(format!("{line}\n").as_bytes())
        .await
        .expect("write");
    let mut answer = String::new();
    tokio::time::timeout(
        Duration::from_secs(10),
        BufReader::new(tcp).read_line(&mut answer),
    )
    .await
    .expect("answer in time")
    .expect("read");
    answer
}

/// Waits until `check` is true, for counters updated on other tasks.
async fn eventually(check: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !check() {
        assert!(Instant::now() < deadline, "condition not met in time");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn local_forward_carries_connections_and_stops() {
    let sshd = sshd_or_skip!();
    sshd.trust().await;
    let echo = echo_server().await;
    let client = std::sync::Arc::new(connect(&sshd.opts()).await.expect("connect"));
    let fwd = client
        .forward_local("127.0.0.1", 0, "127.0.0.1", echo)
        .await
        .expect("forward");
    assert_ne!(fwd.port(), 0, "a free port is picked");
    assert_eq!(fwd.listening(), format!("127.0.0.1:{}", fwd.port()));

    // Two connections at once, each with its own channel.
    let mut a = tokio::net::TcpStream::connect(("127.0.0.1", fwd.port()))
        .await
        .expect("connect a");
    let mut b = tokio::net::TcpStream::connect(("127.0.0.1", fwd.port()))
        .await
        .expect("connect b");
    assert_eq!(round_trip(&mut a, "one").await, "echo:one\n");
    assert_eq!(round_trip(&mut b, "two").await, "echo:two\n");
    let stats = std::sync::Arc::clone(fwd.stats());
    eventually(|| stats.active() == 2 && stats.total() == 2).await;
    drop(a);
    eventually(|| stats.active() == 1).await;

    // Stopping frees the port and ends the connections it carried.
    let port = fwd.port();
    fwd.stop().await;
    eventually(|| stats.active() == 0).await;
    eventually(|| std::net::TcpStream::connect(("127.0.0.1", port)).is_err()).await;
    client.disconnect().await.expect("disconnect");
}

#[tokio::test(flavor = "multi_thread")]
async fn local_forward_to_a_closed_port_counts_a_failure() {
    let sshd = sshd_or_skip!();
    sshd.trust().await;
    let client = std::sync::Arc::new(connect(&sshd.opts()).await.expect("connect"));
    let closed = free_port();
    let fwd = client
        .forward_local("127.0.0.1", 0, "127.0.0.1", closed)
        .await
        .expect("forward");
    let _tcp = tokio::net::TcpStream::connect(("127.0.0.1", fwd.port()))
        .await
        .expect("connect");
    let stats = std::sync::Arc::clone(fwd.stats());
    eventually(|| stats.failed() == 1).await;
    assert_eq!(stats.total(), 0);

    // A port already taken here is a clear error.
    let taken = fwd.port();
    match client
        .forward_local("127.0.0.1", taken, "127.0.0.1", 22)
        .await
    {
        Err(SshError::Tunnel(message)) => {
            assert!(message.contains("could not listen"), "{message}")
        }
        other => panic!("expected a tunnel error, got {other:?}"),
    }
    // A destination that is not a host name is refused before anything opens.
    assert!(client
        .forward_local("127.0.0.1", 0, "bad host\n", 22)
        .await
        .is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_forward_is_a_socks5_proxy() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let sshd = sshd_or_skip!();
    sshd.trust().await;
    let echo = echo_server().await;
    let client = std::sync::Arc::new(connect(&sshd.opts()).await.expect("connect"));
    let fwd = client.forward_dynamic("127.0.0.1", 0).await.expect("socks");

    // By name: the server resolves "localhost".
    let mut tcp = tokio::net::TcpStream::connect(("127.0.0.1", fwd.port()))
        .await
        .expect("connect");
    tcp.write_all(&[5, 1, 0]).await.expect("greeting");
    let mut answer = [0u8; 2];
    tcp.read_exact(&mut answer).await.expect("method");
    assert_eq!(answer, [5, 0]);
    let mut request = vec![5, 1, 0, 3, 9];
    request.extend_from_slice(b"localhost");
    request.extend_from_slice(&echo.to_be_bytes());
    tcp.write_all(&request).await.expect("request");
    let mut reply = [0u8; 10];
    tcp.read_exact(&mut reply).await.expect("reply");
    assert_eq!(reply[1], 0, "the SOCKS request was not granted");
    assert_eq!(round_trip(&mut tcp, "via-socks").await, "echo:via-socks\n");

    // A destination the server cannot reach gets "host unreachable".
    let mut tcp = tokio::net::TcpStream::connect(("127.0.0.1", fwd.port()))
        .await
        .expect("connect");
    tcp.write_all(&[5, 1, 0]).await.expect("greeting");
    tcp.read_exact(&mut answer).await.expect("method");
    let mut request = vec![5, 1, 0, 1, 127, 0, 0, 1];
    request.extend_from_slice(&free_port().to_be_bytes());
    tcp.write_all(&request).await.expect("request");
    tcp.read_exact(&mut reply).await.expect("reply");
    assert_eq!(reply[1], 4);
    let stats = std::sync::Arc::clone(fwd.stats());
    eventually(|| stats.total() == 1 && stats.failed() == 1).await;

    // Asking for a username and password only is turned down.
    let mut tcp = tokio::net::TcpStream::connect(("127.0.0.1", fwd.port()))
        .await
        .expect("connect");
    tcp.write_all(&[5, 1, 2]).await.expect("greeting");
    tcp.read_exact(&mut answer).await.expect("method");
    assert_eq!(answer, [5, 0xff]);
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_forward_carries_server_connections_back() {
    let sshd = sshd_or_skip!();
    sshd.trust().await;
    let echo = echo_server().await;
    let client = std::sync::Arc::new(connect(&sshd.opts()).await.expect("connect"));
    let fwd = client
        .forward_remote("127.0.0.1", 0, "127.0.0.1", echo)
        .await
        .expect("remote forward");
    assert_ne!(fwd.port(), 0, "the server picked a port");
    assert!(fwd.listening().ends_with("on the server"));

    // sshd runs on this machine, so its port is reachable from the test.
    let mut tcp = tokio::net::TcpStream::connect(("127.0.0.1", fwd.port()))
        .await
        .expect("connect to the server's port");
    assert_eq!(round_trip(&mut tcp, "back").await, "echo:back\n");
    let stats = std::sync::Arc::clone(fwd.stats());
    eventually(|| stats.total() == 1).await;

    let port = fwd.port();
    fwd.stop().await;
    eventually(|| std::net::TcpStream::connect(("127.0.0.1", port)).is_err()).await;
    client.disconnect().await.expect("disconnect");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_closed_connection_is_noticed() {
    let sshd = sshd_or_skip!();
    sshd.trust().await;
    let client = std::sync::Arc::new(connect(&sshd.opts()).await.expect("connect"));
    let fwd = client.forward_dynamic("127.0.0.1", 0).await.expect("socks");
    assert!(!client.is_closed());
    client.disconnect().await.expect("disconnect");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !client.is_closed() {
        assert!(
            Instant::now() < deadline,
            "the closed connection was not noticed"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // A tunnel on a closed connection still answers, but cannot carry anything.
    let port = fwd.port();
    let mut tcp = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    tcp.write_all(&[5, 1, 0, 5, 1, 0, 1, 127, 0, 0, 1, 0, 22])
        .await
        .expect("request");
    let mut reply = [0u8; 12];
    tcp.read_exact(&mut reply).await.expect("reply");
    assert_eq!(reply[3], 4, "expected host unreachable");
}
