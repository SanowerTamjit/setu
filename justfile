# Setu's everyday commands. Install `just` (https://just.systems), then run
# `just` to list them. Each one is a plain command you can also run by hand.

set windows-shell := ["pwsh", "-NoLogo", "-Command"]

# List the commands
default:
    @just --list

# Install the JavaScript dependencies and build the UI (needed before any cargo build)
setup:
    pnpm install --frozen-lockfile
    pnpm build:ui

# Everything CI checks, in the same order: format, lint, tests, licenses and advisories
check: fmt-check ui-check clippy test deny

# Quick checks for the pre-commit hook: formatting and the UI (seconds, not minutes)
quick: fmt-check ui-check

# Format Rust code
fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

# Typecheck and unit-test the TypeScript UI
ui-check:
    pnpm --filter @setu/desktop typecheck
    pnpm --filter @setu/desktop test

clippy: build-ui
    cargo clippy --workspace --all-targets --locked -- -D warnings

# Rust and UI tests
test: build-ui
    cargo test --workspace --locked
    pnpm --filter @setu/desktop test

# Licenses, advisories (CVEs) and sources of every dependency
deny:
    cargo deny check
    pnpm audit --prod

build-ui:
    pnpm build:ui

# Run the desktop app with hot reload
app:
    pnpm desktop

# Run the CLI, e.g. `just cli doctor`
cli *args:
    cargo run -p setu-cli -- {{args}}

# Use the repository's git hooks (format and UI checks before each commit, sign-off check)
install-hooks:
    git config core.hooksPath scripts/hooks
    @echo "Hooks installed. Skip once with: git commit --no-verify"
