# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Desktop app (Tauri 2 + xterm.js): opens your default shell on first launch, terminal tabs for any detected shell (shell picker), close/switch tabs with Ctrl+Shift+T, Ctrl+Shift+W and Ctrl+Tab, resizing that follows the window, and a status bar with shell and size.
- `setu-pty`: detects local shells (bash, zsh, fish, sh, PowerShell, Windows PowerShell, cmd and WSL distros) and runs them in a pseudo-terminal with input, output streaming, resize and exit status.
- Project foundation: community files, Cargo workspace with `setu-core`, CI on macOS, Linux and Windows, and the first architecture decision records.
