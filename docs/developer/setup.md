# Building and running Setu

## What you need

| Tool | Version | Notes |
|---|---|---|
| Rust | stable (`rust-toolchain.toml`), at least 1.89 | Install [rustup](https://rustup.rs); the toolchain file picks stable with rustfmt and clippy |
| Node.js | `^22.12.0`, `^24.0.0` or `>=26.0.0` | Matches what Vite and Vitest need |
| pnpm | 10 (from `packageManager` in `package.json`) | `corepack enable` installs it |
| [just](https://just.systems) | any | Optional: every recipe is a plain command you can type |
| Linux only | `libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev` | Debian/Ubuntu names; the WebKitGTK web view Tauri uses |
| OpenSSH (`sshd`, `ssh-keygen`) | any | Optional, for the SSH integration tests (skipped without it) |

## First build

```sh
git clone https://github.com/SanowerTamjit/setu
cd setu
just setup            # pnpm install + build the UI (the desktop crate embeds it)
just app              # run the desktop app with hot reload
```

`just setup` must run before any `cargo build`: the desktop crate embeds the built UI from `apps/desktop/dist`, and fails to compile without it.

## Everyday commands

| Command | What it runs | When |
|---|---|---|
| `just app` | `pnpm desktop` (Tauri dev: Vite with hot reload + the Rust app) | Working on the app |
| `just quick` | `cargo fmt --check`, UI typecheck, UI unit tests | Seconds; also the pre-commit hook |
| `just test` | All Rust tests and UI tests | Before pushing |
| `just check` | Format, UI, clippy (`-D warnings`), all tests, `cargo deny`, `pnpm audit` | Before opening a PR: what CI runs |
| `just cli doctor` | `cargo run -p setu-cli -- doctor` | Trying the CLI |
| `just fmt` | `cargo fmt --all` | |
| `just install-hooks` | Points git at `scripts/hooks` | Once per clone |

The hooks refuse a commit that adds something shaped like a private key or token, unformatted Rust, a failing UI typecheck or UI test (only when those files changed), or a message without `Signed-off-by` (the DCO; use `git commit -s`). Skip once with `git commit --no-verify`.

## Running against throwaway data

Set `SETU_HOME` to keep every Setu file in one folder, away from your real hosts, vault and settings:

```sh
SETU_HOME=/tmp/setu-dev just app
```

Files then live in `/tmp/setu-dev/config`. To skip the first-launch role question, put `{"version":1,"role":"sysadmin"}` in `config/settings.json`.

For SSH work, a local `sshd` is the quickest server: run one on a spare port (say 2222) with its own host key, and add its key to `~/.ssh/known_hosts` with `ssh-keyscan -p 2222 127.0.0.1`, or trust it from the app. `crates/setu-ssh/tests/sshd.rs` shows a complete minimal `sshd_config`. On Linux, a locked account cannot log in with `UsePAM no`; give the test user a usable password field (`usermod -p '*' user`) and use key auth.

## The CLI

```sh
cargo run -p setu-cli -- doctor
cargo install --path crates/setu-cli     # puts `setu` on your PATH
```

## Release builds

`pnpm --filter @setu/desktop tauri build` makes installers for the system you are on. The nightly pipeline that builds all three systems is described in [Nightly builds and signing](releasing.md).

## Tips

- Editing a `.ts` file while the app runs makes it reload to the terminals view. Re-open the view you were testing.
- On Linux the terminal draws with the DOM renderer by default (WebKitGTK draws WebGL output one frame late on some setups). Force GPU in Settings → Terminal to test WebGL.
- In a headless Linux container, run the app under `Xvfb`; `xdotool search --onlyvisible --name Setu` finds the window for screenshots.
