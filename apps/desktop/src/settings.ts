// Preferences, saved as settings.json in Setu's config folder. Everything has
// a default, so a missing or older file just means defaults.

import { invoke } from "@tauri-apps/api/core";

export type ThemeName = "dark" | "light" | "high-contrast" | "ocean" | "paper" | "ember" | "sandstone";
const THEME_NAMES: readonly ThemeName[] = ["dark", "light", "high-contrast", "ocean", "paper", "ember", "sandstone"];
export type KeymapName = "default" | "vim" | "emacs";
/** How the window is laid out: the rail and tab bar, or the Studio navigator. */
export type LayoutName = "classic" | "studio";

/** A shell profile the user defined: a base shell plus its own settings. */
export interface ShellProfileSetting {
  /** Stable id, `custom:<n>`. */
  id: string;
  name: string;
  /** Id of a detected shell (`bash`, `wsl:Ubuntu`, …) to start from. */
  base: string;
  args: string[];
  /** Starting folder; empty means your home folder. */
  cwd: string;
  /** Extra environment variables, `NAME` → value. */
  env: Record<string, string>;
}

/** A terminal colour scheme imported from another terminal (see prompt-themes.ts). */
export interface ColorSchemeSetting {
  name: string;
  foreground: string;
  background: string;
  cursor: string | null;
  selection: string | null;
  /** The 16 ANSI colours: black … white, then bright black … bright white. */
  ansi: string[];
}

export interface Settings {
  version: 1;
  theme: ThemeName;
  layout: LayoutName;
  keymap: KeymapName;
  /** Hide things that look like secrets in terminal output. */
  maskSecrets: boolean;
  shellProfiles: ShellProfileSetting[];
  /** Shell or profile id for new tabs; null means the system's default shell. */
  defaultShell: string | null;
  /** The role chosen on first launch ("none" when skipped), or null before choosing. */
  role: string | null;
  /** Packs turned on or off by hand; others follow the default (see packs.ts). */
  packs: Record<string, boolean>;
  fontSize: number;
  /** Terminal font; empty uses Setu's list of common coding fonts. */
  fontFamily: string;
  /** Join character sequences like -> and != when the font has ligatures. */
  ligatures: boolean;
  /** `auto` uses the GPU everywhere except Linux, where WebKitGTK lags a frame. */
  renderer: RendererName;
  /** Command that opens remote files for editing; empty uses the system's default app. */
  editor: string;
  /** Colour schemes imported from other terminals (at most 50). */
  colorSchemes: ColorSchemeSetting[];
  /** Name of the imported scheme terminals use; empty follows the app theme. */
  colorScheme: string;
  /** Feature flags turned on or off by hand; others use their default (see features.ts). */
  features: Record<string, boolean>;
}

export type RendererName = "auto" | "gpu" | "dom";

export const DEFAULTS: Settings = {
  version: 1,
  theme: "dark",
  layout: "classic",
  keymap: "default",
  maskSecrets: true,
  shellProfiles: [],
  defaultShell: null,
  role: null,
  packs: {},
  fontSize: 14,
  fontFamily: "",
  ligatures: true,
  renderer: "auto",
  editor: "",
  colorSchemes: [],
  colorScheme: "",
  features: {},
};

const HEX = /^#[0-9a-f]{6}$/i;

/** A scheme from settings.json, or null if anything in it is not a plain colour. */
function sanitizeScheme(raw: unknown): ColorSchemeSetting | null {
  if (!raw || typeof raw !== "object") return null;
  const q = raw as Record<string, unknown>;
  const color = (v: unknown) => (typeof v === "string" && HEX.test(v) ? v.toLowerCase() : null);
  const name = typeof q.name === "string" ? q.name.replace(/[\u0000-\u001f\u007f]/g, "").trim().slice(0, 60) : "";
  const ansi = Array.isArray(q.ansi) ? q.ansi.map(color) : [];
  const foreground = color(q.foreground);
  const background = color(q.background);
  if (!name || !foreground || !background || ansi.length !== 16 || ansi.some((c) => c === null)) return null;
  return { name, foreground, background, cursor: color(q.cursor), selection: color(q.selection), ansi: ansi as string[] };
}

let current: Settings = structuredClone(DEFAULTS);
const listeners = new Set<(s: Settings, changed: Partial<Settings>) => void>();

export function sanitize(raw: unknown): Settings {
  const s = structuredClone(DEFAULTS);
  if (!raw || typeof raw !== "object") return s;
  const r = raw as Record<string, unknown>;
  if (THEME_NAMES.includes(r.theme as ThemeName)) s.theme = r.theme as ThemeName;
  if (r.layout === "classic" || r.layout === "studio") s.layout = r.layout;
  if (r.keymap === "default" || r.keymap === "vim" || r.keymap === "emacs") s.keymap = r.keymap;
  if (typeof r.maskSecrets === "boolean") s.maskSecrets = r.maskSecrets;
  if (typeof r.defaultShell === "string") s.defaultShell = r.defaultShell;
  if (r.packs && typeof r.packs === "object") {
    for (const [k, v] of Object.entries(r.packs as Record<string, unknown>)) {
      if (typeof v === "boolean") s.packs[k] = v;
    }
  }
  if (r.features && typeof r.features === "object") {
    for (const [k, v] of Object.entries(r.features as Record<string, unknown>).slice(0, 100)) {
      if (typeof v === "boolean" && /^[A-Za-z][A-Za-z0-9]{0,40}$/.test(k)) s.features[k] = v;
    }
  }
  if (typeof r.role === "string" || r.role === null) s.role = r.role as string | null;
  if (typeof r.fontSize === "number" && r.fontSize >= 8 && r.fontSize <= 32) s.fontSize = r.fontSize;
  if (typeof r.fontFamily === "string" && r.fontFamily.length <= 200) s.fontFamily = r.fontFamily.trim();
  if (typeof r.ligatures === "boolean") s.ligatures = r.ligatures;
  if (typeof r.editor === "string" && r.editor.length <= 500) s.editor = r.editor.trim();
  if (Array.isArray(r.colorSchemes)) {
    const seen = new Set<string>();
    for (const raw of r.colorSchemes.slice(0, 50)) {
      const scheme = sanitizeScheme(raw);
      if (scheme && !seen.has(scheme.name)) {
        seen.add(scheme.name);
        s.colorSchemes.push(scheme);
      }
    }
  }
  if (typeof r.colorScheme === "string" && s.colorSchemes.some((c) => c.name === r.colorScheme)) s.colorScheme = r.colorScheme;
  if (r.renderer === "auto" || r.renderer === "gpu" || r.renderer === "dom") s.renderer = r.renderer;
  if (Array.isArray(r.shellProfiles)) {
    s.shellProfiles = r.shellProfiles.flatMap((p): ShellProfileSetting[] => {
      if (!p || typeof p !== "object") return [];
      const q = p as Record<string, unknown>;
      if (typeof q.id !== "string" || typeof q.name !== "string" || typeof q.base !== "string") return [];
      const env: Record<string, string> = {};
      if (q.env && typeof q.env === "object") {
        for (const [k, v] of Object.entries(q.env as Record<string, unknown>)) {
          if (typeof v === "string") env[k] = v;
        }
      }
      return [
        {
          id: q.id,
          name: q.name,
          base: q.base,
          args: Array.isArray(q.args) ? q.args.filter((a): a is string => typeof a === "string") : [],
          cwd: typeof q.cwd === "string" ? q.cwd : "",
          env,
        },
      ];
    });
  }
  return s;
}

export async function loadSettings(): Promise<Settings> {
  try {
    const text = await invoke<string | null>("settings_load");
    current = sanitize(text ? JSON.parse(text) : null);
  } catch {
    current = structuredClone(DEFAULTS);
  }
  return current;
}

export function settings(): Readonly<Settings> {
  return current;
}

/** Changes some settings, saves them, and tells listeners. */
export function updateSettings(patch: Partial<Settings>) {
  current = sanitize({ ...current, ...patch });
  void invoke("settings_save", { json: JSON.stringify(current, null, 2) }).catch(() => {
    // Preferences still apply for this run.
  });
  for (const l of listeners) l(current, patch);
}

export function onSettingsChange(listener: (s: Settings, changed: Partial<Settings>) => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
