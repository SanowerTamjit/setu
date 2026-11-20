# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Minimum supported Rust version is now 1.89 (needed by russh 0.65, which fixes RUSTSEC-2026-0495).
- The desktop app uses xterm's DOM renderer on Linux: the WebGL renderer showed new output one frame late under WebKitGTK.

### Added

- Saved hosts (`setu-hosts`): a hosts sidebar with folders, tags and search; double-click (or Enter) to connect; add, edit and delete hosts. Hosts are stored in a plain `hosts.toml` without secrets (see `docs/hosts.md`). **Import** brings in hosts from `~/.ssh/config`, following `Include` and OpenSSH's matching rules. Ctrl+Shift+H shows the sidebar and focuses search.
- SSH connections accept a ProxyJump chain (`a,b`), and an empty user means your local user name, as in OpenSSH.
- SSH connections (`setu-ssh`, built on russh): strict host-key checking against `~/.ssh/known_hosts` with a fingerprint trust prompt and a hard stop on changed keys; ssh-agent, default `~/.ssh` keys, key files with passphrase, and password auth; ProxyJump through a bastion; interactive shells with resize. The desktop app adds a "New SSH connection" dialog (Ctrl+Shift+O) and SSH tabs.
- Desktop app (Tauri 2 + xterm.js): opens your default shell on first launch, terminal tabs for any detected shell (shell picker), close/switch tabs with Ctrl+Shift+T, Ctrl+Shift+W and Ctrl+Tab, resizing that follows the window, and a status bar with shell and size.
- `setu-pty`: detects local shells (bash, zsh, fish, sh, PowerShell, Windows PowerShell, cmd and WSL distros) and runs them in a pseudo-terminal with input, output streaming, resize and exit status.
- Project foundation: community files, Cargo workspace with `setu-core`, CI on macOS, Linux and Windows, and the first architecture decision records.
