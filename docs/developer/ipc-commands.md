# IPC commands

The web view talks to Rust only through Tauri commands, registered in `generate_handler!` in `apps/desktop/src-tauri/src/lib.rs`, with typed wrappers in `apps/desktop/src/ipc.ts`. This is the attack surface between the UI and your computer, so every command is listed here.

**Rule:** treat every argument as coming from an attacker who controls the web view. A command checks its own inputs (paths stay in their folder, sizes are capped, ids are validated, names are single path parts) and never returns a stored secret. Adding or changing a command also updates this page and the [threat model](../security/threat-model.md).

## App and windows

| Command | What it does |
|---|---|
| `app_info` | Version, OS, local user name, and the edition of a build with extensions |
| `window_detach` | Moves a tab into a new window |
| `window_take` | The payload for a new window; each token works once |
| `extension_panel` | The settings page a build with extensions adds; `None` in the open-source build |
| `extension_action` | Presses a button (by id, at most 64 characters) on that page |

## Terminals (`terminals.rs`)

| Command | What it does |
|---|---|
| `list_shells` | The local shells found on this computer |
| `open_terminal` | Starts a local shell; output and exit code stream over `Channel`s |
| `write_terminal`, `resize_terminal`, `close_terminal` | Input, size and close for a terminal session (local or SSH) |
| `terminal_hold` | Holds a session's output while its tab moves to another window |
| `terminal_attach` | Shows a session in the calling window, with what it printed meanwhile |

## SSH and keys (`ssh.rs`, `keys.rs`)

| Command | What it does |
|---|---|
| `ssh_connect` | Connects (with jump hosts), checks the host key strictly, signs in; asks the access policy first in builds with one |
| `ssh_open_shell` | Opens a shell on a connection (a tab or a split pane) |
| `ssh_trust_host` | Saves a host key the person accepted; only a key that server actually offered can be trusted |
| `ssh_keys_list` | Keys in `~/.ssh`: type, fingerprint, passphrase and agent state, never key material |
| `ssh_key_generate` | Makes a key (0600, never overwriting); security keys through `ssh-keygen` |
| `ssh_key_add_to_agent` | Gives a key to ssh-agent |
| `ssh_key_load_resident` | Copies keys stored on a security key into `~/.ssh` (`ssh-keygen -K`) |
| `known_hosts_list`, `known_hosts_remove` | Lists and forgets `known_hosts` entries; keeps `known_hosts.old` |

## Hosts (`hosts.rs`)

| Command | What it does |
|---|---|
| `hosts_list`, `host_save`, `host_delete` | The saved hosts in `hosts.toml` |
| `ssh_config_scan` | Reads `~/.ssh/config` and compares it with saved hosts |
| `hosts_parse_file` | Reads hosts from a PuTTY, MobaXterm, CSV or Ansible file; saves nothing |
| `ssh_config_import` | Saves the hosts the person picked |

## Files (`files.rs`, `remote_edit.rs`)

| Command | What it does |
|---|---|
| `files_open`, `files_close` | SFTP on the connection of an SSH tab |
| `files_list`, `local_list` | Lists a remote or local folder |
| `files_mkdir`, `files_rename`, `files_remove` | Changes on the server |
| `files_stat`, `files_set_attributes` | Mode, owner and group; optionally recursive |
| `transfer` | Copies a file or folder, with progress on a `Channel`; can resume a `.setu-part` file |
| `transfer_pause`, `transfer_cancel` | Stops a copy, keeping or deleting its partial file |
| `remote_edit_open` | Copies a remote file to a private folder, opens it in the editor, writes saves back |
| `remote_edit_push` | Writes the local copy now, even if the server's copy changed |
| `remote_edit_stop` | Stops watching and deletes the local copy |

## Vault (`vault.rs`)

| Command | What it does |
|---|---|
| `vault_status` | Whether a vault exists, is unlocked, can unlock automatically, and the saved targets (never the secrets) |
| `vault_create`, `vault_unlock`, `vault_lock` | Set up, unlock with the master password, lock |
| `vault_remember` | Turns OS-keychain unlock on or off |
| `vault_remove` | Removes one saved secret |
| `vault_change_password` | Re-encrypts with a new master password and salt |

## Session, settings, packs, appearance

| Command | What it does |
|---|---|
| `session_load`, `session_save`, `session_clear` | `session.json`: tabs to reopen (no secrets) |
| `settings_load`, `settings_save` | `settings.json`, size-limited; the UI's `sanitize` checks it again on load |
| `packs_list`, `pack_inspect`, `pack_install`, `pack_remove` | Packs: data only, checked before install |
| `color_scheme_parse` | Reads colours from another terminal's scheme file (at most 2 MB, 100 schemes) |
| `prompt_detect` | Whether Powerlevel10k, Oh My Zsh, Starship or Oh My Posh are set up; takes no input, returns no file contents |

## Workflows and work memory (`workflows.rs`, `memory.rs`)

| Command | What it does |
|---|---|
| `workflows_list`, `workflow_save`, `workflow_delete` | Workflow files in `workflows/`; ids are checked so paths stay in that folder |
| `workflow_run` | Runs a workflow **as saved on disk** (never one sent by the window); progress on a `Channel` |
| `workflow_answer`, `workflow_cancel` | Approve or reject an approval step; stop a run |
| `workflow_history`, `workflow_history_clear` | `workflow-runs.json` |
| `board_load`, `board_save` | The task board; every card is checked before writing |
| `memory_view`, `memory_settings` | Work memory and its settings |
| `memory_record` | Keeps one command; refuses while remembering is off, masks secrets itself |
| `memory_delete`, `memory_clear`, `memory_dismiss` | Forget a command, everything, or a suggestion |
| `memory_snippet_save`, `memory_snippet_delete` | Snippets made from work memory |
| `memory_workflow_draft` | Rebuilds a suggested workflow in Rust from its id; not saved |
| `memory_summary` | A Markdown summary of routine work, masked commands left out |

## Tunnels (`tunnels.rs`)

| Command | What it does |
|---|---|
| `tunnels_list` | Saved tunnels from `tunnels.toml` with each one's live status (state, message, listening address, connection counts); never a secret |
| `tunnel_save` | Adds or changes a tunnel; the host must be a saved host, an id the window sends must already exist (new ids are made in Rust), and every field is checked (`setu_hosts::tunnels::Tunnel::validate`: ports 1-65535, addresses without spaces or option characters, no two tunnels on one local port) |
| `tunnel_delete` | Stops a tunnel and removes it |
| `tunnel_start` | Starts a tunnel **as saved on disk**, by id; connects like a workflow run (vault or agent only, never a prompt, host key must be trusted) and asks the access policy first |
| `tunnel_stop` | Stops a tunnel and closes its connections |

## Network tools (`nettools.rs`)

Every host name is checked by `setu_net::check_target` first (letters, digits, `.`, `-`, `_`, `:`; no leading `-`; at most 253 characters), so it can be put into a fixed server command without quoting. `through` is a saved host's name; it logs in like a workflow run (vault or agent only, never a prompt, host key must be trusted, access policy asked first) and disconnects when done.

| Command | What it does |
|---|---|
| `net_check_port` | TCP connections to `host:port`, 1-10 attempts (clamped), 5 s each. Here: `tokio` connect. Through a host: a `direct-tcpip` channel opened and closed at once; nothing runs on the server |
| `net_lookup` | Addresses for a name. Here: the system resolver. Through a host: `getent ahosts NAME`, output capped at 16 KiB and only IP addresses kept |
| `net_ping` | Through a host only: `ping -c 4 -W 2 NAME`, output capped at 16 KiB and stripped to plain text (no escape sequences) |
| `net_tls` | The certificate of `host:port`, checked for `name` (or the host). The handshake is made in Rust with rustls (ring) over a TCP connection, or through a host over a `direct-tcpip` channel; trust is checked against the system roots (`rustls-native-certs`), and a failed check is reported, not fatal. The chain (at most 8) is parsed with `x509-cert`; texts are cleaned and capped. Nothing is sent after the handshake |
| `net_wake` | A Wake-on-LAN packet for a checked MAC address, sent to UDP port 9 on 255.255.255.255 or a given IPv4 address that ends in `.255`; nothing else can be a destination |

## Monitoring (`monitor.rs`)

| Command | What it does |
|---|---|
| `monitor_start` | Watches a saved host (looked up by name in Rust): logs in like a tunnel (vault or agent only, never a prompt, host key must be trusted, access policy first), then every `interval` seconds (clamped 2-60) runs the fixed `setu_monitor::COMMAND`, parses it in Rust and sends a `Snapshot` (numbers, plus cleaned mount points and device, interface and process names) on the window's channel. Alert limits are clamped to 1-100 %. `hold` (seconds, clamped to 0-3600) is how long a reading must stay over its limit; each `sample` event also carries `active` (readings over a limit, with seconds so far and whether each is `firing`), from `setu_monitor::alerts::AlertTracker`. With `notify`, Rust shows a desktop notification when an alert starts or clears (`tauri-plugin-notification`, used from Rust only; no capability is granted to the window). At most 24 at once; stops when the window's channel closes, or after 10 failures in a row. Returns an id |
| `monitor_stop` | Stops a monitor by id and disconnects |

## Server management (`server.rs`)

The window never sends command text. Every command is built in `setu_server::services` from a unit name that passed `check_unit` (letters, digits, `: _ . \ -`, one `@` at most, ending in `.service`; no spaces, quotes or shell characters) and is quoted when it holds a `\`. `sudo` is a flag that adds `sudo -n` (never a password). Hosts are looked up by name in Rust and logged in to like a tunnel (vault or agent only, never a prompt, host key must be trusted, access policy first); one connection per host is kept for 5 minutes (at most 8).

| Command | What it does |
|---|---|
| `server_services` | Runs the fixed `setu_server::services::LIST_COMMAND` (`systemctl list-units` and `list-unit-files`, no input) and returns the services: names checked, texts cleaned and cut, at most 4000, 2 MiB of output read |
| `server_service_details` | `systemctl show -p … UNIT`: state, main process, memory, tasks, restarts, unit file |
| `server_service_logs` | `journalctl -u UNIT -n LINES [-p warning\|err] --no-pager -o short-iso`, with `lines` clamped to 10-5000; the text is stripped of escape and control characters and the newest 512 KiB kept. Returns the command shown beside the logs |
| `server_service_command` | Returns the exact command a change would run (`[sudo -n ]systemctl start\|stop\|restart\|reload\|enable\|disable UNIT`) for the window to show. Runs nothing |
| `server_service_action` | Builds that command again from the same action and unit, asks the command policy (`CommandOrigin::Server`), runs it (120 s at most) and returns whether it worked, the exit code, the cleaned output (16 KiB) and a plain hint for common failures. Recorded as `server.change` with the command, never the output |
| `server_disconnect` | Closes the kept connection to a host |
| `server_processes` | Runs the fixed `setu_server::processes::LIST_COMMAND` (`ps -eo pid=,ppid=,user=,pcpu=,pmem=,rss=,etimes=,stat=,comm= --sort=-pcpu`, no input): command names only (never arguments), at most 500, texts cleaned and cut |
| `server_process_command` | Returns the exact command that would signal a process (`[sudo -n ]kill -TERM\|-KILL PID`). The pid must be 2 to 4194304. Runs nothing |
| `server_process_signal` | Builds that command again from the same signal and pid, asks the command policy (`CommandOrigin::Server { tool: "processes" }`), runs it and returns the result; recorded as `server.change` |
| `server_disks` | Runs the fixed `df -P -k -T` and returns real file systems (pseudo and loop file systems left out, at most 64) |
| `server_folder_sizes` | `[sudo -n ]timeout 60 du -x -k -d 1 -- 'PATH' \| sort -rn \| head -n 41`. `PATH` must pass `setu_server::disk::check_path` (absolute, at most 1024 bytes, no `'`, control characters or bidi overrides) and is single-quoted; only lines directly inside `PATH` are kept. Reads only |
| `server_keys` | Reads the login user's `~/.ssh/authorized_keys` over SFTP (home from the server, checked with `check_path`; at most 256 KiB, must be a plain UTF-8 file, never a link) and returns its keys: line, type, `SHA256:` fingerprint, comment, options, base64 key. At most 500 keys; other lines are only counted. A missing file is an empty list with `missing` set |
| `server_key_check` | Checks a pasted public key with `setu_server::keys::check_new_key` (one line, known type, valid base64 whose inside names the same type, no options, no control characters, at most 16 KiB, never a private key) and returns its type, fingerprint, comment and the line that would be added. Local only: nothing is sent |
| `server_key_add` | Checks the key again, reads the file, refuses a key already there, asks the command policy (`CommandOrigin::Server { tool: "keys" }`, with `add to PATH: TYPE FINGERPRINT (COMMENT)` as the command), writes `authorized_keys.setu-new` (mode kept without group/other write, else 0600), reads it back, renames the old file to `.setu-old`, the new one into place and removes the old one (putting it back if the swap fails). Creates `~/.ssh` at 0700 when missing. Recorded as `server.change` with that description |
| `server_key_remove` | Same safe write, removing the line `line` only if it still holds `key` (the base64 blob as listed); every other line is kept byte for byte. Refuses with "authorized_keys changed since it was read" otherwise |

## Problems (`problems.rs`)

| Command | What it does |
|---|---|
| `problems_list` | Joins the last health check of each saved host (kept in memory since Setu started; hosts no longer saved are dropped), the firing alerts of running monitors and the latest failed run of each workflow (`setu_health::sources`), stamps when each was first seen, sorts them and puts snoozed ones aside. Reads nothing from a server |
| `problems_check` | Checks the named saved hosts (every saved host when `names` is empty, at most 500; names are looked up in Rust), six at a time, 30 s each: logs in like monitoring (vault or agent only, never a prompt, host key must be trusted, access policy first), runs the fixed `setu_health::check::COMMAND` (no input; `systemctl list-units --state=failed`, `df -P -k -T`, `df -P -i -T`, `/var/run/reboot-required`, `timedatectl show -p NTPSynchronized`), reads at most 256 KiB, disconnects and parses the output into findings. A host that cannot be checked becomes one finding with the reason. Progress (`started`, `host`, `finished`) comes over the window's channel. One check at a time |
| `problems_snooze` | Hides a finding id (one line, at most 600 characters) for 1 minute to 90 days; saved in `problems.json` (0600, atomic, at most 1000 entries) |
| `problems_unsnooze` | Shows a snoozed finding again |

## Adding a command

See [How-to recipes](how-to.md#add-an-ipc-command).
