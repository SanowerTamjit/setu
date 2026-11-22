// Every action Setu can run from the keyboard or the command palette.
// Shortcuts are not stored here: they come from the active keymap.

export interface Command {
  /** Stable id, used by keymaps: `tab.new`, `pane.splitRight`, … */
  id: string;
  title: string;
  /** Shown before the title in the palette, such as "Connect" or "Theme". */
  category?: string;
  /** Extra words the palette matches on. */
  keywords?: string;
  /** Hidden from the palette and ignored by keys when this returns false. */
  when?: () => boolean;
  run: () => void | Promise<void>;
}

const commands = new Map<string, Command>();
const providers: (() => Command[])[] = [];

export function registerCommand(command: Command) {
  commands.set(command.id, command);
}

/** Adds commands that change over time (saved hosts, shells, snippets). */
export function registerProvider(provider: () => Command[]) {
  providers.push(provider);
}

export function command(id: string): Command | undefined {
  return commands.get(id);
}

/** All commands available now, fixed ones first. */
export function allCommands(): Command[] {
  const fixed = [...commands.values()];
  const dynamic = providers.flatMap((p) => {
    try {
      return p();
    } catch {
      return [];
    }
  });
  return [...fixed, ...dynamic].filter((c) => !c.when || c.when());
}

export async function runCommand(id: string): Promise<boolean> {
  const c = commands.get(id);
  if (!c || (c.when && !c.when())) return false;
  await c.run();
  return true;
}

/**
 * Scores `text` against `query` (case-insensitive, letters in order, gaps
 * allowed); higher is better, null when it does not match. Word starts and
 * runs of consecutive letters score more, as in most editors' palettes.
 */
export function fuzzyScore(query: string, text: string): number | null {
  const q = query.toLowerCase().replace(/\s+/g, "");
  if (!q) return 0;
  const t = text.toLowerCase();
  let score = 0;
  let ti = 0;
  let run = 0;
  for (const ch of q) {
    const found = t.indexOf(ch, ti);
    if (found < 0) return null;
    const wordStart = found === 0 || /[\s:/._@-]/.test(t[found - 1]);
    run = found === ti ? run + 1 : 1;
    score += 1 + (wordStart ? 3 : 0) + (run > 1 ? 2 : 0) - Math.min(found - ti, 3) * 0.5;
    ti = found + 1;
  }
  // Shorter texts with the same letters are better matches.
  return score - t.length * 0.01;
}
