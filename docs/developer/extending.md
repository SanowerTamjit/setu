# Extending Setu

A build of Setu can add its own behaviour at a few fixed points, without changing or forking the app. The open-source app uses the defaults: **nothing is recorded and nothing is blocked**. A company's own build, or Setu Enterprise, passes its own hooks when it starts.

Extensions are Rust code compiled into the build. Nothing is loaded at run time, and there is no plugin folder to drop files into.

## Starting the app with extensions

```rust
use std::sync::Arc;
use setu_desktop_lib::Extensions;

fn main() {
    let extensions = Extensions::default()
        .with_audit(Arc::new(MyAuditLog::open()))
        .with_policy(Arc::new(MyPolicy::load()))
        .with_access(Arc::new(MyAccessRules::new()))
        .with_recorder(Arc::new(MyRecorder::new()))
        .with_panel(Arc::new(MyAdminPage::new()));
    setu_desktop_lib::run_with(extensions);
}
```

The types live in `setu_core::extend`.

## Audit trail: `AuditSink`

`record(&AuditEvent)` is called when:

| Action | When | `target` | `detail` |
|---|---|---|---|
| `ssh.connect` | a terminal tab logs in, fails to, or the access policy refuses the login (outcome `blocked`) | `user@host:port` | empty, a fixed reason (`host key changed`, `needs a password`, …), or `not allowed: <reason>` |
| `file.transfer` | an SFTP copy ends | host | `upload <source> to <folder>` (or `download`), `(paused)` when paused |
| `workflow.run` | a workflow run finishes | workflow id | name and run id |
| `command.blocked` | the policy refuses a command | saved host | workflow and step (or `server services`) and the policy's reason |
| `server.change` | a change from server management ran (succeeded or failed) | `user@host:port` | the exact command, such as `sudo -n systemctl restart nginx.service`, or for keys what changed: `add to /home/ops/.ssh/authorized_keys: ssh-ed25519 SHA256:… (sam@laptop)` |

Events never hold a password, passphrase, key or command output, and a failed login is described by a fixed reason, not the server's message. `record` is called on the thread doing the work, so return quickly: queue the event and write it elsewhere.

## Command policy: `CommandPolicy`

`check(&CommandCheck)` is asked before each command a workflow step runs on a host, with the saved host's name, the command and where it came from (`CommandOrigin::Workflow { workflow, step }`), and before each change from server management (`CommandOrigin::Server { tool }` with `tool` `"services"`, `"processes"` or `"keys"`; the command is the exact line the person approved, or for keys a description such as `remove from PATH: TYPE FINGERPRINT (COMMENT)`). Return `PolicyDecision::Allow`, or `Deny { reason }`: the command is not run, that host fails with "blocked by policy: <reason>" (server management shows "Your organization's policy does not allow this: <reason>"), and the refusal is recorded as `command.blocked`. Name the rule in the reason, not the command, because the reason is shown and recorded.

Commands typed into a terminal are not checked: they are the person's own keystrokes.

## Access policy: `AccessPolicy`

`check_connect(&ConnectCheck)` is asked before every SSH login, from a terminal tab or a workflow, with the host's name, address, port, remote user and tags. A login typed into the connect form takes the name and tags of the saved host with the same address and port; otherwise the name is the address and there are no tags. Return `Allow`, or `Deny { reason }`: Setu does not open a connection, the person sees "Your organization does not allow this login: <reason>" (a workflow fails that host with the same text), and the refusal is recorded as a `blocked` `ssh.connect`. Only the target is checked, not a jump host on the way.

The app calls it on a blocking thread, so it may take a moment (for example to ask a server), but every login waits for it: answer from a cache when you can.

## Session recording: `SessionRecorder`

`start(&SessionStart)` is asked each time an SSH shell opens in a terminal tab or a split pane, after the login worked, with the host's name, address, port, remote user, tags and the terminal size. Return `None` to leave it unrecorded, or a `SessionTape`, which then gets:

- `output(bytes)`: everything the server sends, exactly as the terminal gets it (a multi-byte character can be split across two calls);
- `resize(cols, rows)`: when the terminal changes size;
- `finish(exit_code)`: once, when the shell exits or the tab closes (`None` when there was no exit status). Nothing is sent after it.

When a session is recorded, the app first writes a dimmed line at the top of the terminal, before any output from the server: `[Setu] This session is recorded by your organization.` The person always knows. Keystrokes are **not** given to the recorder, so a password typed at a `sudo` or `passwd` prompt (which the server does not echo) is not recorded; what the server echoes back, and everything it prints, is. Local shells and workflow runs are not recorded.

`start` runs on an async task and the tape's methods on the thread reading the session, so keep them fast: open a local file, buffer, and upload elsewhere.

## A settings page: `SettingsPanel`

A build can add one page to Settings (for example "Organization", with a licence, an audit log and a policy). The extension returns plain data: sections with a title, an optional status badge (`ok`, `warning`, `error`), a note, label/value rows, monospace lines and buttons. The web view draws it as text and never runs code or HTML from the extension. A button calls `action(id)`, off the main thread, which returns a message to show, a file to save, or a web address to open in the person's browser (for example an organisation's sign-in page). Saved files go to a **new** file in the person's Downloads folder (or home folder), with mode 0600. The name is reduced to plain characters, and an existing file is never overwritten. Addresses open only if they are `https://`, or `http://` to this computer (`127.0.0.1`, `localhost`, `[::1]`), with no user name or password in them; the app opens them with the system browser through the Tauri opener plugin, whose commands the web view is not allowed to call.

`edition()` names the edition (for example "Enterprise"), which then shows in the window title and the status bar.

## Rules for the hooks

- Never log the command text unmasked, and never send events anywhere the user has not agreed to.
- Keep the hooks fast; they run in line with the work.
- New hook points are added to the open-source repository first, with a no-op default, tests and a line here.
