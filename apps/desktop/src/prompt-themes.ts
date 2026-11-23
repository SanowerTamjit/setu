// Support for third-party prompt themes (Powerlevel10k, Oh My Zsh, Starship,
// Oh My Posh) and terminal looks from other apps:
//
// - a built-in Nerd Font symbols font, used only for icon code points the
//   chosen font lacks, so prompt icons and Powerline arrows show with any font;
// - Unicode 11 character widths (set in terminal-tab.ts), the widths zsh and
//   bash use, so right-hand prompts and icons line up;
// - colour schemes imported from other terminals (parsed in Rust,
//   color_schemes.rs), chosen in Settings → Prompts.
//
// All of it sits behind the `promptThemes` feature flag.

import { enabled } from "./features";
import { settings, updateSettings, type ColorSchemeSetting, type Settings } from "./settings";
import { terminalTheme } from "./themes";
import type { ITheme } from "@xterm/xterm";

/** The family name styles.css gives the bundled Symbols Nerd Font Mono. */
export const SYMBOLS_FAMILY = "Setu Symbols";

export function promptThemesOn(): boolean {
  return enabled("promptThemes");
}

export const MAX_SCHEMES = 50;

/**
 * Adds imported schemes, replacing ones with the same name and dropping the
 * oldest past the limit. Returns the names of the imported schemes that were
 * kept, in file order.
 */
export function addSchemes(schemes: ColorSchemeSetting[]): string[] {
  const byName = new Map(settings().colorSchemes.map((s) => [s.name, s]));
  for (const s of schemes) {
    // Re-adding moves a replaced scheme to the end, so it counts as newest.
    byName.delete(s.name);
    byName.set(s.name, s);
  }
  const all = [...byName.values()].slice(-MAX_SCHEMES);
  updateSettings({ colorSchemes: all });
  const kept = new Set(all.map((s) => s.name));
  return [...new Set(schemes.map((s) => s.name))].filter((n) => kept.has(n));
}

/** The imported scheme terminals use now, if any. */
export function activeScheme(s: Readonly<Settings> = settings()): ColorSchemeSetting | null {
  if (!promptThemesOn() || !s.colorScheme) return null;
  return s.colorSchemes.find((c) => c.name === s.colorScheme) ?? null;
}

/** The terminal colours for the current settings. */
export function currentTerminalTheme(s: Readonly<Settings> = settings()): ITheme {
  return terminalTheme(s.theme, s.layout, activeScheme(s));
}

/**
 * Loads the symbols font before the first terminal draws. The GPU renderer
 * caches each glyph the first time it draws it, so a glyph drawn before the
 * font arrived would stay a box.
 */
export async function preloadSymbols(): Promise<void> {
  if (!promptThemesOn() || !document.fonts) return;
  const timeout = new Promise<void>((resolve) => setTimeout(resolve, 1500));
  await Promise.race([document.fonts.load(`14px "${SYMBOLS_FAMILY}"`, "").then(() => undefined), timeout]).catch(() => {});
}

const ESC = "\x1b[";
const fg = (r: number, g: number, b: number) => `${ESC}38;2;${r};${g};${b}m`;
const bg = (r: number, g: number, b: number) => `${ESC}48;2;${r};${g};${b}m`;

/**
 * A test card written straight to a terminal (not to the shell): a
 * Powerline-style prompt, Nerd Font icons, a true-colour ramp, wide
 * characters and the 16 ANSI colours. If it looks right, Powerlevel10k,
 * Starship and Oh My Posh will too.
 */
export function glyphTestCard(): string {
  const reset = `${ESC}0m`;
  const lines: string[] = [];
  lines.push("", `${ESC}1mSetu prompt check${reset}  (only shown here, nothing was sent to the shell)`, "");
  // A two-sided Powerline prompt, like Powerlevel10k's "lean/rainbow" styles.
  const seg = (b: [number, number, number], f: [number, number, number], text: string, next: [number, number, number] | null) =>
    `${bg(...b)}${fg(...f)} ${text} ${next ? bg(...next) : `${ESC}49m`}${fg(...b)}`;
  const os: [number, number, number] = [228, 228, 228];
  const dir: [number, number, number] = [36, 99, 235];
  const git: [number, number, number] = [95, 168, 72];
  lines.push(
    "  " +
      seg(os, [30, 30, 30], "", dir) +
      seg(dir, [255, 255, 255], " ~/src/setu", git) +
      seg(git, [20, 20, 20], " main ", null) +
      reset +
      "        " +
      `${fg(214, 158, 46)}${bg(214, 158, 46)}${fg(20, 20, 20)}  2s ${reset}`,
  );
  lines.push(`  ${fg(95, 168, 72)}❯${reset} `, "");
  lines.push(
    "  Powerline          ",
    "  Icons                  \u{f0219} \u{f0c8b}",
    "  Status            ",
  );
  let ramp = "  True colour ";
  for (let i = 0; i < 48; i++) {
    const t = i / 47;
    ramp += `${bg(Math.round(255 * (1 - t)), Math.round(120 + 100 * Math.sin(Math.PI * t)), Math.round(255 * t))} `;
  }
  lines.push(ramp + reset);
  let ansi = "  ANSI       ";
  for (let i = 0; i < 16; i++) ansi += `${ESC}${i < 8 ? 40 + i : 100 + i - 8}m  `;
  lines.push(ansi + reset);
  lines.push("  Wide       |界|😀|✔|  the bars should line up with the row below", "             |ab|cd|e|", "");
  return lines.join("\r\n");
}

export interface PromptToolInfo {
  id: "powerlevel10k" | "ohmyzsh" | "starship" | "ohmyposh";
  name: string;
  shells: string;
  site: string;
  /** Lines to put in a start-up file; copied, never run by Setu. */
  setup: { label: string; text: string }[];
}

export const PROMPT_TOOLS: PromptToolInfo[] = [
  {
    id: "powerlevel10k",
    name: "Powerlevel10k",
    shells: "zsh",
    site: "github.com/romkatv/powerlevel10k",
    setup: [
      { label: "Install", text: "git clone --depth=1 https://github.com/romkatv/powerlevel10k.git ~/powerlevel10k" },
      { label: "Add to ~/.zshrc", text: "source ~/powerlevel10k/powerlevel10k.zsh-theme" },
      { label: "Then, in a new tab", text: "p10k configure" },
    ],
  },
  {
    id: "ohmyzsh",
    name: "Oh My Zsh",
    shells: "zsh",
    site: "ohmyz.sh",
    setup: [{ label: "With Powerlevel10k, in ~/.zshrc", text: 'ZSH_THEME="powerlevel10k/powerlevel10k"' }],
  },
  {
    id: "starship",
    name: "Starship",
    shells: "bash, zsh, fish, PowerShell",
    site: "starship.rs",
    setup: [
      { label: "zsh (~/.zshrc)", text: 'eval "$(starship init zsh)"' },
      { label: "bash (~/.bashrc)", text: 'eval "$(starship init bash)"' },
      { label: "PowerShell ($PROFILE)", text: "Invoke-Expression (&starship init powershell)" },
    ],
  },
  {
    id: "ohmyposh",
    name: "Oh My Posh",
    shells: "PowerShell, bash, zsh, fish",
    site: "ohmyposh.dev",
    setup: [
      { label: "PowerShell ($PROFILE)", text: "oh-my-posh init pwsh | Invoke-Expression" },
      { label: "zsh (~/.zshrc)", text: 'eval "$(oh-my-posh init zsh)"' },
    ],
  },
];
