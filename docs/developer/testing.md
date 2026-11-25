# Testing

Every change comes with tests, and a bug fix comes with a test that fails without the fix. `just check` runs everything CI runs.

## Rust

```sh
cargo test --workspace                 # everything
cargo test -p setu-hosts               # one crate
cargo test -p setu-ssh known_hosts     # tests whose name contains "known_hosts"
```

- **Unit tests** sit next to the code in `#[cfg(test)] mod tests`. Most logic is tested this way: parsers (`ssh_config`, PuTTY, Ansible, colour schemes), validation, the workflow plan and engine (with a fake `Runner`), secret masking, path resolution (with a fake `Env`).
- **Integration tests** are in each crate's `tests/` folder:
  - `crates/setu-ssh/tests/sshd.rs` starts a real OpenSSH `sshd` on a free localhost port with throwaway keys, and tests connecting, host-key checks, ProxyJump, shells and SFTP against it. It is skipped, with a message, when `sshd` or `ssh-keygen` is not installed (for example on Windows CI).
  - `crates/setu-pty/tests/session.rs` runs real local shells.
  - `crates/setu-cli/tests/cli.rs` runs the `setu` binary against a temporary `SETU_HOME`.
  - `crates/setu-vault/tests/os_keychain.rs` uses the real OS keychain and only runs when asked: `SETU_TEST_OS_KEYCHAIN=1 cargo test -p setu-vault --features os-keychain`, inside a desktop session.
- Tests never use your real files: use `tempfile`, a fake `Env`, or `SETU_HOME`. Tests that need a key or a vault make throwaway ones at run time; never commit a real key, even a test one (the pre-commit hook refuses private key blocks).
- Vault tests use `KdfParams::insecure_fast()` so Argon2 does not slow the suite.
- `unwrap` and `expect` are fine in tests, and only there.

## The UI

UI unit tests are [Vitest](https://vitest.dev) files in `apps/desktop/test/`. They run in Node, without a browser or the app.

```sh
pnpm --filter @setu/desktop test              # once
pnpm --filter @setu/desktop test:watch        # re-run on save
pnpm --filter @setu/desktop typecheck
```

Keep logic out of DOM code so it can be tested: `secrets.ts` (secret detection), `paste-guard.ts`, `keymap.ts`, `layouts.ts`, `session.ts` and `settings.ts` (`sanitize`) are tested this way. For code that calls Rust, mock `@tauri-apps/api/core` before importing it:

```ts
import { vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string) => (cmd === "settings_load" ? null : undefined)),
}));

const { settings } = await import("../src/settings");
```

`apps/desktop/test/settings.test.ts` is a full example.

## Trying it in the real app

Unit tests do not show that a screen works. For UI changes, run the app (`just app`), go through the change by hand, and take screenshots for the pull request: in all three base themes (Dark, Light, High contrast) when it is visual, and in both layouts when it touches the window. [Building and running](setup.md#running-against-throwaway-data) shows how to use throwaway data and a local `sshd`.

## What CI runs

`.github/workflows/ci.yml`, on every pull request and on `master`:

| Job | What |
|---|---|
| Format | `cargo fmt --all -- --check` |
| Desktop UI | Typecheck, Vitest, and a production UI build |
| Clippy and tests | `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`, on Linux, macOS and Windows |
| Licenses and advisories | `cargo deny check` (allowed licenses, RustSec advisories, sources) |
| Minimum supported Rust version | Builds with the MSRV from `Cargo.toml` |
| Dependency review | New dependencies with known vulnerabilities or disallowed licenses |
| Changelog reminder | Flags a PR that changes code without touching `CHANGELOG.md` |

Also: CodeQL (`codeql.yml`), a daily advisory audit (`audit.yml`), OpenSSF Scorecard (`scorecard.yml`), and the nightly builds (`nightly.yml`, see [releasing](releasing.md)).

## When a test fails

Re-run only the failing test while you fix it (`cargo test -p crate name`, or `pnpm --filter @setu/desktop test -- secrets`), then the whole suite once before pushing.
