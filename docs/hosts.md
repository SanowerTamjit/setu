# Saved hosts

Setu keeps your saved hosts in one plain TOML file, so you can back it up, diff it, or keep it in Git.

| OS      | File                                              |
|---------|---------------------------------------------------|
| Linux   | `~/.config/setu/hosts.toml` (or `$XDG_CONFIG_HOME/setu`) |
| macOS   | `~/Library/Application Support/setu/hosts.toml`  |
| Windows | `%APPDATA%\setu\hosts.toml`                       |

With `SETU_HOME` set (portable mode), the file is `$SETU_HOME/config/hosts.toml`.

**No secrets are stored here.** Passwords are asked for when you connect, and keys stay in `~/.ssh`. Passwords and passphrases will be storable in the encrypted vault (`setu-vault`), never in this file.

## Format

```toml
version = 1

[[host]]
name = "web-01"                       # required, unique (case-insensitive)
host = "10.0.4.21"                    # required: address or DNS name
port = 2222                           # default 22
user = "deploy"                       # default: your local user name
folder = "Production/Web"             # slash-separated, optional
tags = ["nginx", "eu-west"]
auth = "key"                          # auto (default) | agent | key | password
key_file = "~/.ssh/id_ed25519"        # required when auth = "key"
jump = "ops@bastion.example.com:22"   # ProxyJump; several hops separated by commas
notes = "Behind the EU load balancer"
```

| `auth`     | What Setu does                                           |
|------------|----------------------------------------------------------|
| `auto`     | ssh-agent, then the default keys in `~/.ssh`, like OpenSSH |
| `agent`    | Only keys in ssh-agent (or Pageant on Windows)           |
| `key`      | The key in `key_file`; asks for its passphrase if needed |
| `password` | Asks for the password at every connection                |

Setu rewrites the whole file when you change hosts in the app, so comments you add by hand are not kept. Saves are atomic: the file is written to `hosts.toml.tmp` and then renamed.

A file with a higher `version` than Setu understands is never overwritten.

## Importing from `~/.ssh/config`

**Import** in the hosts sidebar reads your OpenSSH config and lists every concrete host it finds. Your SSH config is never changed.

- Each name on a `Host` line becomes a host. Patterns with `*`, `?` or `!` are not hosts, but their settings still apply, as in OpenSSH (for example `Host *` with `User`).
- The first value found for a keyword wins, as in OpenSSH.
- `Include` is followed, including globs in the file name such as `Include config.d/*`.
- `HostName` (with `%h`), `User`, `Port`, `IdentityFile` and `ProxyJump` are imported. A `ProxyJump` that names another config host is written out as `user@address:port`.
- `ProxyCommand` is not run; such hosts are imported without it and marked in the list.
- `Match` blocks are skipped with a warning.

Hosts that are already saved show as **Saved** (same settings) or **Update** (the config has different settings). Importing an update changes only the connection settings and keeps your folder, tags and notes.

## Importing from other tools

**Import** in the hosts sidebar (or `Import hosts` in the palette) asks where your hosts are now:

| Source | File | What comes across |
|--------|------|-------------------|
| PuTTY | A `.reg` export of its saved sessions. PuTTY keeps them in the Windows registry; export them with `reg export HKCU\Software\SimonTatham\PuTTY\Sessions putty.reg` | Host, port, user (also from `user@host`), key file. Telnet, serial and raw sessions are listed as left out |
| MobaXterm | A `.mxtsessions` export | Host, port, user, key file, SSH gateway as the jump host, and the bookmark folder |
| Termius or any CSV | A CSV whose first row names the columns: `name`/`label`, `host`/`address`/`ip`, `port`, `user`/`username`, `group`/`folder`, `tags`, `key`, `jump` (any order, commas or semicolons) | Those fields |
| Ansible | An inventory in INI or YAML | `ansible_host`, `ansible_port`, `ansible_user`, `ansible_ssh_private_key_file`, and a jump host from `-J` or `ProxyJump` in `ansible_ssh_common_args`; group and parent-group variables apply; ranges such as `web[01:03]` expand. The first group becomes the folder, other groups become tags. Hosts with a non-SSH `ansible_connection` are left out |

Saved passwords are never read, even where the other tool stores them. As with `~/.ssh/config`, you choose which hosts to import, and hosts keep the folders they had in the other tool (inside the folder you name, if you name one). Hosts whose names are already saved only update their connection settings.

## Jump-host map

`Jump-host map` in the palette draws how your saved hosts are reached: this computer on the left, then each bastion, then the hosts behind it. Jump hosts that are not saved hosts are drawn dashed. Click a host to connect.
