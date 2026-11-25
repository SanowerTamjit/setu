# Architecture

Setu is a Rust core with thin clients on top. Everything that touches a network, a secret, a file or a process lives in Rust; the TypeScript UI only draws and sends requests. The decisions behind this are recorded in [ADR 0002](../adr/0002-desktop-shell-tauri.md) (Tauri 2) and [ADR 0003](../adr/0003-monorepo-layout.md) (the monorepo layout).

```
┌──────────────────────────── apps/desktop ────────────────────────────┐
│  src/  (TypeScript, no framework, xterm.js)                          │
│     main.ts · terminal-tab.ts · files-tab.ts · workflows-view.ts …   │
│     ipc.ts  ← typed wrappers around every Rust command               │
│                       │  invoke() / Channel                          │
│  src-tauri/src/  (Tauri commands: the only bridge)                   │
│     terminals.rs · ssh.rs · files.rs · hosts.rs · vault.rs · …       │
└───────────────────────┬──────────────────────────────────────────────┘
                        │ plain Rust calls
┌──────────── crates/ ──┴──────────────────────────────────────────────┐
│ setu-pty   setu-ssh   setu-hosts   setu-vault   setu-workflow        │
│ setu-memory                     setu-core (paths, extension points)  │
└───────────────────────┬──────────────────────────────────────────────┘
                        │
                  crates/setu-cli  (the `setu` command, same crates)
```

## The crates

| Crate | What it owns | Notable files |
|---|---|---|
| `setu-core` | Where files live on each OS (`SETU_HOME`, XDG, `%APPDATA%`), the version, and the extension points (`extend.rs`) | `paths.rs`, `extend.rs` |
| `setu-pty` | Detecting local shells (bash, zsh, fish, sh, PowerShell, cmd, WSL distros) and running them in a pseudo-terminal (`portable-pty`) | `shells.rs`, `session.rs`, `launch.rs` |
| `setu-ssh` | SSH connections on `russh`: auth (agent, keys, password, FIDO2 keys through the agent), ProxyJump chains, strict `known_hosts` checking, interactive shells, SFTP with resumable transfers, key generation, tunnels (local, remote and SOCKS5 port forwarding) | `client.rs`, `known_hosts.rs`, `shell.rs`, `sftp.rs`, `tunnel.rs`, `keys.rs`, `keygen.rs` |
| `setu-net` | Network tools run from this computer: port check (TCP connect timing), DNS lookup, Wake-on-LAN, and the input checks every network tool uses | `lib.rs` |
| `setu-hosts` | The host inventory (`hosts.toml`) and importers: `ssh_config` (with `Include` and OpenSSH's first-match rules), PuTTY `.reg`, MobaXterm, CSV, Ansible INI/YAML | `model.rs`, `store.rs`, `ssh_config.rs`, `sources.rs`, `import.rs` |
| `setu-vault` | The encrypted vault (Argon2id + XChaCha20-Poly1305) and the optional OS keychain unlock | `vault.rs`, `file.rs`, `keychain.rs` |
| `setu-workflow` | Workflows: the model and its checks, the plan (which steps run when), the async engine over a `Runner` trait, files and run history, templates, and the task board | `model.rs`, `plan.rs`, `engine.rs`, `store.rs`, `board.rs` |
| `setu-memory` | Work memory: the command store, secret masking, and the snippet and workflow suggestions | `store.rs`, `redact.rs`, `suggest.rs` |
| `setu-cli` | The `setu` binary: `doctor`, `hosts`, `hosts import`, `paths` | `main.rs`, `doctor.rs`, `hosts.rs` |

Crates do not know about Tauri. Anything a crate needs from the outside (the environment, a runner, a key store) comes in through a small trait, so it can be tested without a window, a server or a keychain: `setu_core::paths::Env`, `setu_workflow::Runner`, `setu_vault::keychain::KeyStore`.

## The desktop app

### `src-tauri`: the bridge

`apps/desktop/src-tauri/src/lib.rs` builds the Tauri app: it registers one state object per area (`Terminals`, `Hosts`, `VaultState`, `Files`, `Workflows`, `Memory`, …) and every command in `generate_handler!`. Each module wraps one area:

| Module | Commands for |
|---|---|
| `terminals.rs`, `windows.rs` | Local shells, terminal I/O, moving tabs between windows |
| `ssh.rs`, `keys.rs` | SSH connect and shells, host-key trust, SSH keys, `known_hosts` |
| `hosts.rs` | Saved hosts and imports |
| `files.rs`, `remote_edit.rs` | SFTP browsing and transfers, editing remote files locally |
| `vault.rs` | Creating, unlocking, locking the vault; the keychain option |
| `session.rs` | `session.json` (tabs to restore) and `settings.json` |
| `workflows.rs` | Workflows, runs, history and the task board |
| `tunnels.rs` | Saved tunnels: start, stop, live status, reconnect after a drop |
| `nettools.rs` | Port check, DNS lookup, ping and Wake-on-LAN, here or through a saved host |
| `memory.rs` | Work memory |
| `packs.rs`, `color_schemes.rs`, `prompts.rs` | Packs, colour scheme import, prompt tool detection |
| `extensions.rs` | The settings page a build with extensions adds |

The full list, with what each command checks, is in [IPC commands](ipc-commands.md).

Streams use Tauri `Channel`s rather than global events: a terminal's output and exit code, transfer progress and workflow run updates go to the window that asked for them. When a tab moves to another window (`window_detach`, `terminal_hold`, `terminal_attach`), Rust holds the output until the new window attaches, so nothing restarts and nothing is lost.

### `src`: the UI

Plain TypeScript and DOM, no framework; Vite builds it and the Rust crate embeds the result (so build the UI before `cargo build`).

| Area | Files |
|---|---|
| Start-up, rail, views, the command registry | `main.ts`, `commands.ts`, `palette.ts`, `keymap.ts` |
| Layouts | `layouts.ts` (the switch), `studio.ts` (Studio's title bar and navigator) |
| Terminals | `tab.ts`, `terminal-tab.ts`, `pane-tab.ts`, `find-bar.ts`, `secret-mask.ts` + `secrets.ts`, `paste-guard.ts`, `session.ts` |
| SSH, hosts, keys | `ssh-dialog.ts`, `hosts-home.ts`, `hosts-panel.ts`, `host-dialog.ts`, `import-dialog.ts`, `jump-map.ts`, `key-manager.ts`, `known-hosts.ts` |
| Files | `files-tab.ts`, `permissions-dialog.ts` |
| Vault | `vault-dialog.ts` |
| Workflows | `workflows-view.ts`, `board-view.ts`, `memory-view.ts`, `work-memory.ts` |
| Tunnels | `tunnels-view.ts` (view and dialog), `tunnel-form.ts` (form checks and labels, tested in Node) |
| Network tools | `net-view.ts` (view), `net-tools.ts` (input checks and result words, tested in Node) |
| Settings | `settings.ts` (the model, `sanitize`), `settings-dialog.ts` and one `settings-*.ts` per page, `features.ts`, `themes.ts`, `prompt-themes.ts`, `packs.ts` |
| Shared | `ipc.ts` (typed wrappers for every Rust command), `dom.ts`, `icons.ts`, `styles.css` |

Everything the person can do is a **command** (`registerCommand` in `commands.ts`) with an id, a title and a category. The palette lists commands; `keymap.ts` maps keys to command ids per profile (Default, Vim, Emacs); buttons and menus call the same ids. A new feature adds commands rather than its own key handling.

Styles are design tokens on `:root[data-theme=…]`, with Studio overrides under `:root[data-layout="studio"]`. See [How-to recipes](how-to.md#add-a-theme).

## Data on disk

Every file lives in the config folder from `setu_core::paths` (`$SETU_HOME/config` in portable mode). The user-facing table is in [Settings and your files](../user/settings.md#where-setu-keeps-its-files). Rules for all of them:

- Files with personal data are mode 0600 (folders 0700) and written atomically: a temporary file, then a rename.
- Files carry a `version` so their format can change later. `hosts.toml` with a higher version than this build understands is never overwritten; new formats should follow that rule.
- What comes back from disk is parsed and checked again (sizes, ids, lengths). A file that fails to load is reported and left alone, not overwritten.
- No secret is ever in a plain file: only `vault.json`, encrypted.

## Security boundaries

The web view is treated as untrusted. Rust commands check their inputs themselves (paths stay inside their folder, sizes are capped, ids are validated), secrets go into Rust and never come back out, and workflows run only what Rust reads back from disk. Server output, file names, host keys and imported files are untrusted input. The full picture is in the [threat model](../security/threat-model.md); the rules every change is checked against are in [`CLAUDE.md`](../../CLAUDE.md#security-checklist-check-every-change-against-it).

## Extension points

A build of Setu can add an audit trail, a command policy, an access policy, a session recorder and a settings page, all as Rust code compiled in. The open-source app passes `Extensions::default()`, where every hook does nothing. See [Extending Setu](extending.md).
