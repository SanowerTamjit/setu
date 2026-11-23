// Work memory in the terminal: notices each command line you run and
// hands it to Rust, which keeps it only when remembering is on (masking
// secrets). See docs/work-memory.md.
//
// We read the line from the terminal itself when you press Enter (so tab
// completion and history recall are seen as the shell shows them), minus
// the prompt that was there before you started typing. Lines are skipped:
// - in full-screen programs (vim, less, top: the alternate screen);
// - at a prompt asking for a password, passphrase, PIN, token or code;
// - when what you typed was not echoed (a password prompt we missed);
// - when the line starts with a space (as shells skip it in history).

import type { Terminal } from "@xterm/xterm";
import { registerProvider } from "./commands";
import { enabled } from "./features";
import { memoryRecord, memoryView, type MemorySnippet } from "./ipc";

/** Prompts that ask for something secret: nothing typed there is kept. */
const SECRET_PROMPT = /(pass(word|phrase|code)?|\bpin\b|token|secret|otp|one-time|verification code|2fa|mfa)\b[^\n]*[:?>]\s*$/i;

let recording = false;
let snippets: MemorySnippet[] = [];
const listeners = new Set<() => void>();

/** Reloads whether remembering is on, and the saved snippets. */
export async function refreshMemoryState() {
  try {
    const v = await memoryView();
    recording = v.settings.recording;
    snippets = v.snippets;
  } catch {
    recording = false;
  }
  for (const l of listeners) l();
}

export function memoryRecording() {
  return enabled("workMemory") && recording;
}

export function onMemoryChange(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Saved snippets go in the palette; picking one types it without Enter. */
export function registerMemorySnippets(insert: (command: string) => void) {
  registerProvider(() =>
    enabled("workMemory")
      ? snippets.map((s) => ({
          id: `snippet:memory:${s.id}`,
          title: s.title,
          category: "Snippet",
          keywords: `${s.command} my snippets`,
          run: () => insert(s.command),
        }))
      : [],
  );
}

export function isSecretPrompt(prompt: string) {
  return SECRET_PROMPT.test(prompt);
}

/** A row's text; the prompt keeps its trailing space (`$ `) so it lines up. */
function lineText(term: Terminal, y: number, end?: number) {
  return term.buffer.active.getLine(y)?.translateToString(end === undefined, 0, end) ?? "";
}

/** One per terminal pane. */
export class CommandCapture {
  private readonly session = `p${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
  /** What was typed since the last Enter, and whether editing keys were used. */
  private typed = "";
  private edited = false;
  private active = false;
  private promptY = 0;
  private prompt = "";

  constructor(
    private readonly term: Terminal,
    /** The saved host's name for SSH panes, or null for local shells. */
    private readonly host: () => string | null,
  ) {}

  /** Call with each chunk the terminal sends to the shell, before sending. */
  input(data: string) {
    if (!memoryRecording()) {
      this.reset();
      return;
    }
    if (this.term.buffer.active.type === "alternate") {
      this.reset();
      return;
    }
    let i = 0;
    while (i < data.length) {
      const c = data[i];
      if (!this.active) this.begin();
      if (c === "\r" || c === "\n") {
        this.finish();
      } else if (c === "\x7f" || c === "\b") {
        this.typed = this.typed.slice(0, -1);
      } else if (c === "\x03" || c === "\x04" || c === "\x15") {
        // Ctrl+C, Ctrl+D, Ctrl+U: the line is gone.
        this.reset();
      } else if (c === "\x1b") {
        // Arrow keys, history, bracketed paste: read the line from the screen.
        this.edited = true;
        const m = /^\x1b(\[[0-9;?]*[ -/]*[@-~]|O.|.)/.exec(data.slice(i));
        i += m ? m[0].length : 1;
        continue;
      } else if (c === "\t" || c < " ") {
        this.edited = true;
      } else {
        this.typed += c;
      }
      i++;
    }
  }

  private begin() {
    const b = this.term.buffer.active;
    this.active = true;
    this.typed = "";
    this.edited = false;
    this.promptY = b.baseY + b.cursorY;
    this.prompt = lineText(this.term, this.promptY, b.cursorX);
  }

  private reset() {
    this.active = false;
    this.typed = "";
    this.edited = false;
  }

  /** The line as shown now, from the prompt's row through wrapped rows. */
  private shown(): string | null {
    const b = this.term.buffer.active;
    // A row that wraps on is read whole, so a space at the wrap is kept.
    let text = "";
    let y = this.promptY;
    for (; b.getLine(y + 1)?.isWrapped; y++) text += b.getLine(y)?.translateToString(false) ?? "";
    text += lineText(this.term, y);
    return text.startsWith(this.prompt) ? text.slice(this.prompt.length) : null;
  }

  private finish() {
    const typed = this.typed;
    const edited = this.edited;
    const prompt = this.prompt;
    const shown = this.shown();
    this.reset();
    if (isSecretPrompt(prompt)) return;
    let command: string | null;
    if (edited) {
      command = shown;
    } else {
      // Typed but nothing echoed: a password prompt, most likely.
      if (typed.trim() && !(shown ?? "").trim()) return;
      command = typed;
    }
    if (!command || !command.trim()) return;
    void memoryRecord(command, this.host(), this.session).catch(() => undefined);
  }
}
