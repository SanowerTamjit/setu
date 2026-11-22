// The settings dialog: a list of sections on the left, the chosen one on the
// right. Features add their own sections with `registerSettingsSection`.

import { el, field, modal } from "./dom";
import { bindingsFor, KEYMAPS } from "./keymap";
import { allCommands } from "./commands";
import { settings, updateSettings, type RendererName } from "./settings";
import { THEMES } from "./themes";

export interface SettingsSection {
  id: string;
  title: string;
  render: (body: HTMLElement, rerender: () => void) => void;
}

const sections: SettingsSection[] = [];

export function registerSettingsSection(section: SettingsSection) {
  sections.push(section);
}

/** Theme cards show a small preview drawn in each theme's own colours. */
function themeCard(id: (typeof THEMES)[number]["id"], name: string, description: string, onPick: () => void) {
  const chosen = settings().theme === id;
  const card = el(
    "button",
    { type: "button", className: "theme-card" + (chosen ? " chosen" : "") },
    el(
      "span",
      { className: `theme-preview` },
      el("span", { className: "tp-bar" }),
      el("span", { className: "tp-line a" }),
      el("span", { className: "tp-line b" }),
      el("span", { className: "tp-line c" }),
    ),
    el("strong", {}, name),
    el("small", { className: "muted" }, description),
  );
  card.dataset.previewTheme = id;
  card.setAttribute("aria-pressed", String(chosen));
  card.onclick = onPick;
  return card;
}

registerSettingsSection({
  id: "appearance",
  title: "Appearance",
  render(body, rerender) {
    const cards = el(
      "div",
      { className: "theme-cards" },
      ...THEMES.map((t) =>
        themeCard(t.id, t.name, t.description, () => {
          updateSettings({ theme: t.id });
          rerender();
        }),
      ),
    );
    const size = el("input", { type: "number", min: "8", max: "32", value: String(settings().fontSize) });
    size.onchange = () => {
      const n = Number(size.value);
      if (n >= 8 && n <= 32) updateSettings({ fontSize: n });
    };
    body.append(
      el("h3", {}, "Theme"),
      cards,
      el("h3", {}, "Terminal font size"),
      el("label", { className: "inline-field" }, size, el("span", { className: "muted" }, "points")),
    );
  },
});

registerSettingsSection({
  id: "keys",
  title: "Keybindings",
  render(body, rerender) {
    const options = el(
      "div",
      { className: "choice-list" },
      ...KEYMAPS.map((k) => {
        const radio = el("input", { type: "radio", name: "keymap", checked: settings().keymap === k.id });
        radio.onchange = () => {
          updateSettings({ keymap: k.id });
          rerender();
        };
        return el(
          "label",
          { className: "choice" },
          radio,
          el("span", {}, el("strong", {}, k.name), el("small", { className: "muted" }, k.description)),
        );
      }),
    );
    const bindings = bindingsFor(settings().keymap);
    const titles = new Map(allCommands().map((c) => [c.id, c.category ? `${c.category}: ${c.title}` : c.title]));
    const rows = Object.entries(bindings)
      .filter(([id]) => titles.has(id))
      .map(([id, keys]) =>
        el(
          "tr",
          {},
          el("td", {}, titles.get(id)!),
          el("td", {}, ...keys.map((k) => el("kbd", { className: "palette-keys" }, k))),
        ),
      );
    body.append(
      el("h3", {}, "Profile"),
      options,
      el(
        "p",
        { className: "muted" },
        "Every profile keeps the Ctrl+Shift shortcuts. Vim and Emacs add a prefix key; press the prefix twice to send it to the shell.",
      ),
      el("h3", {}, "Shortcuts"),
      el("table", { className: "keys-table" }, el("tbody", {}, ...rows)),
    );
  },
});

registerSettingsSection({
  id: "terminal",
  title: "Terminal",
  render(body) {
    const mask = el("input", { type: "checkbox", checked: settings().maskSecrets });
    mask.onchange = () => updateSettings({ maskSecrets: mask.checked });
    const renderer = el(
      "select",
      {},
      el("option", { value: "auto" }, "Automatic"),
      el("option", { value: "gpu" }, "GPU (WebGL)"),
      el("option", { value: "dom" }, "Compatible (DOM)"),
    );
    renderer.value = settings().renderer;
    renderer.onchange = () => updateSettings({ renderer: renderer.value as RendererName });
    const font = el("input", {
      type: "text",
      value: settings().fontFamily,
      placeholder: "JetBrains Mono, Cascadia Code, Fira Code…",
      spellcheck: false,
    });
    font.onchange = () => updateSettings({ fontFamily: font.value.trim() });
    const ligatures = el("input", { type: "checkbox", checked: settings().ligatures });
    ligatures.onchange = () => updateSettings({ ligatures: ligatures.checked });
    body.append(
      el("h3", {}, "Rendering"),
      field(
        "Renderer",
        renderer,
        "Automatic draws with the GPU on macOS and Windows. On Linux it uses the compatible renderer, because some WebKitGTK setups show new output a frame late with the GPU; pick GPU to try it. The status bar shows which one a terminal uses.",
      ),
      field("Font", font, "Leave empty for the first installed of JetBrains Mono, Cascadia Code, Fira Code, SF Mono, Menlo or Consolas."),
      el(
        "label",
        { className: "check-row remember-row" },
        ligatures,
        el(
          "span",
          {},
          "Font ligatures",
          el("small", { className: "muted" }, "Join sequences such as -> => != and >= into one symbol, when the font has them."),
        ),
      ),
      el("h3", {}, "Privacy"),
      el(
        "label",
        { className: "check-row remember-row" },
        mask,
        el(
          "span",
          {},
          "Hide secrets in terminal output",
          el(
            "small",
            { className: "muted" },
            "Covers things that look like keys, tokens and passwords (cloud keys, private key blocks, passwords in URLs) with a bar. Hover a bar to see what kind of secret it hid, or click it to show that one. Selecting and copying still gives the real text.",
          ),
        ),
      ),
    );
  },
});

export function openSettings(sectionId?: string): Promise<void> {
  return new Promise((resolve) => {
    let current = sections.find((s) => s.id === sectionId) ?? sections[0];
    const nav = el("nav", { className: "settings-nav" });
    const body = el("div", { className: "settings-body" });
    const done = el("button", { type: "button", className: "primary" }, "Done");
    const dialog = el(
      "div",
      { className: "dialog settings-dialog" },
      el("h2", {}, "Settings"),
      el("div", { className: "settings-layout" }, nav, body),
      el("div", { className: "actions" }, el("span", { className: "spacer" }), done),
    );
    dialog.setAttribute("role", "dialog");
    dialog.setAttribute("aria-label", "Settings");
    const { close } = modal(dialog, resolve);
    done.onclick = close;

    const render = () => {
      nav.replaceChildren(
        ...sections.map((s) => {
          const b = el("button", { type: "button", className: "settings-tab" + (s === current ? " active" : "") }, s.title);
          b.onclick = () => {
            current = s;
            render();
          };
          return b;
        }),
      );
      body.replaceChildren();
      current.render(body, render);
    };
    render();
    done.focus();
  });
}
