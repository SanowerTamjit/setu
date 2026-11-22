// The command palette: one search box over every command, saved host,
// shell, theme and snippet. Ctrl+Shift+P by default.

import { allCommands, fuzzyScore, type Command } from "./commands";
import { el, modal } from "./dom";

export interface PaletteOptions {
  /** Shortcut text for a command id, from the active keymap. */
  shortcut: (id: string) => string | undefined;
  /** Commands run recently, most recent first, shown first with no query. */
  recent: () => string[];
  onRun: (id: string) => void;
  /** Starts the search with this text, such as "Connect " or "Theme ". */
  initial?: string;
}

const MAX_SHOWN = 60;

export function openPalette(options: PaletteOptions): Promise<void> {
  return new Promise((resolve) => {
    const input = el("input", {
      type: "text",
      className: "palette-input",
      placeholder: "Type a command, a host to connect to, a theme…",
      spellcheck: false,
      autocomplete: "off",
      value: options.initial ?? "",
    });
    input.setAttribute("aria-label", "Command palette");
    input.setAttribute("role", "combobox");
    input.setAttribute("aria-controls", "palette-list");
    input.setAttribute("aria-expanded", "true");
    const list = el("div", { className: "palette-list", id: "palette-list" });
    list.setAttribute("role", "listbox");
    const empty = el("div", { className: "palette-empty", hidden: true }, "No matching commands");
    const box = el("div", { className: "palette" }, input, list, empty);
    box.setAttribute("role", "dialog");
    box.setAttribute("aria-label", "Command palette");

    let items: Command[] = [];
    let selected = 0;
    const { overlay, close } = modal(box, () => resolve());
    overlay.classList.add("palette-overlay");
    overlay.addEventListener("mousedown", (e) => {
      if (e.target === overlay) close();
    });

    const run = async (c: Command | undefined) => {
      if (!c) return;
      close();
      options.onRun(c.id);
      await c.run();
    };

    const render = () => {
      const query = input.value.trim();
      const all = allCommands();
      if (!query) {
        const recent = options.recent();
        const rank = (c: Command) => {
          const i = recent.indexOf(c.id);
          return i < 0 ? Infinity : i;
        };
        items = [...all].sort((a, b) => rank(a) - rank(b)).slice(0, MAX_SHOWN);
      } else {
        items = all
          .map((c) => ({
            c,
            score: fuzzyScore(query, `${c.category ? c.category + ": " : ""}${c.title} ${c.keywords ?? ""}`),
          }))
          .filter((x): x is { c: Command; score: number } => x.score !== null)
          .sort((a, b) => b.score - a.score)
          .slice(0, MAX_SHOWN)
          .map((x) => x.c);
      }
      selected = Math.min(selected, Math.max(items.length - 1, 0));
      empty.hidden = items.length > 0;
      list.replaceChildren(
        ...items.map((c, i) => {
          const keys = options.shortcut(c.id);
          const row = el(
            "div",
            { className: "palette-item" + (i === selected ? " selected" : ""), id: `palette-item-${i}` },
            el(
              "span",
              { className: "palette-title" },
              ...(c.category ? [el("span", { className: "palette-category" }, `${c.category}: `)] : []),
              c.title,
            ),
            ...(keys ? [el("kbd", { className: "palette-keys" }, keys)] : []),
          );
          row.setAttribute("role", "option");
          row.setAttribute("aria-selected", String(i === selected));
          row.onmousemove = () => {
            if (selected !== i) {
              selected = i;
              highlight();
            }
          };
          row.onclick = () => void run(c);
          return row;
        }),
      );
      highlight();
    };

    const highlight = () => {
      [...list.children].forEach((child, i) => {
        child.classList.toggle("selected", i === selected);
        child.setAttribute("aria-selected", String(i === selected));
      });
      input.setAttribute("aria-activedescendant", `palette-item-${selected}`);
      list.children[selected]?.scrollIntoView({ block: "nearest" });
    };

    input.addEventListener("input", () => {
      selected = 0;
      render();
    });
    input.addEventListener("keydown", (e) => {
      if (e.key === "ArrowDown" || (e.ctrlKey && e.key === "n")) {
        e.preventDefault();
        selected = Math.min(selected + 1, items.length - 1);
        highlight();
      } else if (e.key === "ArrowUp" || (e.ctrlKey && e.key === "p")) {
        e.preventDefault();
        selected = Math.max(selected - 1, 0);
        highlight();
      } else if (e.key === "Enter") {
        e.preventDefault();
        void run(items[selected]);
      }
    });

    render();
    input.focus();
    input.setSelectionRange(input.value.length, input.value.length);
  });
}
