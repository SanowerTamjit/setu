// Settings → Prompts: terminal colour schemes from other terminals, and help
// setting up Powerlevel10k, Oh My Zsh, Starship and Oh My Posh (see
// prompt-themes.ts). Setu never runs these tools' installers; it shows what it
// found and lines to copy.

import { copyText, el, errorText } from "./dom";
import { colorSchemeParse, promptDetect, type DetectedPrompt } from "./ipc";
import { activeScheme, addSchemes, currentTerminalTheme, PROMPT_TOOLS, promptThemesOn, SYMBOLS_FAMILY } from "./prompt-themes";
import { registerSettingsSection } from "./settings-dialog";
import { settings, updateSettings } from "./settings";
import { terminalFont } from "./terminal-tab";
import { setEnabled } from "./features";

/** A small prompt drawn in the terminal's colours and font. */
function preview(): HTMLElement {
  const t = currentTerminalTheme();
  const ansi = [t.black, t.red, t.green, t.yellow, t.blue, t.magenta, t.cyan, t.white, t.brightBlack, t.brightRed, t.brightGreen, t.brightYellow, t.brightBlue, t.brightMagenta, t.brightCyan, t.brightWhite];
  const span = (text: string, color?: string, background?: string) => {
    const s = el("span", {}, text);
    if (color) s.style.color = color;
    if (background) s.style.background = background;
    return s;
  };
  const box = el(
    "div",
    { className: "prompt-preview" },
    el(
      "div",
      {},
      span("  ", t.black, t.white),
      span("", t.white, t.blue),
      span("  ~/src/setu ", t.brightWhite, t.blue),
      span("", t.blue, t.green),
      span("  main ", t.black, t.green),
      span("", t.green),
    ),
    el("div", {}, span("❯ ", t.green), span("git status"), span(" ", undefined, t.cursor)),
    el("div", { className: "prompt-swatches" }, ...ansi.map((c) => span("", undefined, c ?? "transparent"))),
  );
  box.style.background = t.background ?? "";
  box.style.color = t.foreground ?? "";
  box.style.fontFamily = terminalFont(settings().fontFamily);
  return box;
}

function toolRow(info: (typeof PROMPT_TOOLS)[number], found: DetectedPrompt | undefined) {
  const state = !found
    ? el("span", { className: "prompt-state" }, "Checking…")
    : found.enabled
      ? el("span", { className: "prompt-state on" }, "● In use")
      : found.installed
        ? el("span", { className: "prompt-state found" }, "● Installed, not turned on")
        : el("span", { className: "prompt-state" }, "○ Not found");
  const lines = info.setup.map((line) => {
    const copy = el("button", { type: "button", className: "small" }, "Copy");
    copy.onclick = async () => {
      await copyText(line.text);
      copy.textContent = "Copied";
      setTimeout(() => (copy.textContent = "Copy"), 1500);
    };
    return el("div", { className: "prompt-line" }, el("small", { className: "muted" }, line.label), el("code", {}, line.text), copy);
  });
  return el(
    "div",
    { className: "prompt-tool" },
    el(
      "div",
      { className: "prompt-tool-head" },
      el("strong", {}, info.name),
      el("small", { className: "muted" }, `${info.shells} · ${info.site}`),
      state,
    ),
    ...(found?.detail ? [el("small", { className: "muted" }, found.enabled ? `Loaded from ${found.detail}` : found.detail)] : []),
    ...(found?.enabled ? [] : lines),
  );
}

let detected: DetectedPrompt[] | null = null;
/** The result of the last import, shown until the choice changes. */
let note: { kind: "info" | "error"; text: string } | null = null;

registerSettingsSection({
  id: "prompts",
  title: "Prompts",
  render(body, rerender) {
    if (!promptThemesOn()) {
      const on = el("button", { type: "button" }, "Turn on prompt themes");
      on.onclick = () => {
        setEnabled("promptThemes", true);
        rerender();
      };
      body.append(
        el("h3", {}, "Prompt themes"),
        el("p", { className: "muted" }, "Prompt themes are turned off in Settings → Features: terminals use your font as it is, without the built-in icons, and the app theme's colours."),
        on,
      );
      return;
    }

    // Colour schemes.
    const select = el("select", {}, el("option", { value: "" }, "Follow the app theme"), ...settings().colorSchemes.map((s) => el("option", { value: s.name }, s.name)));
    select.value = activeScheme()?.name ?? "";
    select.onchange = () => {
      note = null;
      updateSettings({ colorScheme: select.value });
      rerender();
    };
    const message = el("div", { className: `dialog-message ${note?.kind ?? ""}`, hidden: !note }, note?.text ?? "");
    // Any file: Xresources and Ghostty themes have no extension.
    const file = el("input", { type: "file", hidden: true });
    const importButton = el("button", { type: "button" }, "Import a scheme…");
    importButton.onclick = () => file.click();
    file.onchange = async () => {
      const f = file.files?.[0];
      file.value = "";
      if (!f) return;
      try {
        const scan = await colorSchemeParse(f.name, new Uint8Array(await f.arrayBuffer()));
        const names = addSchemes(scan.schemes);
        if (!names.length) throw new Error("none of the schemes in that file could be kept");
        updateSettings({ colorScheme: names[0] });
        note = {
          kind: "info",
          text:
            names.length === 1
              ? `Imported “${names[0]}” (${scan.format}). Terminals use it now.`
              : `Imported ${names.length} schemes (${scan.format}). Terminals use “${names[0]}” now.`,
        };
      } catch (e) {
        note = { kind: "error", text: errorText(e) };
      }
      rerender();
    };
    const remove = el("button", { type: "button", disabled: !activeScheme() }, "Remove");
    remove.onclick = () => {
      const name = activeScheme()?.name;
      if (!name) return;
      note = null;
      updateSettings({ colorSchemes: settings().colorSchemes.filter((s) => s.name !== name), colorScheme: "" });
      rerender();
    };

    // Prompt tools on this computer.
    const tools = el("div", { className: "prompt-tools" }, ...PROMPT_TOOLS.map((t) => toolRow(t, detected?.find((d) => d.id === t.id))));
    if (!detected) {
      void promptDetect()
        .then((d) => {
          detected = d;
          if (tools.isConnected) rerender();
        })
        .catch(() => {
          detected = [];
          if (tools.isConnected) rerender();
        });
    }
    const recheck = el("button", { type: "button", className: "small" }, "Check again");
    recheck.onclick = () => {
      detected = null;
      rerender();
    };

    body.append(
      el("h3", {}, "Terminal colours"),
      preview(),
      el("div", { className: "prompt-scheme-row" }, select, importButton, remove, file),
      message,
      el(
        "p",
        { className: "muted" },
        "Bring the look of another terminal: iTerm2 .itermcolors, Windows Terminal JSON (a scheme or a whole settings.json), Alacritty .toml, base16 or base24 .yaml, and kitty, Ghostty or Xresources colour files. Only the colours are read. Thousands of ready-made schemes are at iterm2colorschemes.com.",
      ),
      el("h3", {}, "Icons and Powerline symbols"),
      el(
        "p",
        { className: "muted" },
        `Setu has the Nerd Font symbols built in (“${SYMBOLS_FAMILY}”), so prompt icons and Powerline arrows show with any terminal font, and it counts character widths the way zsh and bash do (Unicode 11). You do not need to install a patched font; if you use one, such as MesloLGS NF, set it in Settings → Terminal and its own icons are used. Run “Prompt check” from the command palette to see every kind of glyph in a terminal.`,
      ),
      el("div", { className: "prompt-tools-title" }, el("h3", {}, "Prompt themes on this computer"), recheck),
      el(
        "p",
        { className: "muted" },
        "Setu looks for these in your home folder and shell start-up files. It never installs or runs them for you: copy a line into a terminal or your start-up file, then open a new tab. Over SSH, set them up on the server the same way; the icons and colours work there too.",
      ),
      tools,
    );
  },
});
