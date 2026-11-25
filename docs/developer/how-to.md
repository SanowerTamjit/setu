# How-to recipes

Short checklists for the changes people make most often. Each one ends with the same three things: tests, `CHANGELOG.md` under **Unreleased** if users will notice, and the matching page in `docs/` (see [Keeping the docs up to date](docs-guide.md)).

## Add a command (palette entry and shortcut)

Everything the person can do is a command, so the palette, keymaps and buttons stay in step.

1. Register it where the feature starts:
   ```ts
   registerCommand({
     id: "hosts.refresh",            // stable: keymaps refer to it
     title: "Reload saved hosts",
     category: "Hosts",              // shown before the title in the palette
     keywords: "refresh reread",     // extra words to match
     when: () => enabled("myFlag"),  // optional: hidden when false
     run: () => reloadHosts(),
   });
   ```
   Commands that change over time (one per saved host, snippet, …) come from `registerProvider`.
2. For a default shortcut, add the id to `DEFAULT` in `apps/desktop/src/keymap.ts` (and to `VIM` / `EMACS` if it deserves one). Shortcuts use Ctrl+Shift so plain Ctrl keys still reach the shell.
3. Buttons call `runCommand(id)` rather than the function, so the action has one path.
4. List new shortcuts in [`docs/user/keyboard-shortcuts.md`](../user/keyboard-shortcuts.md).

## Add a setting

1. Add the field to `Settings` and `DEFAULTS` in `apps/desktop/src/settings.ts`.
2. Check it in `sanitize()`: anything from `settings.json` is untrusted (wrong type, huge string, unknown value) and falls back to the default.
3. Add a test in `apps/desktop/test/settings.test.ts` for a valid value and for garbage.
4. Show it on a Settings page (`settings-dialog.ts`, or a feature's own `settings-*.ts` registered with `registerSettingsSection`). Save with `updateSettings({ … })`.
5. Document it in [`docs/user/settings.md`](../user/settings.md) and the feature's page.

## Add a feature flag

New or risky features ship behind a flag; security fixes never do. Follow [Feature flags](feature-flags.md#adding-a-flag-for-contributors): an entry in `FLAGS` in `features.ts`, `enabled("id")` where the feature starts, and a row in the flags table and in the user [Settings](../user/settings.md#features) page.

## Add an IPC command

1. Write the function in the matching module in `apps/desktop/src-tauri/src/` with `#[tauri::command]`. Keep the logic in a crate where it can be tested without Tauri; the command only checks inputs and calls it.
2. Check every argument as if an attacker sent it: paths stay inside their folder, sizes and counts are capped, ids match a strict pattern. Return errors as plain sentences with no secret in them.
3. Long work goes off the main thread (`async`, or `spawn_blocking` for blocking I/O). Streams use a `tauri::ipc::Channel` argument.
4. Register it in `generate_handler!` in `lib.rs`.
5. Add a typed wrapper in `apps/desktop/src/ipc.ts`.
6. Tests for the checks (a path with `..`, an oversized input, a bad id).
7. Add it to [IPC commands](ipc-commands.md) and, if it reads a new file, makes a network call or touches a secret, to the [threat model](../security/threat-model.md).

## Add a theme

A theme covers the app and the terminal, in both layouts.

1. `apps/desktop/src/settings.ts`: add the id to `ThemeName` and `THEME_NAMES`.
2. `apps/desktop/src/themes.ts`: an entry in `THEMES` (name and one-line description), the terminal palette in `PALETTES`, the Studio terminal background in `STUDIO_BACKGROUND`, and the find-match colours in `FIND_COLORS`.
3. `apps/desktop/src/styles.css`: the design tokens under `:root[data-theme="id"]`, the Settings preview card under `[data-preview-theme="id"]`, and the Studio skin under `:root[data-layout="studio"][data-theme="id"]`. Copy an existing theme's blocks (Ember for dark, Sandstone for light) and change the colours; check text contrast (WCAG AA, 4.5:1).
4. Screenshots in Classic and Studio for the pull request.
5. Add a row to the table in [`docs/user/appearance.md`](../user/appearance.md#themes).

## Add an importer (another tool's host list)

1. Add a `Format` in `crates/setu-hosts/src/sources.rs`, recognise it in `detect()`, and parse it in `parse()` into the same `Parsed` result the others produce. Never read passwords, never run a command (`ProxyCommand` and similar are noted, not run), and refuse host names that could be patterns.
2. Unit tests with a small sample file, including a malicious one.
3. Offer it in `apps/desktop/src/import-dialog.ts`.
4. Document the format in [`docs/user/hosts.md`](../user/hosts.md#importing-from-other-tools) and the threat model (a new file read).

## Add a snippet to the built-in pack

Snippets are one line, typed into the terminal and never run. Add it to the system administrator pack, `apps/desktop/src-tauri/packs/sysadmin.json` (built in with `include_str!` in `packs.rs`; the `built_in_packs_are_valid` test checks it), keep placeholders in capitals (`SERVICE`, `HOST`), and update the count in [`docs/user/packs.md`](../user/packs.md).

## Add an extension point

Extension points live in `crates/setu-core/src/extend.rs`, always with a no-op default so the open-source app behaves as before. Add the trait, a `with_…` builder on `Extensions`, call it where the work happens, test it with a fake, and document it in [Extending Setu](extending.md) and the threat model.

## Before opening a pull request

- `just check` passes.
- One topic per pull request, based on `master`, commits signed off (`git commit -s`) with an `area: summary` subject.
- `CHANGELOG.md`, docs and screenshots are in.
- Checked against the security checklist in [`CLAUDE.md`](../../CLAUDE.md#security-checklist-check-every-change-against-it).
