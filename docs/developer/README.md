# Setu developer guide

For people who build, change or review Setu. If you only want to use it, see the [user guide](../user/README.md).

Setu is a Rust core (`crates/`) with a Tauri 2 desktop app (`apps/desktop`) whose UI is plain TypeScript with xterm.js. Protocol, crypto and file work stay in Rust; the web view only draws. Setu holds people's server passwords, keys and production access, so the [security rules](../../CLAUDE.md#security-checklist-check-every-change-against-it) apply to every change.

## Start here

1. [Building and running](setup.md): tools, first build, `just` recipes, throwaway data, a local `sshd` for testing.
2. [Architecture](architecture.md): the crates, the desktop app, how the UI talks to Rust, files on disk.
3. [Contributing](../../CONTRIBUTING.md): pull requests, sign-off (DCO), commit messages.

## Guides

| Page | What it covers |
|---|---|
| [Architecture](architecture.md) | Crates and what each owns, the Tauri bridge, the UI modules, data on disk, security boundaries |
| [IPC commands](ipc-commands.md) | Every command the web view can call, and the rules for adding one |
| [Testing](testing.md) | Rust unit and integration tests (real `sshd`), UI tests with Vitest, what CI runs |
| [How-to recipes](how-to.md) | Add a command, a setting, a flag, an IPC command, a theme, an importer, a snippet, an extension point |
| [Feature flags](feature-flags.md) | Stages, the current flags, adding and retiring one |
| [Extending Setu](extending.md) | Extension points for builds that add an audit trail, policies, session recording or a settings page |
| [Nightly builds and signing](releasing.md) | The nightly pipeline, verifying downloads, code-signing secrets, versioned releases |
| [How Setu is built](development-workflow.md) | The AI-assisted workflow and the maintainer's review checklist |
| [Keeping the docs up to date](docs-guide.md) | Which page to update for which change, writing style, screenshots, link check |

## Security

- [Threat model](../security/threat-model.md): what is protected, from whom, and where each protection lives.
- [Phase 1 security review](../security/phase1-review.md).
- [SECURITY.md](../../SECURITY.md): reporting a vulnerability.

## Decisions and plans

- [Architecture decision records](../adr/)
- [Roadmap](../../ROADMAP.md) and [modules by phase](../README.md#modules-and-phases)
- [Editions](../../EDITIONS.md): what is in the open-source Community edition
- [Governance](../../GOVERNANCE.md)

## Rules in one screen

The full list is in [`CLAUDE.md`](../../CLAUDE.md) (it is written for AI assistants, and is the best summary for humans too):

- Small pull requests, one topic, based on `master`; commits signed off with an `area: summary` subject.
- Tests with every change; a bug fix comes with a test that fails without it.
- User-visible change: a line in `CHANGELOG.md` under **Unreleased**, and the docs updated in the same PR.
- New or risky features behind a flag; security fixes never.
- No new dependency without a reason; licenses must be allowed by `deny.toml`.
- UI changes come with screenshots.
- No `unsafe`; no `unwrap`/`expect` outside tests.
