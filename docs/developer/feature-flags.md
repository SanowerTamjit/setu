# Feature flags

Setu ships newer features behind flags, so a feature that misbehaves on someone's system can be turned off in **Settings → Features** without waiting for a release, and features that are not ready can ship turned off.

## Stages

| Stage | Default | Meaning |
|---|---|---|
| `experimental` | off | Being built. May change or be removed. Turn it on to try it. |
| `beta` | on | Done, but tested on fewer systems. Tell us if it breaks. |
| `stable` | on | The flag stays for one release so people can still turn it off, then it is removed and the feature is always on. |

## Current flags

| Flag | Stage | What it controls |
|---|---|---|
| `securityKeys` | beta | Making keys on a YubiKey or other FIDO2 key, and loading keys stored on one, in the Keys view. |
| `remoteEdit` | beta | The Files tab's **Edit** button (F4): edit a remote file in a local editor. |
| `pasteGuard` | beta | Ask before pasting several lines into a shell without bracketed paste, where each line would run at once. |
| `workflows` | beta | The Workflows view: steps that run commands on saved hosts, with approvals, failure branches and run history, plus the task board. See `docs/user/workflows.md`. |
| `tunnels` | beta | The Tunnels view (rail, Studio title bar, `Go to tunnels`). Tunnels marked *start when Setu opens* still start, since only people who saw the view can have made them. See `docs/user/tunnels.md`. |
| `networkTools` | beta | The Network tools view (rail, Studio title bar, `Go to network tools`): port check, DNS lookup, ping and Wake-on-LAN. See `docs/user/network-tools.md`. |
| `promptThemes` | beta | The built-in Nerd Font symbols, Unicode 11 character widths (new tabs) and imported terminal colour schemes in Settings → Prompts. Off gives terminals your font as it is and the app theme's colours. See `docs/user/appearance.md`. |
| `workMemory` | beta | The Memory tab in Workflows and the terminal's command capture. Remembering itself is off until you turn it on there; turning the flag off hides the tab and stops all remembering. See `docs/user/work-memory.md`. |

Your choices are saved in `settings.json` under `"features"`; a flag you never changed follows its default, so a new release can change the default for you. **Use the defaults** in Settings → Features clears your choices.

## Adding a flag (for contributors)

1. Add an entry to `FLAGS` in `apps/desktop/src/features.ts` with an `id` (camelCase), a title and description people understand, a `stage`, a `default` and the release it arrived in (`since`).
2. Check it where the feature starts: `if (enabled("myFeature")) …`. Hide the entry point (button, menu item, shortcut, palette command) rather than half the feature.
3. Add it to the table above and mention it in `CHANGELOG.md`.
4. Move it to `stable` once it has been on by default for a release without problems, and delete it (and every `enabled("…")` check) one release later.

Flags are for features, not for security fixes: a fix is never behind a flag that could turn it off.
