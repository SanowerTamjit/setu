# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Security

- Host names that could not be real addresses (with spaces, commas or `*`, `?`, `!`, `|`, `@`, `[`, `]`, `#`) are refused when importing, connecting and trusting a host key. Before, a crafted name in an imported file could make "trust this host" add a key trusted for every host.
- Only a host key that a server actually presented can be trusted from the trust prompt.
- On Windows, file names from a server like `C:name` or `name:stream` are refused, so a download cannot land outside the chosen folder.
- Files edited in a local editor are copied to a private per-user folder (`$XDG_RUNTIME_DIR` or `~/.cache/setu/edit` on Linux) instead of the shared `/tmp`, and with no editor set, file types that would run instead of open (`.bat`, `.exe`, `.desktop`, `.command`, …) are refused.
- Pack snippets must be one line, so picking one can never run a command before you press Enter.
- Release builds: GitHub Actions pinned to commit SHAs, signing secrets never given to pull-request builds, an SBOM with every nightly, and daily dependency advisory checks. See `docs/security/phase1-review.md`.

### Changed

- A more restrained look: the Dark and Light themes now use neutral graphite and white surfaces with a blue accent instead of amber, and host cards, avatars, tags and buttons are quieter (one neutral avatar style, Connect as a secondary button, flat cards, smaller corners). High contrast is unchanged.
- A new look: a side rail for Hosts, Terminals, Keys, Snippets and the Vault; a slimmer host sidebar; rounded tabs; and one dialog layout (spacing, type sizes, a close button in the corner) for every dialog, built on shared design tokens across the Dark, Light and High contrast themes.
- The hosts sidebar's **Import** now offers PuTTY, MobaXterm, CSV and Ansible as well as `~/.ssh/config`.
- Files (SFTP) opens with Ctrl+Shift+E instead of Ctrl+Shift+F, which is now find, as in most terminals.
- Minimum supported Rust version is now 1.89 (needed by russh 0.65, which fixes RUSTSEC-2026-0495).
- The desktop app picks the terminal renderer itself (GPU on Windows and macOS, DOM on Linux, where WebGL showed new output one frame late under WebKitGTK); Settings → Terminal → **Renderer** can force either.
- The Files tab's server-side buttons are now icons (Edit, Permissions, New folder, Rename, Delete) with tooltips.

### Added

- Two new themes, Ember (warm charcoal with an amber accent) and Sandstone (soft paper tones with a teal accent), each with its own terminal palette and Studio skin. Pick them in Settings → Appearance or from the palette (`Theme`).
- **Workflows**: a visual builder for the jobs you run on servers. Draw steps that run a command on saved hosts (by name or `tag:`), wait for your approval, or pause; link them to run on success, on failure or always, and unlinked steps run at the same time. Run it to see each step light up, approve from the canvas, stop it, and read every host's output; past runs stay in history. Starts from templates (health check, rolling restart with approval, database backup). Workflows are TOML files you can keep in Git. Works in the Classic and Studio layouts; turn it off in Settings → Features. See `docs/workflows.md`.
- **Task board**: a **Board** tab in Workflows with To do, Doing and Done columns for ops work. Cards hold notes, a priority, a due date, the hosts they are about and the workflow that does the job, showing its last run; drag them between columns or move them with Alt+arrow keys. Saved to `board.toml` as you go. See `docs/workflows.md`.
- A second window layout, **Studio**, with a futuristic look (glass panels, neon edges, HUD-style labels), next to the current one (now called **Classic**): open sessions listed on the left with their connection state, a search box for hosts, commands and snippets in a new title bar, and a header over the terminal with Files, split, find and new-window buttons. Pick it in Settings → Appearance → **Layout**, from the command palette, or with the layout button; switching keeps your sessions running. See `docs/terminal.md`.
- Settings → **Features**: turn newer features (security keys, remote edit, paste guard) off or on. See `docs/feature-flags.md`.
- Paste guard: pasting several lines into a shell that would run each line at once shows them and asks first.

- GPU-rendered terminal (WebGL) with font ligatures: a Renderer setting (Automatic, GPU, DOM), a Font setting, and ligatures on by default for fonts that have them; the status bar shows which renderer is in use. See `docs/terminal.md`.
- Files: **Pause**, **Resume** and **Retry** copies (one or all). Each file is written as `name.setu-part` and renamed when complete; resuming continues from the byte where it stopped, like `rsync --partial`. See `docs/files.md`.
- Files: edit a remote file in your own editor (**Edit** or F4). Every save is written back in place, keeping owner and permissions, and a save is refused if the server's copy changed meanwhile. See `docs/files.md`.
- Files: a permissions editor for remote items: read, write and execute for owner, group and others, the special bits, octal mode, user and group, optionally for everything inside a folder, with the matching `chmod`/`chown` shown. See `docs/files.md`.
- YubiKey and other FIDO2 security keys: make `ed25519-sk` and `ecdsa-sk` keys (optionally stored on the device or requiring the PIN each time), load keys stored on the device, and sign in with them through ssh-agent. See `docs/ssh-keys.md`.
- Hosts view: every saved host as a card with name, address, jump host, folder, tags and how many panes are open, grouped by folder, with search, tag filters, one-click Connect and Files, and keyboard control (Enter connects, E or F2 edits, arrow keys move). Open it from the rail or with "Go to hosts" in the palette. See `docs/hosts.md`.
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
