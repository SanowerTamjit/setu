// Keybinding profiles. Every profile keeps the default Ctrl+Shift shortcuts;
// Vim and Emacs add a prefix key followed by one letter, the way tmux and
// Emacs work, so single keys still reach the shell.
//
// Keys are written as chords like "Ctrl+Shift+T" (Cmd counts as Ctrl on
// macOS) and sequences as "Ctrl+X 2".

import type { KeymapName } from "./settings";

export type Bindings = Record<string, string[]>;

const DEFAULT: Bindings = {
  "palette.open": ["Ctrl+Shift+P"],
  "tab.new": ["Ctrl+Shift+T"],
  "ssh.new": ["Ctrl+Shift+O"],
  "pane.close": ["Ctrl+Shift+W"],
  "pane.splitRight": ["Ctrl+Shift+D"],
  "pane.splitDown": ["Ctrl+Alt+Shift+D"],
  "pane.focusLeft": ["Alt+Left"],
  "pane.focusRight": ["Alt+Right"],
  "pane.focusUp": ["Alt+Up"],
  "pane.focusDown": ["Alt+Down"],
  "tab.next": ["Ctrl+Tab"],
  "tab.previous": ["Ctrl+Shift+Tab"],
  "hosts.search": ["Ctrl+Shift+H"],
  "find.open": ["Ctrl+Shift+F"],
  "files.open": ["Ctrl+Shift+E"],
  "tab.detach": ["Ctrl+Shift+N"],
  "settings.open": ["Ctrl+,"],
  "terminal.copy": ["Ctrl+Shift+C"],
  "terminal.paste": ["Ctrl+Shift+V"],
};

/** Vim: Ctrl+Space, then a letter (h j k l move between panes). */
const VIM: Bindings = {
  "palette.open": ["Ctrl+Space :"],
  "tab.new": ["Ctrl+Space T"],
  "ssh.new": ["Ctrl+Space O"],
  "pane.close": ["Ctrl+Space X"],
  "pane.splitRight": ["Ctrl+Space V"],
  "pane.splitDown": ["Ctrl+Space S"],
  "pane.focusLeft": ["Ctrl+Space H"],
  "pane.focusRight": ["Ctrl+Space L"],
  "pane.focusUp": ["Ctrl+Space K"],
  "pane.focusDown": ["Ctrl+Space J"],
  "tab.next": ["Ctrl+Space N"],
  "tab.previous": ["Ctrl+Space P"],
  "find.open": ["Ctrl+Space /"],
  "files.open": ["Ctrl+Space F"],
};

/** Emacs: Ctrl+X, then the key Emacs uses for windows (2, 3, o, 0). */
const EMACS: Bindings = {
  "palette.open": ["Alt+X"],
  "tab.new": ["Ctrl+X T"],
  "ssh.new": ["Ctrl+X Ctrl+F"],
  "pane.close": ["Ctrl+X 0"],
  "pane.splitRight": ["Ctrl+X 3"],
  "pane.splitDown": ["Ctrl+X 2"],
  "pane.focusRight": ["Ctrl+X O"],
  "tab.next": ["Ctrl+X Right"],
  "tab.previous": ["Ctrl+X Left"],
  "tab.close": ["Ctrl+X K"],
  "find.open": ["Ctrl+X S"],
  "files.open": ["Ctrl+X D"],
};

export const KEYMAPS: { id: KeymapName; name: string; description: string; prefix?: string }[] = [
  { id: "default", name: "Default", description: "Ctrl+Shift shortcuts, as in most terminals" },
  { id: "vim", name: "Vim", description: "Ctrl+Space, then h j k l, v, s, x, :, /", prefix: "Ctrl+Space" },
  { id: "emacs", name: "Emacs", description: "Ctrl+X, then 2, 3, o, 0, k; Alt+X for commands", prefix: "Ctrl+X" },
];

/** The bindings of a profile: its own keys plus the defaults. */
export function bindingsFor(name: KeymapName): Bindings {
  const extra = name === "vim" ? VIM : name === "emacs" ? EMACS : {};
  const out: Bindings = {};
  for (const [id, keys] of Object.entries(DEFAULT)) out[id] = [...keys];
  for (const [id, keys] of Object.entries(extra)) out[id] = [...keys, ...(out[id] ?? [])];
  return out;
}

export function prefixOf(name: KeymapName): string | null {
  return KEYMAPS.find((k) => k.id === name)?.prefix ?? null;
}

const CODE_NAMES: Record<string, string> = {
  Space: "Space",
  Tab: "Tab",
  Enter: "Enter",
  Escape: "Escape",
  Backspace: "Backspace",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  ArrowUp: "Up",
  ArrowDown: "Down",
  Comma: ",",
  Period: ".",
  Slash: "/",
  Backslash: "\\",
  Minus: "-",
  Equal: "=",
  Semicolon: ";",
};

/**
 * The chord a key event makes, by physical key so layouts and Shift do not
 * change it ("Ctrl+Shift+T", "Alt+Left"). `;` with Shift is read as `:`.
 * Returns null for a modifier key on its own.
 */
export function chordOf(e: KeyboardEvent): string | null {
  if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return null;
  let key: string | undefined;
  if (e.code.startsWith("Key")) key = e.code.slice(3);
  else if (e.code.startsWith("Digit")) key = e.code.slice(5);
  else if (/^F\d+$/.test(e.code)) key = e.code;
  else key = CODE_NAMES[e.code];
  if (!key) return null;
  let shift = e.shiftKey;
  if (key === ";" && shift) {
    key = ":";
    shift = false;
  }
  const parts: string[] = [];
  if (e.ctrlKey || e.metaKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (shift) parts.push("Shift");
  parts.push(key);
  return parts.join("+");
}

/** Matches key presses against bindings, keeping track of a pending prefix. */
export class KeyDispatcher {
  private pending: string | null = null;
  private timer: number | undefined;
  private table = new Map<string, string>();
  private prefixes = new Set<string>();

  constructor(private readonly onPending: (prefix: string | null) => void) {}

  setBindings(bindings: Bindings) {
    this.table.clear();
    this.prefixes.clear();
    for (const [id, keys] of Object.entries(bindings)) {
      for (const k of keys) {
        this.table.set(k, id);
        const parts = k.split(" ");
        if (parts.length > 1) this.prefixes.add(parts[0]);
      }
    }
    this.setPending(null);
  }

  /**
   * What to do with a key press: run a command, swallow it (a prefix, or an
   * unknown key after one), pass the prefix itself through to the shell
   * (pressed twice), or leave it alone.
   */
  handle(e: KeyboardEvent, allowPrefix = true): { command: string } | { swallow: true } | { passPrefix: string } | null {
    const chord = chordOf(e);
    if (!chord) return null;
    if (!allowPrefix) {
      this.setPending(null);
      const id = this.table.get(chord);
      return id ? { command: id } : null;
    }
    if (this.pending) {
      const prefix = this.pending;
      this.setPending(null);
      if (chord === prefix) return { passPrefix: prefix };
      const id = this.table.get(`${prefix} ${chord}`) ?? this.table.get(`${prefix} ${chord.replace(/^Shift\+/, "")}`);
      return id ? { command: id } : { swallow: true };
    }
    if (this.prefixes.has(chord)) {
      this.setPending(chord);
      return { swallow: true };
    }
    const id = this.table.get(chord);
    return id ? { command: id } : null;
  }

  private setPending(prefix: string | null) {
    this.pending = prefix;
    window.clearTimeout(this.timer);
    if (prefix) this.timer = window.setTimeout(() => this.setPending(null), 2000);
    this.onPending(prefix);
  }
}

/** The control bytes a prefix sends to a shell when pressed twice. */
export function prefixBytes(prefix: string): string {
  if (prefix === "Ctrl+X") return "\x18";
  if (prefix === "Ctrl+Space") return "\x00";
  return "";
}
