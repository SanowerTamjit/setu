# Roles and packs

## Your role

On first launch Setu asks what kind of work you do. **System administrator** turns on the built-in system administrator pack. DevOps, security operations, network and IT operations packs follow in later releases. Skip the question if you like; Settings → Packs → **Change role** asks again.

## Snippets

Snippets from your packs are in the command palette under `Snippet:`. Picking one types the command into the focused terminal without running it, so you can read it and replace placeholders such as `SERVICE` or `HOST` before pressing Enter.

The system administrator pack has 25: disk and inode usage, largest folders and files, memory, the busiest processes, uptime and OS version, failed services, service status, restart and logs, errors since boot, kernel messages, sign-in attempts and who is signed in, listening ports, connections, DNS, firewall, time sync, certificate expiry, pending updates (apt and dnf) and whether a reboot is needed.

## Installing a pack from a file

Settings → Packs → **Install from file** reads one `.json` file. Setu shows the pack's name, version, the permissions it asks for and the snippets it adds before anything is installed. Installed packs are kept in `packs/` in Setu's config folder (`setu paths` shows where); turn any pack off with its tick box, or remove it. A pack with the same id replaces the installed version.

## Pack format (version 1)

```json
{
  "format": 1,
  "id": "acme.docker-basics",
  "name": "Docker basics",
  "version": "0.3.0",
  "description": "Our team's everyday Docker commands.",
  "kind": "snippets",
  "roles": [],
  "os": ["linux", "macos"],
  "permissions": [],
  "contributes": {
    "snippets": [
      { "id": "ps", "title": "Running containers", "command": "docker ps", "tags": ["docker"] },
      { "id": "logs", "title": "Follow a container's logs", "command": "docker logs -f --tail 100 CONTAINER",
        "description": "Replace CONTAINER before pressing Enter." }
    ]
  }
}
```

| Field | Meaning |
|-------|---------|
| `format` | Always `1` for now |
| `id` | Dotted lowercase name, such as `acme.docker-basics`; unique |
| `version` | `major.minor.patch`, optionally with `-pre` |
| `kind` | `role-pack` or `snippets` |
| `roles` | For a role pack, the roles it serves (`sysadmin`, `devops`, `secops`, `netops`, `itops`) |
| `os` | `linux`, `macos`, `windows`; empty means all |
| `permissions` | What the pack asks to be allowed to do, shown before install. Packs in format 1 only hold data, so this is empty |
| `dependencies` | Reserved; a pack with dependencies is refused for now |
| `contributes.snippets` | Up to 1000 snippets, each with a unique `id`, a `title` and a `command` (no control characters) |

Unknown fields are refused, so a typo is caught instead of ignored. Later releases add layouts, runbooks and checks to packs, signed packages and a marketplace.
