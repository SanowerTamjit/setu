// Settings → Shells: the default shell for new tabs, and your own profiles
// (a detected shell plus arguments, a starting folder and variables).

import { el, field } from "./dom";
import { listShells, type ShellProfile } from "./ipc";
import { settings, updateSettings, type ShellProfileSetting } from "./settings";
import { registerSettingsSection } from "./settings-dialog";

/** Splits `--login -c "echo hi"` into words; double quotes group words. */
export function splitArgs(text: string): string[] {
  const out: string[] = [];
  const re = /"([^"]*)"|(\S+)/g;
  for (const m of text.matchAll(re)) out.push(m[1] ?? m[2]);
  return out;
}

export function joinArgs(args: string[]): string {
  return args.map((a) => (/\s/.test(a) || a === "" ? `"${a}"` : a)).join(" ");
}

/** Reads `NAME=value` lines; blank lines and `#` comments are skipped. */
export function parseEnv(text: string): { env: Record<string, string> } | { error: string } {
  const env: Record<string, string> = {};
  for (const [i, raw] of text.split("\n").entries()) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const eq = line.indexOf("=");
    const name = eq < 0 ? line : line.slice(0, eq).trim();
    if (eq < 0 || !/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) {
      return { error: `Line ${i + 1}: write variables as NAME=value (letters, digits and _ in the name).` };
    }
    env[name] = line.slice(eq + 1).trim();
  }
  return { env };
}

let detected: ShellProfile[] | null = null;

function nextId(profiles: readonly ShellProfileSetting[]) {
  const used = new Set(profiles.map((p) => p.id));
  let n = 1;
  while (used.has(`custom:${n}`)) n++;
  return `custom:${n}`;
}

function editor(
  shells: ShellProfile[],
  existing: ShellProfileSetting | null,
  onDone: (saved: ShellProfileSetting | null) => void,
) {
  const name = el("input", { type: "text", value: existing?.name ?? "", placeholder: "Work API" });
  const base = el(
    "select",
    {},
    ...shells.map((s) =>
      el("option", { value: s.id, selected: s.id === existing?.base }, `${s.name} (${s.program})`),
    ),
  );
  const cwd = el("input", { type: "text", value: existing?.cwd ?? "", placeholder: "~/projects/api" });
  const args = el("input", { type: "text", value: joinArgs(existing?.args ?? []), placeholder: "--login" });
  const env = el("textarea", {
    rows: 4,
    spellcheck: false,
    value: Object.entries(existing?.env ?? {})
      .map(([k, v]) => `${k}=${v}`)
      .join("\n"),
    placeholder: "AWS_PROFILE=staging\nKUBECONFIG=~/.kube/staging",
  });
  const error = el("p", { className: "danger", hidden: true });
  const save = el("button", { type: "button", className: "primary" }, existing ? "Save profile" : "Add profile");
  const cancel = el("button", { type: "button" }, "Cancel");
  cancel.onclick = () => onDone(null);
  save.onclick = () => {
    const parsed = parseEnv(env.value);
    const fail = (text: string) => {
      error.textContent = text;
      error.hidden = false;
    };
    if (!name.value.trim()) return fail("Give the profile a name.");
    if ("error" in parsed) return fail(parsed.error);
    onDone({
      id: existing?.id ?? nextId(settings().shellProfiles),
      name: name.value.trim(),
      base: base.value,
      args: splitArgs(args.value),
      cwd: cwd.value.trim(),
      env: parsed.env,
    });
  };
  const isWsl = () => base.value.startsWith("wsl:");
  const hint = el("small", { className: "muted" });
  const updateHint = () => {
    hint.textContent = isWsl()
      ? "For a WSL distro the folder is a Linux path, and the variables are passed into Linux."
      : "Leave the folder empty to start in your home folder.";
  };
  base.onchange = updateHint;
  updateHint();
  const form = el(
    "div",
    { className: "profile-editor" },
    el("div", { className: "row even" }, field("Name", name), field("Starts from", base)),
    field("Starting folder", cwd),
    hint,
    field("Extra arguments", args, "Added after the shell's own. Use double quotes to group words."),
    field("Environment variables", env, "One NAME=value per line."),
    error,
    el("div", { className: "actions" }, el("span", { className: "spacer" }), cancel, save),
  );
  queueMicrotask(() => name.focus());
  return form;
}

registerSettingsSection({
  id: "shells",
  title: "Shells",
  render(body, rerender) {
    if (!detected) {
      body.append(el("p", { className: "muted" }, "Looking for shells…"));
      void listShells()
        .then((found) => (detected = found))
        .catch(() => (detected = []))
        .then(rerender);
      return;
    }
    const shells = detected;
    const profiles = settings().shellProfiles;
    const names = new Map(shells.map((s) => [s.id, s.name]));

    const choice = el(
      "select",
      {},
      el("option", { value: "", selected: settings().defaultShell === null }, "System default"),
      ...profiles.map((p) => el("option", { value: p.id, selected: settings().defaultShell === p.id }, p.name)),
      ...shells.map((s) => el("option", { value: s.id, selected: settings().defaultShell === s.id }, s.name)),
    );
    choice.onchange = () => updateSettings({ defaultShell: choice.value || null });

    const editing = el("div", {});
    const showEditor = (existing: ShellProfileSetting | null) => {
      editing.replaceChildren(
        editor(shells, existing, (saved) => {
          if (saved) {
            const list = settings().shellProfiles.filter((p) => p.id !== saved.id);
            const at = settings().shellProfiles.findIndex((p) => p.id === saved.id);
            list.splice(at < 0 ? list.length : at, 0, saved);
            updateSettings({ shellProfiles: list });
          }
          rerender();
        }),
      );
      add.hidden = true;
    };

    const rows = profiles.map((p) => {
      const edit = el("button", { type: "button" }, "Edit");
      edit.onclick = () => showEditor(p);
      const remove = el("button", { type: "button", className: "danger-button" }, "Delete");
      remove.onclick = () => {
        updateSettings({
          shellProfiles: settings().shellProfiles.filter((q) => q.id !== p.id),
          ...(settings().defaultShell === p.id ? { defaultShell: null } : {}),
        });
        rerender();
      };
      const vars = Object.keys(p.env);
      const facts = [
        names.get(p.base) ?? `${p.base} (not found)`,
        p.cwd || "home folder",
        ...(vars.length ? [vars.join(", ")] : []),
      ];
      return el(
        "div",
        { className: "profile-row" },
        el("span", {}, el("strong", {}, p.name), el("small", { className: "muted" }, facts.join(" · "))),
        el("span", { className: "profile-actions" }, edit, remove),
      );
    });

    const add = el("button", { type: "button" }, "New profile…");
    add.onclick = () => showEditor(null);

    body.append(
      el("h3", {}, "New tabs start"),
      field("Shell", choice),
      el("h3", {}, "Your profiles"),
      ...(rows.length
        ? [el("div", { className: "profile-list" }, ...rows)]
        : [el("p", { className: "muted" }, "None yet. A profile starts a shell with its own folder and variables.")]),
      editing,
      add,
      el("h3", {}, "Found on this computer"),
      el(
        "ul",
        { className: "shell-list" },
        ...shells.map((s) =>
          el("li", {}, el("strong", {}, s.name), " ", el("small", { className: "muted" }, s.program)),
        ),
      ),
    );
  },
});
