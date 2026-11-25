# Keeping the docs up to date

The docs are part of the product. A feature is not done until the page that describes it is right, so docs change in the **same pull request** as the code.

## Where things go

| You changed | Update |
|---|---|
| Something people see or do in the app or CLI | The matching page in [`docs/user/`](../user/README.md), and `CHANGELOG.md` under **Unreleased** |
| A shortcut | [`docs/user/keyboard-shortcuts.md`](../user/keyboard-shortcuts.md) (and the feature's page) |
| A setting, a feature flag, or a file Setu writes | [`docs/user/settings.md`](../user/settings.md) |
| A new feature area | A new page in `docs/user/`, linked from [`docs/user/README.md`](../user/README.md) |
| A crate, module or how the parts fit | [`docs/developer/architecture.md`](architecture.md) and the map in `CLAUDE.md` |
| An IPC command | [`docs/developer/ipc-commands.md`](ipc-commands.md) and the [threat model](../security/threat-model.md) |
| A new file read, network call or secret | The [threat model](../security/threat-model.md) |
| Build, test or release steps | [`setup.md`](setup.md), [`testing.md`](testing.md), [`releasing.md`](releasing.md) |
| A lasting decision (a library, a format, a boundary) | A new ADR in [`docs/adr/`](../adr/) from the template |

A page that is no longer true is worse than no page. When you remove or rename something, search the docs for it: `grep -rn "old name" docs/ README.md`.

## How to write

- **For the reader's task.** User pages say what you can do and how, in the words on the screen (bold for buttons and menu items: **Import**, Settings → **Features**). Developer pages say where the code is and the rules it follows.
- **Plain and short.** Short sentences, everyday words, no marketing. Tables for reference, numbered steps for procedures.
- **Only what exists.** Document what is on `master`. Planned work belongs in [`ROADMAP.md`](../../ROADMAP.md), not in the guide; a short "Not yet" line is fine where people will look for it.
- **Exact.** Shortcuts, file names, defaults and limits match the code. When you quote a limit (50 MB, 100 runs), check it in the source.
- **Secrets.** Examples never contain a real host, key, token or password. Use `example.com`, `10.0.0.x` and placeholders.
- **Links are relative** (`../user/hosts.md`, `images/hosts-view.png`), so they work on GitHub and in a checkout.

## Screenshots

User-guide images live in [`docs/user/images/`](../user/images/), named after what they show (`hosts-view.png`, not `screenshot-3.png`).

- Take them from the real app, at a normal window size (about 1100×700), in the **Dark** theme and **Classic** layout unless the page is about another theme or layout.
- Use made-up hosts and data. Check every pixel for real addresses, user names, keys or tokens before committing.
- Replace an image when the screen it shows changes noticeably; delete images no page links to.
- Keep each under about 300 KB (PNG); the whole folder small enough to clone quickly.

## Checking your change

Before opening the pull request, check that every relative link and image resolves:

```sh
python3 - <<'EOF'
import re, pathlib
bad = 0
for md in pathlib.Path("docs").rglob("*.md"):
    for target in re.findall(r"\]\(([^)#\s]+)", md.read_text()):
        if "://" in target or target.startswith("mailto:"):
            continue
        if not (md.parent / target).exists():
            print(f"{md}: {target}")
            bad += 1
print("broken links:", bad)
EOF
```

Then read the changed pages on GitHub (the PR's **Files changed → View file**) to see them as readers will, images included.
