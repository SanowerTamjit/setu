# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Minimum supported Rust version is now 1.89 (needed by russh 0.65, which fixes RUSTSEC-2026-0495).
- The desktop app uses xterm's DOM renderer on Linux: the WebGL renderer showed new output one frame late under WebKitGTK.

### Added

- Nightly builds: a scheduled workflow builds the desktop app (`.dmg`, `.msi`, `-setup.exe`, `.AppImage`, `.deb`, `.rpm`) and the `setu` CLI for macOS (universal), Windows and Linux, and publishes them as the rolling `nightly` pre-release with `SHA256SUMS` and GitHub build-provenance attestations. macOS signing and notarization and Windows Authenticode switch on when their secrets are added; until then macOS builds are ad-hoc signed. See `docs/releasing.md`.
- `setu` command line (`setu-cli`): `setu doctor` checks private-key permissions and passphrases, ssh-agent and the keys it holds, `~/.ssh/config` permissions and missing `IdentityFile`s, `known_hosts`, and Setu's saved hosts and vault file, with a fix for each problem (`--json` for scripts; exit code 1 when something blocks SSH). `setu hosts` lists and searches saved hosts, and `setu hosts import` previews or saves hosts from `~/.ssh/config`. See `docs/cli.md`.
- Files tabs (SFTP): two panes, your computer and an SSH host, with upload and download by button or drag and drop, folder copies, a transfer queue with progress, speed and cancel, and new folder, rename and delete on the host. Opens from an SSH tab (Ctrl+Shift+F) or from a host's ⇅ button, reusing that connection. See `docs/files.md`.
- Encrypted vault (`setu-vault`): saves SSH passwords and key passphrases under a master password (Argon2id + XChaCha20-Poly1305), locks itself after 15 minutes idle, and fills them in when you connect, so saved hosts connect without asking. "Save in vault" in the connect dialog, a vault indicator in the status bar, and a vault dialog to remove secrets, lock, or change the master password. See `docs/vault.md`.
- Saved hosts (`setu-hosts`): a hosts sidebar with folders, tags and search; double-click (or Enter) to connect; add, edit and delete hosts. Hosts are stored in a plain `hosts.toml` without secrets (see `docs/hosts.md`). **Import** brings in hosts from `~/.ssh/config`, following `Include` and OpenSSH's matching rules. Ctrl+Shift+H shows the sidebar and focuses search.
- SSH connections accept a ProxyJump chain (`a,b`), and an empty user means your local user name, as in OpenSSH.
- SSH connections (`setu-ssh`, built on russh): strict host-key checking against `~/.ssh/known_hosts` with a fingerprint trust prompt and a hard stop on changed keys; ssh-agent, default `~/.ssh` keys, key files with passphrase, and password auth; ProxyJump through a bastion; interactive shells with resize. The desktop app adds a "New SSH connection" dialog (Ctrl+Shift+O) and SSH tabs.
- Desktop app (Tauri 2 + xterm.js): opens your default shell on first launch, terminal tabs for any detected shell (shell picker), close/switch tabs with Ctrl+Shift+T, Ctrl+Shift+W and Ctrl+Tab, resizing that follows the window, and a status bar with shell and size.
- `setu-pty`: detects local shells (bash, zsh, fish, sh, PowerShell, Windows PowerShell, cmd and WSL distros) and runs them in a pseudo-terminal with input, output streaming, resize and exit status.
- Project foundation: community files, Cargo workspace with `setu-core`, CI on macOS, Linux and Windows, and the first architecture decision records.
