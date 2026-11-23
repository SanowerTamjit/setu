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
        .with_policy(Arc::new(MyPolicy::load()));
    setu_desktop_lib::run_with(extensions);
}
```

The types live in `setu_core::extend`.

## Audit trail: `AuditSink`

`record(&AuditEvent)` is called when:

| Action | When | `target` | `detail` |
|---|---|---|---|
| `ssh.connect` | a terminal tab logs in, or fails to | `user@host:port` | empty, or a fixed reason (`host key changed`, `needs a password`, …) |
| `file.transfer` | an SFTP copy ends | host | `upload <source> to <folder>` (or `download`), `(paused)` when paused |
| `workflow.run` | a workflow run finishes | workflow id | name and run id |
| `command.blocked` | the policy refuses a command | saved host | workflow, step and the policy's reason |

Events never hold a password, passphrase, key or command output, and a failed login is described by a fixed reason, not the server's message. `record` is called on the thread doing the work, so return quickly: queue the event and write it elsewhere.

## Command policy: `CommandPolicy`

`check(&CommandCheck)` is asked before each command a workflow step runs on a host, with the saved host's name, the command and where it came from (`CommandOrigin::Workflow { workflow, step }`). Return `PolicyDecision::Allow`, or `Deny { reason }`: the command is not run, that host fails with "blocked by policy: <reason>", and the refusal is recorded as `command.blocked`. Name the rule in the reason, not the command, because the reason is shown and recorded.

Commands typed into a terminal are not checked: they are the person's own keystrokes.

## Rules for the hooks

- Never log the command text unmasked, and never send events anywhere the user has not agreed to.
- Keep the hooks fast; they run in line with the work.
- New hook points are added to the open-source repository first, with a no-op default, tests and a line here.
