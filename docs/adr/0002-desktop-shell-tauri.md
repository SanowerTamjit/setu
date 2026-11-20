# 0002. Desktop shell: Tauri 2, confirmed by a spike

- Status: Proposed
- Date: 2026-11-20

## Context

Setu needs one UI that looks and behaves the same on macOS, Linux and Windows, starts in about two seconds, and adds no input lag over a native terminal. The candidates are Tauri 2 (Rust plus the system web view) and Electron (bundled Chromium).

## Decision

Build the desktop app on **Tauri 2** with a TypeScript UI and xterm.js (WebGL renderer). All protocol, PTY and crypto work stays in the Rust core and is exposed to the UI through Tauri commands and events; the web view never holds private keys or vault secrets.

This ADR moves to **Accepted** once the spike passes on all three OSes:

1. A local shell and an SSH session render in xterm.js with no visible lag at 120 columns of fast output (`yes`, `cat` of a large file).
2. Copy/paste, IME input and resize work on each OS.
3. WebGL rendering works on WebKitGTK (Linux) and WebView2 (Windows), with a canvas fallback.
4. Installer size and idle memory are recorded for comparison.

## Consequences

- Small installers and low memory use; the core is shared with the CLI.
- Web view differences (WebKitGTK, WKWebView, WebView2) are a risk, mitigated by the spike and CI on all three OSes.

## Alternatives considered

- **Electron**: identical rendering everywhere and a mature ecosystem, but much larger installers and memory use, and a second runtime (Node) next to the Rust core. Kept as the fallback if the spike fails.
- **Native toolkits per OS**: best fidelity, but three UIs is not sustainable for a small team.
