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

Before opening a pull request, run what CI runs:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Pull requests

- Keep PRs small and focused: one change per PR. Large changes start with an issue or an RFC (see below).
- Add tests for new behaviour. Bug fixes should come with a test that fails without the fix.
- Update `CHANGELOG.md` under **Unreleased** for user-visible changes.
- Fill in the PR template. CI must pass on macOS, Linux and Windows.
- Maintainers aim for a first response within 48 hours.

### Security-sensitive code

Changes to `crates/setu-ssh`, `crates/setu-vault`, credential handling, host-key verification or cryptography need a review from a security code owner (see `.github/CODEOWNERS`). Never log secrets, passwords, keys or terminal output.

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
