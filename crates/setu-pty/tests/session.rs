//! Integration tests that run real programs in a real PTY on each OS.

use setu_pty::{PtySession, SpawnOptions};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(20);

/// Spawns `opts` and returns the session plus a channel of output chunks.
fn spawn(opts: SpawnOptions) -> (PtySession, mpsc::Receiver<Vec<u8>>) {
    let (tx, rx) = mpsc::channel();
    let session = PtySession::spawn(opts, move |chunk| {
        let _ = tx.send(chunk.to_vec());
    })
    .expect("spawn");
    (session, rx)
}

/// Collects output until it contains `needle` or the timeout passes.
fn read_until(rx: &mpsc::Receiver<Vec<u8>>, needle: &str) -> String {
    let deadline = Instant::now() + TIMEOUT;
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
}

#[cfg(unix)]
fn echo(text: &str) -> SpawnOptions {
    SpawnOptions::new("/bin/sh").args(["-c", &format!("echo {text}")])
}

#[cfg(windows)]
fn echo(text: &str) -> SpawnOptions {
    SpawnOptions::new("cmd.exe").args(["/C", &format!("echo {text}")])
}

#[cfg(unix)]
fn interactive_shell() -> SpawnOptions {
    SpawnOptions::new("/bin/sh")
}

#[cfg(windows)]
fn interactive_shell() -> SpawnOptions {
    SpawnOptions::new("cmd.exe").args(["/Q", "/K"])
}

#[test]
fn runs_a_command_and_reads_its_output() {
    let (session, rx) = spawn(echo("hello-from-setu"));
    let out = read_until(&rx, "hello-from-setu");
    assert!(out.contains("hello-from-setu"), "output was: {out:?}");
    let status = session
        .wait_timeout(TIMEOUT)
        .expect("wait")
        .expect("exited");
    assert!(status.success());
}

#[test]
fn reports_a_failing_exit_code() {
    #[cfg(unix)]
    let opts = SpawnOptions::new("/bin/sh").args(["-c", "exit 3"]);
    #[cfg(windows)]
    let opts = SpawnOptions::new("cmd.exe").args(["/C", "exit 3"]);
    let (session, _rx) = spawn(opts);
    let status = session
        .wait_timeout(TIMEOUT)
        .expect("wait")
        .expect("exited");
    assert_eq!(status.code, 3);
}

#[test]
fn input_reaches_an_interactive_shell() {
    let (session, rx) = spawn(interactive_shell());
    session
        .write(b"echo typed-$((40+2))\r")
        .unwrap_or_else(|e| panic!("{e}"));
    #[cfg(unix)]
    let needle = "typed-42";
    // cmd does not do arithmetic in echo; just check the text round-trips.
    #[cfg(windows)]
    let needle = "typed-$((40+2))";
    let out = read_until(&rx, needle);
    assert!(out.contains(needle), "output was: {out:?}");
    session.kill().expect("kill");
}

#[test]
fn passes_extra_environment_and_term() {
    #[cfg(unix)]
    let opts = SpawnOptions::new("/bin/sh")
        .args(["-c", "echo \"$SETU_TEST_VAR/$TERM\""])
        .env("SETU_TEST_VAR", "bridge");
    #[cfg(windows)]
    let opts = SpawnOptions::new("cmd.exe")
        .args(["/C", "echo %SETU_TEST_VAR%/%TERM%"])
        .env("SETU_TEST_VAR", "bridge");
    let (_session, rx) = spawn(opts);
    let out = read_until(&rx, "bridge/xterm-256color");
    assert!(out.contains("bridge/xterm-256color"), "output was: {out:?}");
}

#[cfg(unix)]
#[test]
fn resize_is_seen_by_the_program() {
    let (session, rx) = spawn(interactive_shell());
    let size = setu_pty::PtySize {
        rows: 33,
        cols: 111,
    };
    session.resize(size).expect("resize");
    assert_eq!(session.size(), size);
    session.write(b"stty size\n").expect("write");
    let out = read_until(&rx, "33 111");
    assert!(out.contains("33 111"), "output was: {out:?}");
    session.kill().expect("kill");
}

#[test]
fn kill_ends_a_running_program() {
    let (session, _rx) = spawn(interactive_shell());
    assert_eq!(session.try_wait().expect("try_wait"), None);
    session.kill().expect("kill");
    assert!(session.wait_timeout(TIMEOUT).expect("wait").is_some());
}

#[test]
fn spawning_a_missing_program_fails_cleanly() {
    let result = PtySession::spawn(SpawnOptions::new("/definitely/not/a/shell"), |_| {});
    assert!(result.is_err());
}

#[test]
fn detects_at_least_one_shell() {
    let shells = setu_pty::detect_shells();
    assert!(!shells.is_empty(), "no shells detected");
    assert!(shells[0].program.is_file(), "{:?}", shells[0]);
}
