# Nightly builds and signing

Every night at 02:17 UTC, the [Nightly workflow](../.github/workflows/nightly.yml) builds `master` for macOS, Linux and Windows. It replaces the **`nightly`** pre-release on the [Releases page](https://github.com/SanowerTamjit/setu/releases/tag/nightly). If nothing changed since the last nightly, it skips the build. A maintainer can also run it by hand: **Actions → Nightly → Run workflow**.

Nightlies are untested snapshots for trying new work. They are not for production machines.

## What is published

| Platform | Desktop app | CLI |
|----------|-------------|-----|
| Linux x86_64 (glibc 2.35+, e.g. Ubuntu 22.04, Debian 12, Fedora 36) | `.AppImage`, `.deb`, `.rpm` | `setu-cli-…-linux-x86_64.tar.gz` |
| macOS 10.13+ (Apple silicon and Intel, one universal build) | `.dmg` | `setu-cli-…-macos-universal.tar.gz` |
| Windows 10/11 x64 | `.msi`, `-setup.exe` | `setu-cli-…-windows-x86_64.zip` |

There is also `SHA256SUMS`, a checksum for every file. The release notes say how each platform was signed.

Names look like `Setu-nightly-20261120-ab12cd3-linux-x86_64.deb`, where the date and short commit say exactly what you have.

## Checking a download

```sh
sha256sum -c SHA256SUMS --ignore-missing                       # the file is intact
gh attestation verify Setu-nightly-…-linux-x86_64.deb --repo SanowerTamjit/setu   # it was built by this repo's workflow
```

Every file has a [build provenance attestation](https://docs.github.com/actions/security-for-github-actions/using-artifact-attestations). It is signed with GitHub's keyless Sigstore signing, so it works today without any certificate. It proves the file came from this repository's Nightly workflow at that commit. It does not replace OS code signing, which is what makes macOS and Windows open the app without warnings.

## Installing an unsigned nightly

Until the certificates below are added:

- **macOS:** the app is ad-hoc signed, not notarized. After dragging Setu to Applications, right-click it and choose **Open**, then **Open** again (only the first time). On macOS 15 and later, use **System Settings → Privacy & Security → Open Anyway**. For the CLI, run `xattr -d com.apple.quarantine setu` after unpacking.
- **Windows:** SmartScreen shows "Windows protected your PC". Choose **More info → Run anyway**.
- **Linux:** nothing extra. For the AppImage, run `chmod +x` on the file and start it.

## Turning on signing (for the maintainer)

The workflow checks for these repository secrets (**Settings → Secrets and variables → Actions → New repository secret**). Each platform switches to real signing as soon as its secrets exist. Nothing else needs to change, and missing secrets never fail the build.

### macOS: Developer ID signing and notarization

Needs a paid [Apple Developer Program](https://developer.apple.com/programs/) membership.

| Secret | What to put in it |
|--------|-------------------|
| `APPLE_CERTIFICATE` | Your **Developer ID Application** certificate and its private key, exported from Keychain Access as `.p12`, then base64-encoded: `base64 -i cert.p12 \| pbcopy` |
| `APPLE_CERTIFICATE_PASSWORD` | The password you set when exporting the `.p12` |
| `APPLE_SIGNING_IDENTITY` | The certificate's name, e.g. `Developer ID Application: Your Name (TEAMID1234)` (from `security find-identity -v -p codesigning`) |
| `APPLE_ID` | The Apple ID email of the developer account (for notarization) |
| `APPLE_PASSWORD` | An [app-specific password](https://support.apple.com/102654) for that Apple ID, not the account password |
| `APPLE_TEAM_ID` | The 10-character team ID from the Apple Developer membership page |

With the first three, builds are signed. With all six, they are also notarized, so they open without any warning. Notarization adds a few minutes per build.

### Windows: Authenticode

| Secret | What to put in it |
|--------|-------------------|
| `WINDOWS_CERTIFICATE` | A code-signing certificate with its private key as `.pfx`, base64-encoded: `[Convert]::ToBase64String([IO.File]::ReadAllBytes("cert.pfx"))` |
| `WINDOWS_CERTIFICATE_PASSWORD` | The `.pfx` password |

The app, both installers and `setu.exe` are signed with SHA-256 and a DigiCert timestamp.

Since June 2023, certificate authorities issue new OV and EV code-signing keys only on hardware tokens or cloud HSMs, so most new certificates **cannot be exported as `.pfx`**. If that is your case, two options fit an open-source project better:

- [SignPath Foundation](https://signpath.org/): free code signing for open-source projects, through a GitHub Actions integration.
- [Azure Trusted Signing](https://learn.microsoft.com/azure/trusted-signing/): a low-cost monthly service, also driven from Actions.

Either one replaces the "Import the Windows code-signing certificate" step with that service's action. Ask for that change once you have picked one.

### Linux

Nothing to add. Linux packages are verified with `SHA256SUMS` and the provenance attestation. Signing the `.deb`/`.rpm` repositories with GPG comes with real package repositories, after Phase 1.

## Testing changes to the pipeline

A pull request that changes `.github/workflows/nightly.yml`, `scripts/release/` or `tauri.conf.json` runs the three builds and uploads their files as workflow artifacts (kept 7 days). It never publishes. Only scheduled or manual runs on `master` publish the release.

## Versioned releases and release notes

Nightlies are snapshots; a versioned release (`v0.1.0`, …) is a promise. Before tagging one:

1. Move everything under **Unreleased** in `CHANGELOG.md` to a new `## [0.1.0] - YYYY-MM-DD` heading. Sections, in this order: **Security**, **Added**, **Changed**, **Fixed**, **Removed**. Each line says what changed for users, in plain words.
2. Bump `version` in `Cargo.toml` (`[workspace.package]`), `apps/desktop/package.json` and `apps/desktop/src-tauri/tauri.conf.json` together.
3. Check that `audit.yml` (advisories) is green on the release commit, and that flags due to retire are removed (`docs/feature-flags.md`).
4. Tag the commit `vX.Y.Z`. The release notes are the version's `CHANGELOG.md` section, followed by GitHub's generated list of pull requests, grouped by label (`.github/release.yml`: `security`, `feature`, `bug`, `documentation`, `dependencies`; `no-changelog` hides a PR).
5. Security fixes get a GitHub security advisory (Security → Advisories) published with the release, crediting the reporter.

Every release ships the same proofs as nightlies: `SHA256SUMS`, a Sigstore provenance attestation for each file, and an SPDX SBOM (`*.spdx.json`) listing every dependency.

## Keeping signing secrets safe

Signing secrets are only given to builds of `master` and manual runs, never to pull-request builds. For one more layer, create a `release` environment (**Settings → Environments → New environment**) that only `master` may use, move the `APPLE_*` and `WINDOWS_*` secrets into it, and add `environment: release` to the nightly `build` job.
