// The settings page a build with extensions adds (docs/extending.md). The
// Rust side sends plain data; this file only draws it. Nothing from the
// extension is treated as HTML.

import { el, errorText } from "./dom";
import { extensionAction, extensionPanel, type PanelSection, type PanelStatus, type PanelView } from "./ipc";
import { registerSettingsSection } from "./settings-dialog";

const BADGE: Record<PanelStatus, string> = { ok: "OK", warning: "Check", error: "Problem" };

/** The badge text for a section's status. */
export function badgeText(status: PanelStatus | undefined): string | null {
  return status ? BADGE[status] : null;
}

/** Loads the page; `null` in the open-source build or if it fails to load. */
export async function loadPanel(): Promise<PanelView | null> {
  try {
    return await extensionPanel();
  } catch {
    return null;
  }
}

function section(s: PanelSection, onAction: (id: string, button: HTMLButtonElement) => void): HTMLElement {
  const badge = badgeText(s.status);
  const head = el("h3", { className: "ext-head" }, s.title);
  if (badge && s.status) head.append(el("span", { className: `ext-badge ext-${s.status}` }, badge));
  const box = el("section", { className: "ext-section" }, head);
  if (s.note) box.append(el("p", { className: "muted" }, s.note));
  if (s.rows.length) {
    box.append(
      el(
        "table",
        { className: "ext-rows" },
        el("tbody", {}, ...s.rows.map(([k, v]) => el("tr", {}, el("th", {}, k), el("td", {}, v)))),
      ),
    );
  }
  if (s.lines?.length) box.append(el("pre", { className: "ext-lines" }, s.lines.join("\n")));
  if (s.actions?.length) {
    box.append(
      el(
        "div",
        { className: "ext-actions" },
        ...s.actions.map((a) => {
          const b = el("button", { type: "button" }, a.label);
          b.onclick = () => onAction(a.id, b);
          return b;
        }),
      ),
    );
  }
  return box;
}

/** Adds the page to Settings when this build has one. */
export async function registerExtensionPanel(): Promise<void> {
  const first = await loadPanel();
  if (!first) return;
  registerSettingsSection({
    id: "extension",
    title: first.title,
    render(body) {
      const message = el("p", { className: "ext-message", role: "status" });
      const draw = (view: PanelView) => {
        body.replaceChildren(
          el("h3", {}, view.title),
          message,
          ...view.sections.map((s) =>
            section(s, async (id, button) => {
              button.disabled = true;
              message.textContent = "Working…";
              try {
                message.textContent = (await extensionAction(id)).message;
              } catch (e) {
                message.textContent = errorText(e);
              }
              const next = await loadPanel();
              if (next) draw(next);
            }),
          ),
        );
      };
      draw(first);
      void loadPanel().then((v) => v && draw(v));
    },
  });
}
