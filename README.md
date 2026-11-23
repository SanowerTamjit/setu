# Setu

**Setu** (সেতু, "bridge") is a free, open-source, cross-platform IDE for operations work: system engineering, administration, DevOps/SRE, SecOps, NetOps and IT operations. It runs on macOS, Linux and Windows.

It aims to replace the toolbox most operators juggle every day (a terminal, an SFTP client, a tunnel manager, a password vault, a text editor, a folder of scripts and a wiki of runbooks) with one fast, local-first workbench.

> **Status: pre-alpha.** Setu is in Phase 1 (MVP). Nothing is usable yet. See [ROADMAP.md](ROADMAP.md) for what is being built and in what order.

## What Phase 1 delivers

- A local terminal on first launch, with tabs (bash, zsh, fish, PowerShell, cmd, WSL).
- SSH connections with keys, agent, ProxyJump and strict host-key checking.
- A dual-pane SFTP file browser with upload and download.
- A host list with folders and tags, imported from your existing `~/.ssh/config`.
- An encrypted local vault for passwords and key passphrases (Argon2id + AEAD).

No account, no telemetry, no cloud required.

## Principles

- **Works on first launch.** No setup wizard, sensible defaults, a working terminal immediately.
- **Keep existing habits.** Reads your SSH config, keys and shells; nothing to migrate.
- **Local-first and plain files.** Settings, hosts and workspaces are files you can keep in Git.
- **Security is the product.** Setu holds server credentials, so it verifies host keys strictly, encrypts secrets at rest and never logs them.
- **Open core, stable line.** Everything an individual needs is free and stays free. See [EDITIONS.md](EDITIONS.md).

## Repository layout

```
apps/desktop      Tauri 2 desktop app (UI shell)              (Phase 1)
crates/           Rust core: setu-core, setu-pty, setu-ssh, setu-hosts, setu-vault, setu-cli, ...
packages/ui       TypeScript UI components (xterm.js terminal)  (Phase 1)
plugins/          First-party plugins                            (Phase 2+)
docs/             Documentation and architecture decision records (ADRs)
```

## Building from source

Requirements: Rust (the version in `rust-toolchain.toml`), Node.js `^22.12.0 || ^24.0.0 || >=26.0.0` with pnpm, and on Linux the WebKitGTK development packages (`libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev` on Debian/Ubuntu).

```sh
pnpm install
pnpm desktop            # run the desktop app with hot reload

pnpm build:ui           # build the UI once (needed before plain cargo builds)
cargo test --workspace

cargo install --path crates/setu-cli   # the `setu` command (see docs/cli.md)
```

Nightly builds for macOS, Linux and Windows: see [docs/releasing.md](docs/releasing.md).

## Contributing

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) to get started, and look for issues labelled `good first issue`. By participating you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).

Found a security problem? Please do not open a public issue; follow [SECURITY.md](SECURITY.md).

## License

Setu Community is licensed under the [Apache License 2.0](LICENSE). "Setu" and the Setu logo are trademarks of the project; see [GOVERNANCE.md](GOVERNANCE.md#trademark).
