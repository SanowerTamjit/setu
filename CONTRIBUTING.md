# Contributing to Setu

Thanks for your interest in Setu! This guide gets you from clone to a reviewed pull request.

## Ways to help

- Try a build and report bugs, especially OS-specific terminal or SSH issues.
- Pick an issue labelled `good first issue` or `help wanted`. Comment on it first so nobody duplicates work.
- Improve docs and translations.
- Propose features as issues. Features tagged for a later phase (see [ROADMAP.md](ROADMAP.md)) are welcome as ideas but will wait for that phase.

## Development setup

You need:

- Rust, installed via [rustup](https://rustup.rs). The toolchain is pinned in `rust-toolchain.toml` and installs automatically.
- Node.js 22 and pnpm (for the desktop app). On Linux, also `libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev`.
- Docker (for SSH/SFTP integration tests, once they land).

```sh
git clone https://github.com/SanowerTamjit/setu
cd setu
pnpm install
pnpm build:ui          # the desktop crate embeds the built UI
cargo test --workspace
pnpm desktop           # run the app with hot reload
```

Before opening a pull request, run what CI runs. With [just](https://just.systems) installed:

```sh
just install-hooks   # once: checks secrets, formatting and UI tests before each commit
just check           # format, UI typecheck and tests, clippy, Rust tests, cargo deny
```

Without `just`, the same commands are listed in the `justfile`:

```sh
cargo fmt --all -- --check
pnpm --filter @setu/desktop typecheck && pnpm --filter @setu/desktop test
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

UI logic has unit tests in `apps/desktop/test/` (Vitest); add one when you change it.

## Pull requests

- Keep PRs small and focused: one change per PR. Large changes start with an issue or an RFC (see below).
- Add tests for new behaviour. Bug fixes should come with a test that fails without the fix.
- Update `CHANGELOG.md` under **Unreleased** for user-visible changes.
- Fill in the PR template. CI must pass on macOS, Linux and Windows.
- Maintainers aim for a first response within 48 hours.

### Feature flags

New features that are not tested everywhere yet, or that change behaviour people rely on, ship behind a flag. See [docs/feature-flags.md](docs/feature-flags.md).

### Using AI assistants

You are welcome to use them. [CLAUDE.md](CLAUDE.md) (read by Claude Code, and by other agents through [AGENTS.md](AGENTS.md)) has the project's rules and security checklist, and [docs/development-workflow.md](docs/development-workflow.md) describes how the project itself is built with them. You are responsible for every line you submit: read it, test it, and add a `Co-Authored-By:` trailer naming the assistant.

### Security-sensitive code

Changes to `crates/setu-ssh`, `crates/setu-vault`, credential handling, host-key verification or cryptography need a review from a security code owner (see `.github/CODEOWNERS`). Never log secrets, passwords, keys or terminal output. The threat model is in [docs/security/threat-model.md](docs/security/threat-model.md).

## Sign your commits (DCO)

Every commit must carry a `Signed-off-by` line certifying the [Developer Certificate of Origin](https://developercertificate.org/):

```sh
git commit -s -m "pty: detect WSL distros on Windows"
```

## Commit messages

Use a short prefix with the area, an imperative summary, and a body explaining why:

```
ssh: reject unknown host keys by default

Strict checking is the safe default; users accept a new key explicitly.
```

## Architecture decisions and RFCs

- Significant technical decisions are recorded as ADRs in [`docs/adr/`](docs/adr/). Copy the template and open a PR.
- Larger features or anything that changes a public interface go through a short RFC: open an issue titled `RFC: ...` describing the problem, the proposal and the alternatives, and allow at least a week for discussion.

## Code of Conduct

By participating you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
