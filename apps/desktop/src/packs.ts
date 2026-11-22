// Packs in the app: which are on, their snippets in the palette, the role
// picker shown on first launch, and Settings → Packs.

import { registerProvider } from "./commands";
import { el, errorText, modal } from "./dom";
import { packInspect, packInstall, packRemove, packsList, type PackManifest, type PackView } from "./ipc";
import { settings, updateSettings } from "./settings";
import { registerSettingsSection } from "./settings-dialog";

export const ROLES = [
  { id: "sysadmin", name: "System administrator", note: "Servers, services, disks, logs, users and updates", ready: true },
  { id: "devops", name: "DevOps engineer", note: "Pipelines, containers, Kubernetes, infrastructure as code", ready: false },
  { id: "secops", name: "Security operations", note: "Hardening, audits, incident response", ready: false },
  { id: "netops", name: "Network engineer", note: "Routers, switches, firewalls and VLANs", ready: false },
  { id: "itops", name: "IT operations", note: "Desktops, accounts and the service desk", ready: false },
] as const;

let packs: PackView[] = [];

export async function loadPacks() {
  packs = await packsList().catch(() => []);
  return packs;
}

/** On unless turned off by hand; a built-in role pack follows the chosen role. */
export function packEnabled(p: PackView) {
  const manual = settings().packs[p.id];
  if (manual !== undefined) return manual;
  if (p.builtIn && p.kind === "role-pack") return p.roles.includes(settings().role ?? "");
  return true;
}

/** Puts snippets in the palette; picking one types it into the terminal without running it. */
export function registerSnippets(insert: (command: string) => void) {
  registerProvider(() =>
    packs.filter(packEnabled).flatMap((p) =>
      p.contributes.snippets.map((s) => ({
        id: `snippet:${p.id}:${s.id}`,
        title: s.title,
        category: "Snippet",
        keywords: `${s.command} ${(s.tags ?? []).join(" ")} ${p.name}`,
        run: () => insert(s.command),
      })),
    ),
  );
}

/** The first-launch question: which role? Resolves once answered or skipped. */
export function openRolePicker(): Promise<void> {
  return new Promise((resolve) => {
    let chosen: string = settings().role && settings().role !== "none" ? settings().role! : "sysadmin";
    const list = el(
      "div",
      { className: "choice-list" },
      ...ROLES.map((r) => {
        const radio = el("input", { type: "radio", name: "role", checked: r.id === chosen, disabled: !r.ready });
        radio.onchange = () => (chosen = r.id);
        return el(
          "label",
          { className: "choice" + (r.ready ? "" : " disabled") },
          radio,
          el(
            "span",
            {},
            el("strong", {}, r.name, ...(r.ready ? [] : [el("span", { className: "badge" }, "Later release")])),
            el("small", { className: "muted" }, r.note),
          ),
        );
      }),
    );
    const go = el("button", { type: "submit", className: "primary" }, "Continue");
    const skip = el("button", { type: "button" }, "Skip for now");
    const form = el(
      "form",
      { className: "dialog role-dialog" },
      el("h2", {}, "Welcome to Setu"),
      el("p", { className: "muted" }, "What kind of work do you do? Setu turns on the matching pack: snippets for everyday jobs, in the command palette. You can change this later in Settings."),
      list,
      el("div", { className: "actions" }, skip, el("span", { className: "spacer" }), go),
    );
    const { close } = modal(form, resolve);
    // Escape counts as skipping, so the question is not asked again.
    const answer = (role: string) => {
      updateSettings({ role });
      close();
    };
    form.onsubmit = (e) => {
      e.preventDefault();
      answer(chosen);
    };
    skip.onclick = () => answer("none");
    go.focus();
  });
}

function readFile(): Promise<string | null> {
  return new Promise((resolve) => {
    // WebKitGTK only opens the file chooser for an input in the page.
    const input = el("input", { type: "file", accept: ".json,application/json", hidden: true });
    document.body.append(input);
    input.onchange = async () => {
      resolve((await input.files?.[0]?.text()) ?? null);
      input.remove();
    };
    input.oncancel = () => {
      resolve(null);
      input.remove();
    };
    input.click();
  });
}

function packCard(p: PackView, rerender: () => void, flash: (t: string, e?: boolean) => void) {
  const toggle = el("input", { type: "checkbox", checked: packEnabled(p) });
  toggle.onchange = () => {
    updateSettings({ packs: { ...settings().packs, [p.id]: toggle.checked } });
    rerender();
  };
  const remove = el("button", { type: "button", className: "danger-button" }, "Remove");
  remove.onclick = async () => {
    try {
      await packRemove(p.id);
      const next = { ...settings().packs };
      delete next[p.id];
      updateSettings({ packs: next });
      await loadPacks();
      rerender();
    } catch (err) {
      flash(errorText(err), true);
    }
  };
  const n = p.contributes.snippets.length;
  return el(
    "div",
    { className: "pack-card" },
    el(
      "label",
      { className: "pack-head" },
      toggle,
      el(
        "span",
        { className: "pack-title" },
        el("strong", {}, p.name),
        el("small", { className: "muted" }, `${p.id} · ${p.version}${p.builtIn ? " · built in" : ""}`),
      ),
    ),
    ...(p.description ? [el("p", { className: "muted pack-desc" }, p.description)] : []),
    el(
      "div",
      { className: "pack-facts" },
      el("span", { className: "badge" }, `${n} ${n === 1 ? "snippet" : "snippets"}`),
      el("span", { className: "badge" }, p.permissions.length ? `Asks for: ${p.permissions.join(", ")}` : "Asks for no permissions"),
      ...(p.builtIn ? [] : [el("span", { className: "spacer" }), remove]),
    ),
  );
}

function installPreview(
  body: HTMLElement,
  text: string,
  manifest: PackManifest,
  replaces: string | null,
  done: (installed: boolean) => void,
  flash: (t: string, e?: boolean) => void,
) {
  const install = el("button", { type: "button", className: "primary" }, replaces ? `Replace version ${replaces}` : "Install");
  const cancel = el("button", { type: "button" }, "Cancel");
  cancel.onclick = () => done(false);
  install.onclick = async () => {
    install.disabled = true;
    try {
      await packInstall(text);
      flash(`Installed ${manifest.name} ${manifest.version}. Its snippets are in the command palette.`);
      done(true);
    } catch (err) {
      flash(errorText(err), true);
      install.disabled = false;
    }
  };
  const shown = manifest.contributes.snippets.slice(0, 6);
  body.replaceChildren(
    el("h3", {}, `Install ${manifest.name}?`),
    el(
      "div",
      { className: "pack-card install" },
      el("strong", {}, `${manifest.name} ${manifest.version}`),
      el("small", { className: "muted" }, manifest.id),
      ...(manifest.description ? [el("p", { className: "pack-desc" }, manifest.description)] : []),
      el(
        "p",
        {},
        el("strong", {}, "Permissions: "),
        manifest.permissions.length ? manifest.permissions.join(", ") : "none. It only adds snippets; nothing in it runs until you press Enter.",
      ),
      el("p", { className: "muted" }, `Adds ${manifest.contributes.snippets.length} snippets, such as:`),
      el("ul", { className: "snippet-preview" }, ...shown.map((s) => el("li", {}, el("span", {}, s.title), el("code", {}, s.command)))),
    ),
    el("div", { className: "actions" }, el("span", { className: "spacer" }), cancel, install),
  );
}

registerSettingsSection({
  id: "packs",
  title: "Packs",
  render(body, rerender) {
    const message = el("div", { className: "dialog-message", hidden: true });
    const flash = (text: string, error = false) => {
      message.textContent = text;
      message.className = `dialog-message ${error ? "error" : "info"}`;
      message.hidden = false;
    };
    const role = ROLES.find((r) => r.id === settings().role);
    const change = el("button", { type: "button" }, "Change role…");
    change.onclick = async () => {
      await openRolePicker();
      rerender();
    };
    const install = el("button", { type: "button" }, "Install from file…");
    install.onclick = async () => {
      const text = await readFile();
      if (!text) return;
      try {
        const { manifest, replaces } = await packInspect(text);
        installPreview(body, text, manifest, replaces, async (installed) => {
          if (installed) await loadPacks();
          rerender();
          if (installed) {
            const again = body.querySelector<HTMLElement>(".dialog-message");
            if (again) {
              again.textContent = `Installed ${manifest.name} ${manifest.version}. Its snippets are in the command palette.`;
              again.className = "dialog-message info";
              again.hidden = false;
            }
          }
        }, flash);
        body.prepend(message);
      } catch (err) {
        flash(errorText(err), true);
      }
    };
    body.append(
      message,
      el("h3", {}, "Your role"),
      el("div", { className: "inline-field" }, el("span", {}, role ? role.name : "Not chosen"), change),
      el("h3", {}, "Packs"),
      ...packs.map((p) => packCard(p, rerender, flash)),
      el("div", { className: "pack-install" }, install, el("small", { className: "muted" }, "A pack is one .json file. Snippets type a command into the terminal; you press Enter.")),
    );
  },
});
