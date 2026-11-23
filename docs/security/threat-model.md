# Threat model

What Setu protects, from whom, and where each protection lives. Update this file in the same pull request as any change that adds a way in (a new IPC command, a new file Setu reads, a new network call) or changes one of the protections below.

## What we protect

| Asset | Where it lives | Protection |
|---|---|---|
| SSH passwords and key passphrases | `vault.json` | Argon2id (64 MiB, 3 passes) + XChaCha20-Poly1305; header authenticated as AAD; file 0600, written atomically; secrets zeroized in memory; auto-lock |
| Vault unlock key ("unlock automatically") | OS keychain (opt-in) | Only the derived key, never the password; deleted when it stops matching |
| Private keys | `~/.ssh` | Made 0600, never overwritten; passphrases never on a command line |
| Trust in servers | `~/.ssh/known_hosts` | Strict checking; changed keys refused with a warning; only keys a server actually offered can be trusted; host names that would be patterns are refused |
| Files being edited | per-user private folder | 0700 random folder under `XDG_RUNTIME_DIR` or `~/.cache` (Linux), per-user temp elsewhere; deleted on close, swept after a week |
| What you type and see | memory only | Never logged; `session.json` holds hosts, users, ports and key paths only |
| Workflows and their run output | `workflows/*.toml`, `workflow-runs.json` | 0600 files (folder 0700), written atomically; history keeps the last 100 runs and 8 KiB of output per host; **Clear run history** removes it |
| Task board cards (titles, notes, host names) | `board.toml` | 0600 file, written atomically; size-capped and checked (ids, lengths, dates) on load and on save; a file that fails to load is never written over; cards never run anything |
| The builds people install | GitHub Releases | Pinned actions, no signing secrets in PR builds, SHA-256 sums, Sigstore provenance, SBOM |

## Who might attack, and how

1. **A malicious or compromised server** you connect to. It controls everything it sends: terminal output (escape sequences), SFTP listings (file names), file contents, host keys.
   - File names are checked to be a single path part (and on Windows, no drive letter or `:stream`) before anything is written locally.
   - Files opened "with the default app" that would run instead of open (`.bat`, `.exe`, `.desktop`, `.command`, …) are refused.
   - Terminal output is rendered by xterm.js, which does not run scripts or write the clipboard (no OSC 52).
2. **A malicious file you import**: an `ssh_config`, PuTTY/MobaXterm/Termius/CSV/Ansible export, or a pack.
   - Host names with whitespace, commas, `* ? ! | @ [ ] #` are refused, so "trust this host" cannot become "trust this key for every host".
   - `ProxyCommand` and other commands are never run, only noted.
   - Passwords in import files are never read.
   - Packs are data only: one-line snippets typed, never run; unknown fields refused.
3. **A web page whose text you copy.** Pasting several lines into a shell without bracketed paste would run them at once; the paste guard shows them and asks first.
4. **Another user on the same computer.** Config, vault, session and key files are 0600; edit folders are 0700 and not in shared `/tmp`.
5. **A workflow file someone gives you** (`workflows/*.toml`). It is a script by design: it runs the commands it lists on your hosts. Protections: nothing runs without **Run** and a confirmation that lists every command and host; commands only go to saved hosts over SSH (nothing runs locally, no `sh -c`); the file is size-capped and checked (ids, links, loops, sizes) before it is listed or run; the Rust side runs only what it reads back from disk, never a workflow sent by the window; runs never prompt, so they never accept a new or changed host key and take secrets only from the vault or ssh-agent; server output is stripped of escape sequences and shown as text.
6. **Code in the web view gone wrong** (an XSS bug in the UI). The CSP allows only Setu's own scripts. Rust commands check their inputs instead of trusting the UI: host-key trust needs a key the server offered, key commands only touch `~/.ssh`, JSON files are size-limited and parsed, workflow ids are checked so files stay in `workflows/`; `board_save` checks every card before writing `board.toml`.
7. **The supply chain**: a compromised dependency or GitHub Action. Lockfiles with `--locked`/`--frozen-lockfile`, `cargo deny` (advisories, licenses, sources), daily advisory audit, dependency review on PRs, actions pinned to commit SHAs, CodeQL, OpenSSF Scorecard.

## Out of scope

- An attacker who already runs code as you, or as root, on your computer (they can read your keychain and memory).
- Weaknesses of the server's own SSH setup.
- Physical access to an unlocked, logged-in computer while "unlock automatically" is on (the UI says so).

## Known gaps (tracked for later)

- SSH host certificates (`@cert-authority`) are refused rather than checked.
- Windows: `setu doctor` does not check file ACLs yet.
- Windows and macOS release signing switches on only once the certificates are added (see `docs/releasing.md`).
- Signing secrets are repository secrets; moving them to a protected `release` environment that only `master` can use is a one-time settings change (see `docs/releasing.md`).
