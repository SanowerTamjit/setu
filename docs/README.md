# Setu documentation

- [Architecture decision records](adr/)
- [Roadmap](../ROADMAP.md)
- [Editions: free vs paid](../EDITIONS.md)
- [Threat model](security/threat-model.md) and the [Phase 1 security review](security/phase1-review.md)
- [Feature flags](feature-flags.md)
- [How Setu is built (AI-assisted workflow)](development-workflow.md)

## Architecture in one paragraph

A Rust core (`crates/`) powers every client: the Tauri 2 desktop app (`apps/desktop`), the `setu` CLI and, later, plugins. The UI is TypeScript with xterm.js. Protocol and crypto code lives in Rust (`russh`, `portable-pty`, `rustls`), never in the web view. The Community build runs fully offline; Enterprise features live in a separate repository and talk to the core only through public interfaces.

## Modules and phases

Setu is planned as 25 modules. **C** = Community (this repo), **C+E** = free basics here with Enterprise extras elsewhere, **E** = Enterprise only (not in this repo).

| # | Module | Edition | Phase |
| --- | --- | --- | --- |
| 1 | Terminal core | C | P1 |
| 2 | Local shells | C | P1 |
| 3 | Connections (SSH in P1; Mosh, Telnet, serial, RDP, VNC later) | C | P1 / P2 |
| 4 | Files (SFTP in P1; cloud storage later) | C | P1 / P3 |
| 5 | Tunnels and networking | C | P2 |
| 6 | Host inventory | C+E | P1 |
| 7 | Server management | C+E | P2 |
| 8 | Monitoring | C+E | P2 |
| 9 | Automation and DevOps (incl. `setu` CLI) | C+E | P2–P3 |
| 10 | Containers and Kubernetes | C | P3 |
| 11 | Cloud integration | C+E | P3 |
| 12 | Database access (plugin) | C | P3 |
| 13 | Security and secrets (vault) | C+E | P1–P3 |
| 14 | Sync and collaboration | C+E | P2–P3 |
| 15 | Enterprise governance | E | P3–P4 |
| 16 | Ops editor and workspaces | C | P1 basics, P2 full |
| 17 | Network device management | C+E | P3 |
| 18 | Security operations workspace | C+E | P3 |
| 19 | IT service desk tools | C+E | P3 |
| 20 | Problems and health panel | C+E | P2 |
| 21 | Role packs and workspace templates | C+E | P2–P3 |
| 22 | Git and GitOps | C | P1 panel, P2–P3 rest |
| 23 | AI platform (opt-in, bring your own model) | C+E | P2–P4 |
| 24 | Plug and play | C+E | P1–P4 |
| 25 | Team workspace | C+E | P1–P4 |
