# Security Policy

Setu stores server credentials and opens connections to production systems, so we take security reports seriously.

## Reporting a vulnerability

**Please do not report security issues in public issues, discussions or pull requests.**

Report privately through GitHub: go to the repository's **Security** tab and choose **Report a vulnerability** (GitHub private vulnerability reporting).

Please include:

- The affected version or commit, and your operating system.
- Steps to reproduce, or a proof of concept.
- The impact you believe it has.

## What to expect

- Acknowledgement within 3 working days.
- An initial assessment within 10 working days.
- Coordinated disclosure: we aim to release a fix and publish an advisory within **90 days** of the report, and will credit you unless you prefer otherwise.

## Supported versions

Setu is pre-alpha. Until 1.0, only the latest commit on the default branch and the latest nightly build receive security fixes.

## Scope

In scope: the Setu desktop app, CLI and Rust crates in this repository, including credential storage, host-key verification, SSH/SFTP handling and the update mechanism.

Out of scope: vulnerabilities in third-party servers you connect to, and issues that require an already compromised local machine.

## Security design principles

- Strict SSH host-key verification by default, with a clear warning when a key changes; no plaintext fallbacks.
- Secrets are encrypted at rest (Argon2id key derivation, AEAD cipher) and are never written to logs, crash reports or telemetry.
- Telemetry is off unless you opt in, and never records commands, hostnames or output.
- Dependencies are pinned and checked with `cargo-deny` and `pnpm audit` in CI and every day; GitHub Actions are pinned to commit SHAs.
- Data from servers and imported files is untrusted: file names, host names and snippets are checked before they reach the file system, `known_hosts` or a shell.

See [docs/security/threat-model.md](docs/security/threat-model.md) for what is protected and the known gaps, and [docs/security/phase1-review.md](docs/security/phase1-review.md) for the latest review.

## Verifying a download

Every build comes with `SHA256SUMS`, a GitHub build-provenance attestation (`gh attestation verify <file> --repo SanowerTamjit/setu`) and an SPDX software bill of materials. See [docs/developer/releasing.md](docs/developer/releasing.md).
