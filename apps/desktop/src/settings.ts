// Preferences, saved as settings.json in Setu's config folder. Everything has
// a default, so a missing or older file just means defaults.

import { invoke } from "@tauri-apps/api/core";

export type ThemeName = "dark" | "light" | "high-contrast" | "ember" | "sandstone";
const THEME_NAMES: readonly ThemeName[] = ["dark", "light", "high-contrast", "ember", "sandstone"];
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
  features: {},
};

let current: Settings = structuredClone(DEFAULTS);
const listeners = new Set<(s: Settings, changed: Partial<Settings>) => void>();

function sanitize(raw: unknown): Settings {
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
