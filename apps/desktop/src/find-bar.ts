// Find in a terminal's scrollback: a small bar over the top-right corner of
// the terminal, driven by xterm's search addon.

import type { Terminal } from "@xterm/xterm";
import { SearchAddon, type ISearchOptions } from "@xterm/addon-search";
import { el } from "./dom";
import { settings } from "./settings";
import { findColors } from "./themes";

/** Matches beyond this many are found but not all highlighted, to stay fast. */
const HIGHLIGHT_LIMIT = 1000;

export class FindBar {
  readonly element: HTMLDivElement;
  private readonly addon = new SearchAddon({ highlightLimit: HIGHLIGHT_LIMIT });
  private readonly input: HTMLInputElement;
  private readonly count: HTMLSpanElement;
  private readonly caseButton: HTMLButtonElement;
  private readonly regexButton: HTMLButtonElement;
  private readonly wordButton: HTMLButtonElement;
  private caseSensitive = false;
  private regex = false;
  private wholeWord = false;

  constructor(
    host: HTMLElement,
    private readonly term: Terminal,
  ) {
    term.loadAddon(this.addon);

    this.input = el("input", {
      type: "text",
      placeholder: "Find in scrollback",
      spellcheck: false,
      autocomplete: "off",
      className: "find-input",
    });
    this.input.setAttribute("aria-label", "Find in scrollback");
    this.count = el("span", { className: "find-count" });
    this.count.setAttribute("aria-live", "polite");

    const toggle = (label: string, title: string, onClick: () => void) => {
      const b = el("button", { className: "find-toggle", title, type: "button" }, label);
      b.setAttribute("aria-pressed", "false");
      b.onclick = () => {
        onClick();
        this.search(true);
        this.input.focus();
      };
      return b;
    };
    this.caseButton = toggle("Aa", "Match case", () => (this.caseSensitive = !this.caseSensitive));
    this.wordButton = toggle("ab", "Whole word", () => (this.wholeWord = !this.wholeWord));
    this.regexButton = toggle(".*", "Regular expression", () => (this.regex = !this.regex));

    const button = (label: string, title: string, onClick: () => void) => {
      const b = el("button", { className: "find-button", title, type: "button" }, label);
      b.onclick = () => {
        onClick();
        this.input.focus();
      };
      return b;
    };

    this.element = el(
      "div",
      { className: "find-bar", hidden: true },
      this.input,
      this.count,
      this.caseButton,
      this.wordButton,
      this.regexButton,
      button("↑", "Previous match (Shift+Enter)", () => this.previous()),
      button("↓", "Next match (Enter)", () => this.next()),
      button("✕", "Close (Escape)", () => this.close()),
    );
    this.element.setAttribute("role", "search");
    host.append(this.element);

    this.addon.onDidChangeResults(({ resultIndex, resultCount }) => this.showCount(resultIndex, resultCount));

    this.input.addEventListener("input", () => this.search(true));
    this.input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        if (e.shiftKey) this.previous();
        else this.next();
      } else if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        this.close();
      }
    });
  }

  get isOpen() {
    return !this.element.hidden;
  }

  /** Opens the bar, prefilled with the terminal's selection if there is one on a single line. */
  open() {
    const selection = this.term.getSelection();
    if (selection && !selection.includes("\n") && selection.length <= 200) {
      this.input.value = selection;
    }
    this.element.hidden = false;
    this.input.focus();
    this.input.select();
    if (this.input.value) this.search(true);
  }

  close() {
    if (this.element.hidden) return;
    this.element.hidden = true;
    this.addon.clearDecorations();
    this.term.clearSelection();
    this.term.focus();
  }

  next() {
    this.search(false);
  }

  previous() {
    const query = this.input.value;
    if (!query || !this.valid(query)) return;
    this.addon.findPrevious(query, this.options(false));
  }

  private options(incremental: boolean): ISearchOptions {
    return {
      caseSensitive: this.caseSensitive,
      regex: this.regex,
      wholeWord: this.wholeWord,
      incremental,
      decorations: findColors(settings().theme),
    };
  }

  /** An invalid regular expression would throw inside the addon; show it instead. */
  private valid(query: string) {
    this.input.classList.remove("invalid");
    this.count.classList.remove("none");
    if (!this.regex) return true;
    try {
      new RegExp(query);
      return true;
    } catch {
      this.input.classList.add("invalid");
      this.addon.clearDecorations();
      this.term.clearSelection();
      this.count.textContent = "Invalid pattern";
      this.count.classList.add("none");
      return false;
    }
  }

  /** `incremental` keeps the current match while typing instead of moving on. */
  private search(incremental: boolean) {
    for (const [b, on] of [
      [this.caseButton, this.caseSensitive],
      [this.wordButton, this.wholeWord],
      [this.regexButton, this.regex],
    ] as const) {
      b.classList.toggle("on", on);
      b.setAttribute("aria-pressed", String(on));
    }
    const query = this.input.value;
    if (!query) {
      this.input.classList.remove("invalid");
      this.addon.clearDecorations();
      this.term.clearSelection();
      this.count.textContent = "";
      return;
    }
    if (!this.valid(query)) return;
    this.addon.findNext(query, this.options(incremental));
  }

  private showCount(index: number, total: number) {
    if (!this.input.value) {
      this.count.textContent = "";
    } else if (total === 0) {
      this.count.textContent = "No results";
    } else if (index < 0) {
      // More matches than the highlight limit: the addon stops counting.
      this.count.textContent = total >= HIGHLIGHT_LIMIT ? `${HIGHLIGHT_LIMIT}+ matches` : `${total} matches`;
    } else {
      const shown = total >= HIGHLIGHT_LIMIT ? `${HIGHLIGHT_LIMIT}+` : String(total);
      this.count.textContent = `${index + 1} of ${shown}`;
    }
    this.count.classList.toggle("none", total === 0);
  }

  dispose() {
    this.addon.dispose();
    this.element.remove();
  }
}
