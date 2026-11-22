# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- The hosts sidebar's **Import** now offers PuTTY, MobaXterm, CSV and Ansible as well as `~/.ssh/config`.
- Files (SFTP) opens with Ctrl+Shift+E instead of Ctrl+Shift+F, which is now find, as in most terminals.
- Minimum supported Rust version is now 1.89 (needed by russh 0.65, which fixes RUSTSEC-2026-0495).
- The desktop app uses xterm's DOM renderer on Linux: the WebGL renderer showed new output one frame late under WebKitGTK.

### Added

- Command palette (Ctrl+Shift+P): one fuzzy search over commands, saved hosts, shells and profiles, open tabs, themes, keybinding profiles and snippets, with recent commands first and shortcuts shown. See `docs/terminal.md`.
- Themes: Dark, Light and High contrast for the whole app and the terminal, plus the terminal font size, in a new Settings dialog (Ctrl+,). Preferences are saved in `settings.json`.
- Keybinding profiles: Default, Vim (Ctrl+Space prefix, h j k l between panes) and Emacs (Ctrl+X 2, 3, o, 0; Alt+X for commands), with the pending prefix shown in the status bar. See `docs/terminal.md`.
- Tabs in their own window: drag a tab out of the window, or press Ctrl+Shift+N, and it moves to a new window with its running shells and scrollback.
- Shell profiles: a detected shell (including a WSL distro) with its own starting folder, arguments and environment variables, and a choice of shell for new tabs, in Settings → Shells.
- SSH key manager: lists keys in `~/.ssh` with fingerprint, passphrase and agent state; makes Ed25519, ECDSA or RSA keys (mode 0600, never overwriting); copies public keys; adds keys to ssh-agent. See `docs/ssh-keys.md`.
- Known hosts manager: lists, filters and forgets `known_hosts` entries, keeping `known_hosts.old`. See `docs/ssh-keys.md`.
- Secrets in terminal output are covered with a bar (cloud keys, tokens, JWTs, passwords in URLs and assignments, private key blocks), also in `less` and `vim`; copying still gives the real text. On by default. See `docs/terminal.md`.
- Copy and paste in the terminal with Ctrl+Shift+C and Ctrl+Shift+V.
- Host import from PuTTY (`.reg` export), MobaXterm (`.mxtsessions`), Termius or any CSV, and Ansible inventories (INI and YAML), keeping folders; passwords are never read. A jump-host map shows which hosts are reached through which bastions. See `docs/hosts.md`.
- Roles and packs: a first-launch role picker turns on the built-in system administrator pack of 25 snippets; snippets are typed into the terminal from the palette, never run. Settings → Packs installs a pack from a local `.json` file after showing what it asks for. See `docs/packs.md`.
- Tabs come back after a restart: Setu reopens your tabs with their splits, divider positions and focused pane. Local panes start a fresh shell; SSH panes wait for Enter, then reconnect without the dialog unless a password, passphrase or host-key check needs you, and split SSH panes log in once. Saved in `session.json` (no secrets; mode 0600 on Unix), which `setu paths` now lists. See `docs/terminal.md`.
- Split panes: split a terminal tab side by side (Ctrl+Shift+D) or stacked (Ctrl+Alt+Shift+D), as often as you like; drag the dividers to resize, move focus with Alt+Arrow keys, and close a pane with Ctrl+Shift+W. Splitting an SSH pane opens another shell over the same connection, with no new login. See `docs/terminal.md`.
- Find in a terminal's scrollback (Ctrl+Shift+F): matches highlighted as you type, a match counter, next and previous, and match case, whole word and regular expression options. See `docs/terminal.md`.
- Vault: **Unlock automatically on this computer** keeps the vault key in the macOS Keychain, Windows Credential Manager or the Linux Secret Service, so the vault opens at start-up and after the idle timeout without the master password. Off by default; **Lock now** still requires the password, and turning it off deletes the stored key. See `docs/vault.md`.
- Nightly builds: a scheduled workflow builds the desktop app (`.dmg`, `.msi`, `-setup.exe`, `.AppImage`, `.deb`, `.rpm`) and the `setu` CLI for macOS (universal), Windows and Linux, and publishes them as the rolling `nightly` pre-release with `SHA256SUMS` and GitHub build-provenance attestations. macOS signing and notarization and Windows Authenticode switch on when their secrets are added; until then macOS builds are ad-hoc signed. See `docs/releasing.md`.
- `setu` command line (`setu-cli`): `setu doctor` checks private-key permissions and passphrases, ssh-agent and the keys it holds, `~/.ssh/config` permissions and missing `IdentityFile`s, `known_hosts`, and Setu's saved hosts and vault file, with a fix for each problem (`--json` for scripts; exit code 1 when something blocks SSH). `setu hosts` lists and searches saved hosts, and `setu hosts import` previews or saves hosts from `~/.ssh/config`. See `docs/cli.md`.
- Files tabs (SFTP): two panes, your computer and an SSH host, with upload and download by button or drag and drop, folder copies, a transfer queue with progress, speed and cancel, and new folder, rename and delete on the host. Opens from an SSH tab (Ctrl+Shift+E) or from a host's ⇅ button, reusing that connection. See `docs/files.md`.
- Encrypted vault (`setu-vault`): saves SSH passwords and key passphrases under a master password (Argon2id + XChaCha20-Poly1305), locks itself after 15 minutes idle, and fills them in when you connect, so saved hosts connect without asking. "Save in vault" in the connect dialog, a vault indicator in the status bar, and a vault dialog to remove secrets, lock, or change the master password. See `docs/vault.md`.
- Saved hosts (`setu-hosts`): a hosts sidebar with folders, tags and search; double-click (or Enter) to connect; add, edit and delete hosts. Hosts are stored in a plain `hosts.toml` without secrets (see `docs/hosts.md`). **Import** brings in hosts from `~/.ssh/config`, following `Include` and OpenSSH's matching rules. Ctrl+Shift+H shows the sidebar and focuses search.
- SSH connections accept a ProxyJump chain (`a,b`), and an empty user means your local user name, as in OpenSSH.
- SSH connections (`setu-ssh`, built on russh): strict host-key checking against `~/.ssh/known_hosts` with a fingerprint trust prompt and a hard stop on changed keys; ssh-agent, default `~/.ssh` keys, key files with passphrase, and password auth; ProxyJump through a bastion; interactive shells with resize. The desktop app adds a "New SSH connection" dialog (Ctrl+Shift+O) and SSH tabs.
- Desktop app (Tauri 2 + xterm.js): opens your default shell on first launch, terminal tabs for any detected shell (shell picker), close/switch tabs with Ctrl+Shift+T, Ctrl+Shift+W and Ctrl+Tab, resizing that follows the window, and a status bar with shell and size.
- `setu-pty`: detects local shells (bash, zsh, fish, sh, PowerShell, Windows PowerShell, cmd and WSL distros) and runs them in a pseudo-terminal with input, output streaming, resize and exit status.
- Project foundation: community files, Cargo workspace with `setu-core`, CI on macOS, Linux and Windows, and the first architecture decision records.
