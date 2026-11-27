# Roadmap

Setu is built in five phases. Each phase closes with a **gate** that must pass before the next phase gets full effort. If a gate is missed, the phase is extended rather than the next one started early.

| Phase | Name | Gate |
| --- | --- | --- |
| P0 | Foundation | Repo, CI, community files, shell decision recorded |
| P1 | MVP (done) | A new user connects to a real host, opens tabs and uploads a file in under 2 minutes, on macOS, Linux and Windows |
| **P2** | **Public 1.0 (current)** | Stable releases, package format v1, tunnels, server management, basic monitoring, AI assistant (opt-in) |
| P3 | Enterprise beta | Containers/Kubernetes, cloud, role workspaces, agents (levels 0–2) |
| P4 | Enterprise GA | Governance features (Enterprise edition, separate repo) |

Setu is currently maintained part-time, so the dates are intentionally left out. Progress is tracked in pull requests and the milestone for each phase.

## Phase 2: Public 1.0

Phase 2 turns the MVP into a 1.0 that people can run every day: the networking, server and monitoring tools ops people reach for next, an opt-in assistant, and stable releases. It is built in small pull requests, roughly in this order, each behind a feature flag until it has had some use.

| # | Deliverable | Module | Notes |
| --- | --- | --- | --- |
| 1 | Tunnel engine: local (`-L`), remote (`-R`) and dynamic SOCKS5 (`-D`) forwarding in `setu-ssh` | 5 | Tested against a real `sshd` |
| 2 | Tunnels screen: saved tunnel profiles per host, start and stop, live connection counts, auto-reconnect | 5 | Loopback only unless you choose otherwise |
| 3 | Network tools: ping, DNS lookup, port check, TLS certificate check, Wake-on-LAN | 5 | Run from this computer or through a host |
| 4 | Monitoring: agentless live CPU, memory, disk and network for an SSH session, with threshold alerts | 8 | Reads `/proc` and standard tools; nothing installed on the server |
| 5 | Server management: services (systemd) with logs, processes, disk usage, `authorized_keys` | 7 | Every change shows the exact command and asks first |
| 6 | Problems and health panel: findings from checks across hosts in one list | 20 | Built on monitoring and workflow checks; read-only check over SSH |
| 7 | AI assistant (opt-in, off by default, bring your own model) with secret redaction | 23 | Never runs a command without you |
| 8 | Package format v1 and `setu://install` links for packs | 21, 24 | Data only, signed |
| 9 | Stable release channel and auto-update | — | Signed builds on all three platforms |

The P2 gate: a stable, signed 1.0 on macOS, Linux and Windows, with tunnels, monitoring and server management usable without reading the docs.

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

### Also delivered in P1

- GPU-rendered terminal with ligatures, resumable transfer queue (pause, resume, retry), editing a remote file in a local editor, a remote permissions editor, and YubiKey/FIDO2 SSH keys.
- Visual workflows (moved forward from module 9, automation): a canvas of steps that run commands on saved hosts in order or in parallel, with approvals, failure branches, a live run view and run history (`crates/setu-workflow`, `docs/user/workflows.md`). Scheduled runs, variables and fleet-wide jobs with dry-run stay in P2–P3.

### Explicitly not in P1

Tunnels GUI, RDP/VNC/serial/Mosh, monitoring, server management, sync, plugins/WASM, AI features, anything from the Enterprise edition. (Tunnels, monitoring, server management and the assistant are Phase 2, above.)

## Later phases

See [docs/README.md](docs/README.md) for the full module list and phase tags. Each module lands in the phase shown there; requests for later-phase features are welcome as issues and will be labelled with their phase.
