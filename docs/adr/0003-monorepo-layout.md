# 0003. Monorepo layout and toolchain

- Status: Accepted
- Date: 2026-11-20

## Context

The desktop app, CLI and future plugins share one Rust core. Contributors should build everything with one clone.

## Decision

One public monorepo:

```
apps/desktop    Tauri 2 app (TypeScript UI + src-tauri)
crates/         Rust crates, one per concern: setu-core, setu-pty, setu-ssh, setu-vault, setu-hosts, setu-cli
packages/ui     Shared TypeScript UI components
plugins/        First-party plugins (Phase 2+)
docs/           Documentation and ADRs
```

- Rust: a Cargo workspace at the root, stable toolchain, MSRV 1.85 (raised to 1.89 in PR 4 for russh 0.65, which fixes RUSTSEC-2026-0495), edition 2021, `unsafe_code` denied by default (a crate that needs it opts out with a justification).
- TypeScript: a pnpm workspace (added with the desktop app).
- `Cargo.lock` is committed, because the repository ships applications.
- Dependencies are checked by `cargo-deny` for licenses compatible with Apache-2.0 and for security advisories.

Enterprise code lives in a separate private repository and depends on these crates; nothing here may depend on it.

## Consequences

Crates can be tested, reviewed and owned separately (security owners for `setu-ssh` and `setu-vault`). The CLI and app cannot drift apart because they call the same core.
