# Setu user guide

Setu is a free, open-source terminal, SSH client and ops workbench for macOS, Linux and Windows. It works on first launch, keeps everything on your computer in plain files, and reads the SSH setup you already have.

> Setu is **pre-alpha**: features and screens still change. This guide describes the current nightly build.

## Start here

- **[Getting started](getting-started.md)**: install, first launch, connect to a server and copy a file in a few minutes.

## Using Setu

| Page | What it covers |
|---|---|
| [Terminal tabs](terminal.md) | Tabs, split panes, the command palette, find in scrollback, tabs that come back after a restart, hidden secrets, shell profiles, tabs in their own window |
| [Connecting with SSH](ssh.md) | The connect dialog, ways to sign in, host-key trust, jump hosts |
| [Saved hosts](hosts.md) | The hosts view, folders and tags, importing from `~/.ssh/config`, PuTTY, MobaXterm, CSV and Ansible, the jump-host map |
| [Tunnels](tunnels.md) | Local, remote and SOCKS port forwarding over a saved host, with reconnect after a drop |
| [Server management](server-management.md) | A server's systemd services: status, logs, and start, stop, restart, reload, enable and disable, always showing the command first |
| [Monitoring](monitoring.md) | Live CPU, memory, network and disk use for Linux servers, with alerts; nothing installed on the server |
| [Network tools](network-tools.md) | Port check, DNS lookup, ping and Wake-on-LAN, from your computer or through a saved host |
| [Files (SFTP)](files.md) | Upload and download, the transfer queue (pause, resume, retry), editing remote files in your editor, permissions |
| [The vault](vault.md) | Saving passwords and passphrases, auto-lock, unlocking with the OS keychain |
| [SSH keys and known hosts](ssh-keys.md) | Making keys, ssh-agent, YubiKey and FIDO2 keys, managing `known_hosts` |
| [Appearance](appearance.md) | Seven themes, the Classic and Studio layouts, fonts and rendering, Powerlevel10k and other prompts, colour schemes |
| [Snippets and packs](packs.md) | Your role, ready-made commands, installing a pack |
| [Workflows and the task board](workflows.md) | Run commands on many servers with approvals and failure branches; a To do / Doing / Done board |
| [Work memory](work-memory.md) | Opt-in: Setu notices repeated work and suggests workflows and snippets |
| [The command line](cli.md) | `setu doctor`, `setu hosts`, `setu hosts import`, `setu paths` |

## Reference

- [Keyboard shortcuts](keyboard-shortcuts.md)
- [Settings and your files](settings.md): every Settings page, feature switches, where files are kept, portable mode
- [Troubleshooting](troubleshooting.md)
- [What's new](../../CHANGELOG.md)

## Your security

Setu holds your server passwords and keys, so it is careful with them: host keys are checked strictly, secrets are encrypted in the vault and never logged, imported files never run commands, and nothing is sent anywhere. The [threat model](../security/threat-model.md) explains what is protected and how.

Want to build Setu or change it? See the [developer guide](../developer/README.md).
