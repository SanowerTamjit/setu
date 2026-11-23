# Phase 1 security and project review (2026-11-22)

A review of everything in Phase 1 (PRs #1 to #23) for ways data could leak or be tampered with, CVEs in dependencies, build and supply-chain safety, and how the project is run. **Fixed** means a fix is in this pull request, with a test where it is code.

## Findings fixed

| # | Severity | Area | Finding | Fix |
|---|---|---|---|---|
| 1 | High | SSH host keys | A host name from an imported file (PuTTY, MobaXterm, CSV, Ansible, ssh_config, hosts.toml) was written unchecked into `known_hosts` when its key was trusted. A name like `x⏎* ssh-ed25519 AAAA…` would add a wildcard line trusting an attacker's key **for every host**, silently enabling man-in-the-middle on all later connections. | Host names with whitespace, commas or OpenSSH pattern/marker characters are refused at import, connect and trust; trust writes the key as parsed, not the given text (`known_hosts::check_host`, tests `host_names_cannot_inject_entries`, `normalize_rejects_unusable_hosts`). |
| 2 | Medium | SSH host keys / IPC | `ssh_trust_host` wrote any key the web view sent. A UI bug (or XSS) could pin a key of its choosing. | The app remembers keys servers actually offered; only those can be trusted, once each (`OfferedKeys`, test `only_offered_keys_can_be_trusted`). |
| 3 | Medium (Windows) | SFTP downloads, remote edit | `check_name` allowed `C:name` (a path on drive C, which `Path::join` treats as absolute) and `name:stream` (an NTFS alternate data stream). A malicious server's listing could write outside the chosen download folder. | `check_local_name`: one plain path part, and no `:` on Windows (test in `sftp.rs`). |
| 4 | Medium (shared Linux machines) | Remote edit temp files | Copies went to `/tmp/setu-edit`, shared by every user. Another user could create it first, so Setu's permission change would fail (denial of service), and the old sweep ran inside a folder another user controlled. | Per-user root (`$XDG_RUNTIME_DIR/setu-edit`, else `~/.cache/setu/edit`), 0700, refused if it is not a folder; each edit gets a fresh random 0700 folder (test `edit_folders_are_private_and_new`). |
| 5 | Medium | Remote edit | With no editor set, the file was handed to the system's default app. For `deploy.bat`, `x.exe`, `run.command`, `app.desktop`, … that **runs** the server's file on your computer instead of opening it. | Those types are refused with no editor set, with a message to set one (test `default_app_never_runs_programs`). |
| 6 | Medium | Packs | Snippets could contain line breaks. Picking one types it into the terminal; in a shell without bracketed paste each line break is Enter, so a pack could run commands you never saw. | Multi-line snippets are refused at install, and the app joins lines before typing (test in `packs.rs`; `docs/packs.md` updated). |
| 7 | Low | Pasting | Pasting several lines into a shell without bracketed paste runs them at once (a known trick: web pages can hide extra lines in copied text). | New paste guard (feature flag `pasteGuard`, on): shows the lines and asks first. |

## Checked, no problem found

| Area | What was checked | Result |
|---|---|---|
| Vault crypto | KDF (Argon2id 64 MiB/3/1, bounds on hostile params), cipher (XChaCha20-Poly1305, random 24-byte nonce per save), header bound as AAD, constant-time password check, atomic 0600 writes, zeroize, redacted `Debug` | OK |
| Vault / OS keychain | Only the derived key is stored; opt-in; removed when it stops matching; no password stored | OK |
| SSH | Strict host-key checking for every hop of a jump chain; changed keys never overridable from the UI; host certificates refused (not silently accepted) | OK |
| Secrets in logs | No `println!`/`eprintln!`/`console.log` of secrets in the app or crates; `AuthRequest` and `UnlockKey` `Debug` redact | OK |
| Session restore | `session.json` holds host, user, port, auth kind, key path, jump: no passwords, passphrases, output or input; 0600, atomic, 1 MB cap | OK |
| Detached windows | Scrollback moves in memory only, one-time token, 16 MB cap | OK |
| Key generation | 0600 files, never overwritten, `ssh-keygen` gets no passphrase on the command line (PIN via a private askpass) | OK |
| Imports | `ProxyCommand`/`LocalCommand` never run; passwords never read | OK |
| Desktop IPC surface | All 56 commands reviewed; CSP `default-src 'self'` (no inline scripts, no remote origins); only `core:default` permissions; no shell/open plugins | OK |
| SFTP | Remote names checked before local writes; symlinked folders not followed on download; resumable `.setu-part` files | OK |
| Rust dependencies | `cargo deny check advisories` against the RustSec database (2026-11-22) | 0 advisories. The RSA ignore was stale and is removed |
| npm dependencies | `pnpm audit` | 0 known vulnerabilities |
| Licenses and sources | `cargo deny check licenses sources` | OK |

## Build and supply chain

| Check | Before | Now |
|---|---|---|
| Actions pinned to commit SHAs | Tags only (26 high findings in zizmor) | All pinned, tag in a comment, updated by Dependabot; zizmor: 0 findings |
| Token kept in `.git/config` after checkout | Yes | `persist-credentials: false` everywhere |
| Signing secrets in pull-request nightly builds | Passed to PR code | Never passed to `pull_request` builds |
| Workflow permissions | `contents: read` default, write only in publish | Unchanged (already least privilege) |
| SBOM | None | SPDX SBOM in every nightly, checksummed and attested |
| Provenance | Sigstore attestation | Unchanged, now covers the SBOM |
| New CVEs between pull requests | Found only on the next PR | Daily `audit.yml` (RustSec + npm) |
| Static analysis | Clippy only | CodeQL for Rust, TypeScript and the workflows (security-extended) |
| Dependency review | None | Pull requests adding vulnerable or copyleft dependencies fail (runs once the dependency graph is on, see below) |
| OpenSSF Scorecard | None | Weekly, results under Security → Code scanning |
| Reproducibility | `--locked`, `--frozen-lockfile`, stable toolchain | Unchanged; exact toolchain pinning is left for versioned releases |

## For the maintainer (repository settings, not code)

These need a click in GitHub settings; nothing in the repository can do them:

1. **Settings → Code security**: turn on *Dependency graph*, *Private vulnerability reporting* (SECURITY.md points to it), *Dependabot alerts* and *security updates*, *Secret scanning* and *Push protection*. Then add the repository variable `DEPENDENCY_REVIEW` = `on` (Settings → Secrets and variables → Actions → Variables) so the dependency review check runs on pull requests.
2. **Settings → Branches → master**: require pull requests, the CI checks, and signed-off commits; block force pushes.
3. **Settings → Environments**: create `release`, limited to `master`, and move the signing secrets there (then add `environment: release` to the nightly build job).
4. **Settings → Actions → General**: "Allow actions … pinned to a full-length commit SHA" and read-only `GITHUB_TOKEN` by default.

## Project practices added

- `CLAUDE.md` (and `AGENTS.md`): how AI assistants work in this repository, with a security checklist.
- `docs/development-workflow.md`: the AI-assisted development method.
- `docs/feature-flags.md`: feature flags and their life cycle.
- `docs/security/threat-model.md`: what is protected, from whom, and known gaps.
- Release notes: `CHANGELOG.md` gains a **Security** section; `.github/release.yml` groups generated notes by label; `docs/releasing.md` has the release checklist.
