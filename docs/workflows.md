# Workflows

The **Workflows** view lets you draw the steps you would otherwise type into several terminals: run commands on your saved hosts, in order or at the same time, stop for an approval before a risky step, and roll back when something fails. Each run is kept in history with what every host printed.

Open it from the rail (Classic), the **Workflows** switch in the title bar (Studio), or the command palette (`Go to workflows`, `New workflow`). It can be turned off in Settings → Features (`workflows`).

## Steps

| Step | What it does |
|---|---|
| **Command** | Runs a command on one or more hosts, like `ssh host 'command'` (through the login shell, no terminal). Exit code 0 is success. With several hosts it runs on all of them at once, or **one host at a time**, stopping at the first failure (for rolling restarts). An optional **timeout** fails the host when it takes too long. |
| **Approval** | Pauses the run until you press **Approve** or **Reject** (on the step or in the side panel). Rejecting fails the step, so a failure link can roll back. |
| **Wait** | Pauses for a number of seconds, for example to let a service settle. |

Hosts are your saved hosts, by name, or `tag:<tag>` for every host with that tag (`tag:web`). The step shows how many hosts it will reach, and a name that is not a saved host is marked.

## Links

Drag the dot on a step's right edge onto another step to link them. Click a link to choose when it lets the next step run:

- **on success** (green): the previous step succeeded on every host. This is the default.
- **on failure** (red): the previous step failed, for alerts and rollbacks.
- **always** (grey, dashed): either way, for clean-up and summaries.

Steps that are not linked to each other run at the same time. A step with several incoming links waits for all of them and runs when every one of them fires; links from a step that was skipped are ignored, so branches can join again after an if/else. Links can never make a loop.

A run **fails** when a step fails and nothing handles it (no failure or always link out of it). A failure that a link handles, such as a health check that triggers an alert, does not fail the run.

## Running

**Run** saves the workflow and shows every command and the hosts it will run on before anything starts. While it runs, each step shows its state (waiting, running, needs approval, done, failed, skipped, stopped) and the links that fired are highlighted. Click a step to see each host's exit code and output. **Stop run** cancels steps that are running and everything still waiting.

Runs never ask for anything, because nobody may be watching:

- The host key must already be trusted: connect to the host once from Hosts to check and trust it. A changed key stops the step with a warning.
- Passwords and key passphrases come from the vault (unlock it first), or use ssh-agent. Keys and jump hosts set on the saved host are used as usual.
- Each host gets one connection for the whole run, closed when the run ends.

## Files

Each workflow is a TOML file in `workflows/` in Setu's config folder (`~/.config/setu/workflows` on Linux), so you can keep it in Git or share it. Runs are kept in `workflow-runs.json` next to it (the last 100 runs, the last 8 KiB of output per host). Both are readable only by you. **Clear run history** in the list removes them.

```toml
version = 1
id = "deploy-web"
name = "Deploy web app"

[[step]]
id = "check"
name = "Check disk space"
kind = "command"          # command | approval | wait
command = "df -h / | tail -1"
targets = ["tag:web"]
x = 60                     # position on the canvas
y = 60

[[step]]
id = "approve"
name = "Approve deploy"
kind = "approval"
message = "Deploy release 1.4.2?"
x = 340
y = 60

[[step]]
id = "deploy"
name = "Deploy"
kind = "command"
command = "./deploy.sh 1.4.2"
targets = ["web-01", "web-02"]
one_at_a_time = true
timeout_seconds = 120
x = 620
y = 60

[[step]]
id = "rollback"
name = "Roll back"
kind = "command"
command = "./deploy.sh 1.4.1"
targets = ["tag:web"]
x = 900
y = 200

[[link]]
from = "check"
to = "approve"

[[link]]
from = "approve"
to = "deploy"

[[link]]
from = "deploy"
to = "rollback"
when = "failure"           # success (default) | failure | always
```

A file that does not parse, or breaks a rule (unknown step in a link, a loop, a command over 8 KiB, more than 100 steps or 200 hosts per step), is listed under the workflows with the reason and is never run.

## Security

- A workflow runs only the commands you see in it, on hosts from your inventory, as the user you log in with. Setu runs nothing locally.
- The app reads the workflow back from disk and checks it again before running; the window cannot send it commands of its own.
- Server output is shown as plain text: terminal escape sequences and control characters are removed.
- A workflow from someone else is a script: read every command before you run it, as you would a shell script.
