# Roadmap

Setu is built in five phases. Each phase closes with a **gate** that must pass before the next phase gets full effort. If a gate is missed, the phase is extended rather than the next one started early.

| Phase | Name | Gate |
| --- | --- | --- |
| P0 | Foundation | Repo, CI, community files, shell decision recorded |
| **P1** | **MVP (current)** | A new user connects to a real host, opens tabs and uploads a file in under 2 minutes, on macOS, Linux and Windows |
| P2 | Public 1.0 | Stable releases, package format v1, tunnels, server management, basic monitoring, AI assistant (opt-in) |
| P3 | Enterprise beta | Containers/Kubernetes, cloud, role workspaces, agents (levels 0–2) |
| P4 | Enterprise GA | Governance features (Enterprise edition, separate repo) |

Setu is currently maintained part-time, so the dates are intentionally left out. Progress is tracked in pull requests and the milestone for each phase.

## Phase 1: MVP

The MVP is deliberately small: **terminal, SSH, SFTP, vault and host list.** Anything else waits.

### Core scope (must ship for the P1 gate)

| # | Deliverable | Module | Notes |
| --- | --- | --- | --- |
| 1 | Foundation: community files, Cargo/pnpm monorepo, CI on 3 OSes | — | This is where we are |
| 2 | `setu-pty`: local shells via `portable-pty` (bash, zsh, fish, PowerShell, cmd, WSL detection) | 2 | Rust, tested headless |
| 3 | Desktop shell: Tauri 2 app, xterm.js (WebGL) terminal, tabs, opens a local shell on first launch | 1 | See ADR 0002 |
| 4 | `setu-ssh`: SSH via `russh`: keys, agent, password, ProxyJump, strict `known_hosts` verification | 3 | Integration tests against a containerised `sshd` |
| 5 | Host inventory: hosts file (TOML), folders and tags, import from `~/.ssh/config` | 6 | Plain file, Git-friendly |
| 6 | `setu-vault`: encrypted local vault, Argon2id + XChaCha20-Poly1305, auto-lock, OS keychain option | 13 | Security review before merge |
| 7 | Files: dual-pane SFTP browser, upload/download queue | 4 | Built on `russh-sftp` |
| 8 | `setu` CLI with `setu doctor` (key permissions, ssh-agent, config checks) | 9, 24 | Same core as the app |
| 9 | Release pipeline: signed nightly builds for macOS, Linux, Windows | — | Notarization and Authenticode |

### Stretch scope (P1 if time allows, otherwise early P2)

These are tagged P1 in the plan but are not required for the gate:

- Split panes, scrollback search, session restore, themes, command palette.
- Workspace shell with a basic editor (syntax highlighting, remote file edit).
- Basic Git panel (status, diff, commit).
- Import from PuTTY, MobaXterm, Termius, Tabby, Windows Terminal, Ansible inventory.
- Role picker with the system-administrator pack; install a package from a local file.

### Explicitly not in P1

Tunnels GUI, RDP/VNC/serial/Mosh, monitoring, server management, sync, plugins/WASM, AI features, anything from the Enterprise edition.

## Later phases

See [docs/README.md](docs/README.md) for the full module list and phase tags. Each module lands in the phase shown there; requests for later-phase features are welcome as issues and will be labelled with their phase.
