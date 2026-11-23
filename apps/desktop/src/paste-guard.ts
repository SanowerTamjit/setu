// Ask before pasting several lines into a shell that would run them at once.
//
// With bracketed paste (bash 5.1+, zsh, fish, PowerShell 7 by default), the
// shell receives pasted text as one block and waits for Enter. Without it,
// every line break is an Enter, so pasting text copied from a web page can
// run commands the person never saw (the page can hide them from view).

import type { Terminal } from "@xterm/xterm";
import { confirmDialog, el } from "./dom";
import { enabled } from "./features";

/** Most lines shown in the preview; the rest are counted. */
const PREVIEW_LINES = 12;

/** Lines that would each run as a command (a trailing line break counts). */
export function runningLines(text: string): number {
  return (text.match(/\r\n|\r|\n/g) ?? []).length;
}

/** Whether pasting `text` now would run lines unseen, so it needs asking. */
export function needsPasteCheck(term: Terminal, text: string): boolean {
  return enabled("pasteGuard") && !term.modes.bracketedPasteMode && runningLines(text) > 0;
}

/** Resolves true when the text may be pasted now. */
export async function pasteAllowed(term: Terminal, text: string): Promise<boolean> {
  if (!needsPasteCheck(term, text)) return true;
  const lines = runningLines(text);
  const all = text.split(/\r\n|\r|\n/);
  const shown = all.slice(0, PREVIEW_LINES).join("\n");
  const more = all.length > PREVIEW_LINES ? `\n… and ${all.length - PREVIEW_LINES} more lines` : "";
  const body = el(
    "div",
    {},
    el(
      "p",
      {},
      `This shell runs each line as soon as it is pasted, so ${lines === 1 ? "1 command" : `${lines} commands`} would run without Enter. Check the text first:`,
    ),
    el("pre", { className: "paste-preview" }, shown + more),
  );
  return confirmDialog("Paste and run these lines?", body, "Paste anyway", true, true);
}
