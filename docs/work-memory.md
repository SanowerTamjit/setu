# Work memory and automation suggestions

Engineers repeat a lot of work: the same five commands to deploy, `df -h` on every web server one by one, a long `journalctl` line typed from memory. **Work memory** notices that and offers to automate it, so you do the work once.

It lives in **Workflows → Memory** (or the palette: *Open work memory and suggestions*).

## Turning it on

Nothing is remembered until you press **Start remembering** in the Memory tab. While it is on, the status bar shows **● Remembering**; click it to open the tab. **Pause** stops it at any time, and *Pause or resume remembering* in the palette does the same.

To remove the feature altogether, turn off **Work memory and automation suggestions** in Settings → Features: the tab and the indicator go, and nothing is remembered even if you had turned it on.

## What is kept

Each command line you run in a Setu terminal: the text, when, and where (the saved host's name for SSH tabs, `local` for your own shells). It is read from the terminal when you press Enter, so tab completion and history recall (↑) are kept as the shell shows them.

Not kept:

- anything typed at a prompt that asks for a password, passphrase, PIN, token, one-time or verification code;
- a line you typed that the shell did not echo (a password prompt Setu did not recognise);
- lines starting with a space (as `HISTCONTROL=ignorespace` does in bash);
- what you type inside full-screen programs (vim, less, top);
- commands on your ignore list (Settings in the Memory tab: one per line, whole words, so `vim` skips `vim notes.txt` but not `vimdiff`);
- anything holding a private key block, and lines over 2,000 characters.

Secrets in what is left are masked before they are saved: `NAME=value` and `--name value` where the name is about a password, token, secret or key; `user:password@` in URLs; `curl -u user:password`; `Authorization: Bearer …`; `mysql -pPASSWORD`; `sshpass -p …`; and words shaped like GitHub, GitLab, Slack, npm, AWS or OpenAI-style keys, JWTs or long random strings. A masked command shows with a **masked** badge and is never suggested.

Commands older than the retention period (30 days by default, 1 to 365) and anything past the newest 5,000 are deleted.

## Where it is stored

One file, `work-memory.json` in Setu's config folder (`setu paths` shows it), readable only by you (0600) and written atomically. **Nothing is sent anywhere**: there is no network call in this feature, and no AI service is used.

You can delete a single command (×), **Forget all commands** (snippets and settings stay), or **Delete everything** in the Memory settings, which removes the file.

## Suggestions

Setu looks for three kinds of repeated work. A *sitting* is the commands of one terminal tab or pane with no break longer than 30 minutes.

| Suggestion | When | Becomes |
|---|---|---|
| **Workflow** | The same 2 to 5 commands, in the same order, on saved hosts, in 3 or more sittings | A workflow with one step per command, each after the previous one succeeded, on the hosts they ran on |
| **Fleet workflow** | One command run on 3 or more saved hosts | A workflow with one step on all of them, or on a tag they all share (`tag:web`) |
| **Snippet** | A command (10+ characters) typed 4 or more times in 2 or more sittings, or a few local commands always typed together | A snippet in the command palette |

`ls`, `cd`, `clear` and similar everyday commands are never suggested on their own. In a workflow, a `cd` or `export` is carried into the steps after it (`cd /srv/app && ./deploy.sh`), because each step runs in a fresh SSH session.

**Open as workflow** opens a draft in the builder. Nothing runs: check the steps, rename it, add an approval step if you like, then **Save** and **Run** as with any workflow (see `docs/workflows.md`). The suggestion is then hidden.

**Save as snippet** asks for a name. Snippets show in the palette under *Snippet*; picking one types the command into the terminal **without pressing Enter**. A command that holds a secret cannot be a snippet.

**Not useful** hides a suggestion for good.

## Sharing with a teammate or an AI assistant

**Copy summary** copies a short Markdown summary of your routine work (the commands you run most and the routines you repeat, masked commands left out) to the clipboard. Paste it into your team's runbook, a ticket, or an AI assistant as context. It leaves your device only when you paste it.

## Files

| File | What |
|---|---|
| `work-memory.json` | Settings (on/off, retention, ignore list), commands, saved snippets, dismissed suggestions |
