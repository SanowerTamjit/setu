// The permissions editor for remote files: read, write and execute for
// owner, group and others, the special bits, the octal mode, and the user
// and group, with the matching chmod/chown command shown as you change it.

import { el, errorText, field, modal } from "./dom";
import { filesSetAttributes, filesStat, type FileEntry } from "./ipc";

const WHO = [
  { label: "Owner", shift: 6 },
  { label: "Group", shift: 3 },
  { label: "Others", shift: 0 },
] as const;
const WHAT = [
  { label: "Read", bit: 4 },
  { label: "Write", bit: 2 },
  { label: "Execute", bit: 1 },
] as const;
const SPECIAL = [
  { label: "Set user ID", bit: 0o4000, note: "runs as the file's owner" },
  { label: "Set group ID", bit: 0o2000, note: "runs as the group; in a folder, new files get its group" },
  { label: "Sticky", bit: 0o1000, note: "in a folder, only owners can delete their files" },
] as const;

/** `rwxr-x---` style, with s/S/t/T for the special bits. */
export function symbolic(mode: number): string {
  let out = "";
  for (const { shift } of WHO) {
    const r = mode & (4 << shift) ? "r" : "-";
    const w = mode & (2 << shift) ? "w" : "-";
    const x = mode & (1 << shift);
    const special = shift === 6 ? mode & 0o4000 : shift === 3 ? mode & 0o2000 : mode & 0o1000;
    const on = shift === 0 ? "t" : "s";
    out += r + w + (special ? (x ? on : on.toUpperCase()) : x ? "x" : "-");
  }
  return out;
}

const octal = (mode: number) => (mode & 0o7777).toString(8).padStart(mode & 0o7000 ? 4 : 3, "0");

/** Shell-quotes a name for the command preview. */
const quote = (s: string) => (/^[A-Za-z0-9._:\/-]+$/.test(s) ? s : `'${s.replace(/'/g, "'\\''")}'`);

/**
 * Opens the editor for `entries` in remote folder `dir`. Resolves true when
 * something changed, so the listing can refresh.
 */
export async function openPermissions(sftpId: number, dir: string, entries: FileEntry[]): Promise<boolean> {
  const first = entries[0];
  const path = `${dir.replace(/\/$/, "")}/${first.name}`;
  const stat = await filesStat(sftpId, path);
  const startMode = stat.permissions ?? 0o644;
  const startUser = stat.user ?? (stat.uid !== null ? String(stat.uid) : "");
  const startGroup = stat.group ?? (stat.gid !== null ? String(stat.gid) : "");
  const hasFolder = entries.some((e) => e.kind === "dir");
  let mode = startMode;

  return new Promise((resolve) => {
    let changed = false;
    const boxes: { input: HTMLInputElement; bit: number }[] = [];
    const grid = el(
      "table",
      { className: "perm-grid" },
      el("thead", {}, el("tr", {}, el("th", {}), ...WHAT.map((w) => el("th", {}, hasFolder && w.bit === 1 ? "Execute / enter" : w.label)))),
      el(
        "tbody",
        {},
        ...WHO.map((who) =>
          el(
            "tr",
            {},
            el("th", { scope: "row" }, who.label),
            ...WHAT.map((what) => {
              const input = el("input", { type: "checkbox" });
              input.setAttribute("aria-label", `${who.label} ${what.label.toLowerCase()}`);
              boxes.push({ input, bit: what.bit << who.shift });
              return el("td", {}, input);
            }),
          ),
        ),
      ),
    );
    const specials = SPECIAL.map((s) => {
      const input = el("input", { type: "checkbox" });
      boxes.push({ input, bit: s.bit });
      return el("label", { className: "perm-special", title: s.note }, input, el("span", {}, s.label));
    });
    const octalInput = el("input", { type: "text", className: "perm-octal", maxLength: 4, spellcheck: false, inputMode: "numeric" });
    const symbolicOut = el("code", { className: "perm-symbolic" });
    const user = el("input", { type: "text", value: startUser, spellcheck: false, autocomplete: "off" });
    const group = el("input", { type: "text", value: startGroup, spellcheck: false, autocomplete: "off" });
    const recursive = el("input", { type: "checkbox" });
    const command = el("code", { className: "perm-command" });
    const message = el("div", { className: "dialog-message error", hidden: true });

    const targets = entries.map((e) => quote(e.name)).join(" ");
    const sync = (fromOctal = false) => {
      for (const b of boxes) b.input.checked = (mode & b.bit) !== 0;
      if (!fromOctal) octalInput.value = octal(mode);
      symbolicOut.textContent = (hasFolder && entries.length === 1 ? "d" : "-") + symbolic(mode);
      const r = recursive.checked ? "-R " : "";
      const lines: string[] = [];
      if (mode !== startMode || entries.length > 1) lines.push(`chmod ${r}${octal(mode)} ${targets}`);
      const u = user.value.trim();
      const g = group.value.trim();
      const owner = u !== startUser ? (g !== startGroup ? `${u}:${g}` : u) : g !== startGroup ? `:${g}` : "";
      if (owner) lines.push(`chown ${r}${quote(owner)} ${targets}`);
      command.textContent = lines.length ? lines.join("\n") : "No changes yet";
      if (recursive.checked && mode !== startMode) {
        command.textContent += `\n# folders, this one included, get ${octal((mode | ((mode & 0o444) >> 2)) & 0o7777)} so they stay enterable`;
      }
    };
    for (const b of boxes) {
      b.input.onchange = () => {
        mode = b.input.checked ? mode | b.bit : mode & ~b.bit;
        sync();
      };
    }
    octalInput.oninput = () => {
      const v = octalInput.value.trim();
      if (/^[0-7]{3,4}$/.test(v)) {
        mode = parseInt(v, 8);
        octalInput.classList.remove("invalid");
        sync(true);
      } else {
        octalInput.classList.add("invalid");
      }
    };
    user.oninput = group.oninput = recursive.onchange = () => sync();

    const apply = el("button", { type: "submit", className: "primary" }, "Apply");
    const cancel = el("button", { type: "button" }, "Cancel");
    const title = entries.length === 1 ? first.name : `${entries.length} items`;
    const form = el(
      "form",
      { className: "dialog perm-dialog" },
      el("h2", {}, `Permissions: ${title}`),
      el("p", { className: "muted perm-path" }, entries.length === 1 ? path : `in ${dir}${entries.length > 1 ? " (shows the first item's current settings)" : ""}`),
      grid,
      el("div", { className: "perm-specials" }, ...specials),
      el("div", { className: "perm-mode" }, field("Mode", octalInput), el("div", { className: "perm-symbolic-wrap" }, symbolicOut)),
      el("div", { className: "row even" }, field("User", user, "A name or a number"), field("Group", group, "A name or a number")),
      ...(hasFolder
        ? [
            el(
              "label",
              { className: "check-row remember-row" },
              recursive,
              el(
                "span",
                {},
                "Also apply to everything inside",
                el("small", { className: "muted" }, "Files inside get this mode. Folders get it with execute added wherever read is allowed, so they can still be opened. Symbolic links are left alone."),
              ),
            ),
          ]
        : []),
      el("div", { className: "perm-preview" }, el("small", { className: "muted" }, "Same as running"), command),
      message,
      el("div", { className: "actions" }, el("span", { className: "spacer" }), cancel, apply),
    );
    const { close } = modal(form, () => resolve(changed));
    cancel.onclick = close;
    form.onsubmit = async (e) => {
      e.preventDefault();
      if (octalInput.classList.contains("invalid")) {
        message.textContent = "The mode must be three or four digits from 0 to 7, like 644 or 2775.";
        message.hidden = false;
        return;
      }
      apply.disabled = true;
      message.hidden = true;
      const u = user.value.trim();
      const g = group.value.trim();
      try {
        await filesSetAttributes(sftpId, dir, entries.map((x) => x.name), {
          mode: mode !== startMode || entries.length > 1 ? mode : null,
          user: u !== startUser ? u : null,
          group: g !== startGroup ? g : null,
          recursive: recursive.checked,
        });
        changed = true;
        close();
      } catch (err) {
        message.textContent = errorText(err);
        message.hidden = false;
        apply.disabled = false;
      }
    };
    sync();
    octalInput.focus();
    octalInput.select();
  });
}
