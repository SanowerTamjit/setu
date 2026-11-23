// Colour themes: the app's CSS tokens (styles.css, by `data-theme`) and the
// matching terminal palettes here.

import type { ITheme } from "@xterm/xterm";
import type { ColorSchemeSetting, LayoutName, ThemeName } from "./settings";

export const THEMES: { id: ThemeName; name: string; description: string }[] = [
  { id: "dark", name: "Dark", description: "The default: graphite with a blue accent, easy on the eyes" },
  { id: "light", name: "Light", description: "Clean white surfaces, for bright rooms" },
  { id: "high-contrast", name: "High contrast", description: "Black and white with strong focus outlines" },
  { id: "ocean", name: "Ocean", description: "Deep blue-green with a teal accent, calm for long night shifts" },
  { id: "paper", name: "Paper", description: "Warm off-white with an amber accent, soft on the eyes by day" },

  { id: "ember", name: "Ember", description: "Warm charcoal with an amber glow, for late nights" },
  { id: "sandstone", name: "Sandstone", description: "Soft paper tones with a teal accent, gentle in daylight" },
];

const DARK: ITheme = {
  background: "#111214",
  foreground: "#e3e5e8",
  cursor: "#4c8dff",
  cursorAccent: "#111214",
  selectionBackground: "#2b3a55",
  black: "#1c2129",
  red: "#e06c75",
  green: "#98c379",
  yellow: "#e5c07b",
  blue: "#61afef",
  magenta: "#c678dd",
  cyan: "#56b6c2",
  white: "#d8dee9",
  brightBlack: "#5c6370",
  brightRed: "#ff7a85",
  brightGreen: "#b5e890",
  brightYellow: "#ffd68a",
  brightBlue: "#82c4ff",
  brightMagenta: "#de9bf0",
  brightCyan: "#7fd6e0",
  brightWhite: "#ffffff",
};

const LIGHT: ITheme = {
  background: "#f7f8fa",
  foreground: "#1b1f27",
  cursor: "#2563eb",
  cursorAccent: "#f7f8fa",
  selectionBackground: "#cfd8e8",
  black: "#1f2430",
  red: "#c4323f",
  green: "#2f7d32",
  yellow: "#8a5a00",
  blue: "#1f6fbf",
  magenta: "#8e3fae",
  cyan: "#127a87",
  white: "#d5d9e0",
  brightBlack: "#5d6575",
  brightRed: "#d9434f",
  brightGreen: "#3a9a3e",
  brightYellow: "#a36d00",
  brightBlue: "#2a83dc",
  brightMagenta: "#a552c6",
  brightCyan: "#1793a3",
  brightWhite: "#ffffff",
};

const HIGH_CONTRAST: ITheme = {
  background: "#000000",
  foreground: "#ffffff",
  cursor: "#ffd400",
  cursorAccent: "#000000",
  selectionBackground: "#ffd400",
  selectionForeground: "#000000",
  black: "#000000",
  red: "#ff6b6b",
  green: "#6dff6d",
  yellow: "#ffd400",
  blue: "#66c2ff",
  magenta: "#f0a0ff",
  cyan: "#55ffff",
  white: "#ffffff",
  brightBlack: "#bfbfbf",
  brightRed: "#ff9999",
  brightGreen: "#a6ffa6",
  brightYellow: "#ffe680",
  brightBlue: "#b3e0ff",
  brightMagenta: "#f7c8ff",
  brightCyan: "#aaffff",
  brightWhite: "#ffffff",
};

const OCEAN: ITheme = {
  background: "#0c1821",
  foreground: "#dbe7ee",
  cursor: "#2ec4b6",
  cursorAccent: "#0c1821",
  selectionBackground: "#1f4052",
  black: "#13232f",
  red: "#ef6f6c",
  green: "#7ed492",
  yellow: "#f2c66d",
  blue: "#5cb3f0",
  magenta: "#c792ea",
  cyan: "#4fd1d9",
  white: "#c9d6df",
  brightBlack: "#5a7587",
  brightRed: "#ff9592",
  brightGreen: "#a6e8b3",
  brightYellow: "#ffd98f",
  brightBlue: "#8ccbf7",
  brightMagenta: "#ddb6ff",
  brightCyan: "#80e6ec",
  brightWhite: "#ffffff",
};

const PAPER: ITheme = {
  background: "#f6f1e7",
  foreground: "#2b2620",
  cursor: "#b45309",
  cursorAccent: "#f6f1e7",
  selectionBackground: "#e6d8bd",
  black: "#2b2620",
  red: "#b42318",
  green: "#3f7a2e",
  yellow: "#8a5a00",
  blue: "#2b6a9e",
  magenta: "#8e3fa0",
  cyan: "#1f7a78",
  white: "#d9cfbd",
  brightBlack: "#6b6152",
  brightRed: "#cf3a2c",
  brightGreen: "#4d933a",
  brightYellow: "#a66d00",
  brightBlue: "#3a7fb8",
  brightMagenta: "#a552b8",
  brightCyan: "#27918e",
  brightWhite: "#fffaf0",
};

const EMBER: ITheme = {
  background: "#16120f",
  foreground: "#efe6dc",
  cursor: "#f0883e",
  cursorAccent: "#16120f",
  selectionBackground: "#4a3424",
  black: "#2a221c",
  red: "#e8705f",
  green: "#a3c06e",
  yellow: "#e8b44a",
  blue: "#7fa8d4",
  magenta: "#c98bb8",
  cyan: "#6fbfb0",
  white: "#ddd2c4",
  brightBlack: "#857565",
  brightRed: "#ff9283",
  brightGreen: "#c3dc8f",
  brightYellow: "#f8d27a",
  brightBlue: "#a3c4e8",
  brightMagenta: "#e3aad3",
  brightCyan: "#94dccd",
  brightWhite: "#fff8ef",
};

const SANDSTONE: ITheme = {
  background: "#f5f0e6",
  foreground: "#2b2620",
  cursor: "#0f766e",
  cursorAccent: "#f5f0e6",
  selectionBackground: "#cfe0d8",
  black: "#2b2620",
  red: "#b3372b",
  green: "#3f7a2a",
  yellow: "#8a5a00",
  blue: "#2a6496",
  magenta: "#8a3f8a",
  cyan: "#0e7490",
  white: "#d8cfbf",
  brightBlack: "#6b6152",
  brightRed: "#c84a3d",
  brightGreen: "#4d8f36",
  brightYellow: "#a06a00",
  brightBlue: "#3678b0",
  brightMagenta: "#9e52a0",
  brightCyan: "#1589a8",
  brightWhite: "#fffdf8",
};

const PALETTES: Record<ThemeName, ITheme> = {
  dark: DARK,
  light: LIGHT,
  "high-contrast": HIGH_CONTRAST,
  ocean: OCEAN,
  paper: PAPER,

  ember: EMBER,
  sandstone: SANDSTONE,
};

/** Studio's terminal sits on a deeper, bluer background to match its panels. */
const STUDIO_BACKGROUND: Partial<Record<ThemeName, string>> = {
  dark: "#0a0d14",
  light: "#fbfcff",
  ocean: "#0a151d",
  paper: "#fbf6ec",

  ember: "#120e0b",
  sandstone: "#fbf7ef",
};

const ANSI_KEYS = [
  "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
  "brightBlack", "brightRed", "brightGreen", "brightYellow", "brightBlue", "brightMagenta", "brightCyan", "brightWhite",
] as const;

/** An imported colour scheme as an xterm theme. */
export function schemeTheme(scheme: ColorSchemeSetting): ITheme {
  const theme: ITheme = {
    background: scheme.background,
    foreground: scheme.foreground,
    cursor: scheme.cursor ?? scheme.foreground,
    cursorAccent: scheme.background,
    // A see-through selection keeps text readable when a scheme has none.
    selectionBackground: scheme.selection ?? `${scheme.foreground}40`,
  };
  ANSI_KEYS.forEach((k, i) => (theme[k] = scheme.ansi[i]));
  return theme;
}

/**
 * The terminal colours: an imported scheme when one is chosen, otherwise the
 * app theme's palette.
 */
export function terminalTheme(name: ThemeName, layout: LayoutName = "classic", scheme?: ColorSchemeSetting | null): ITheme {
  if (scheme) return schemeTheme(scheme);
  const base = PALETTES[name] ?? DARK;
  const background = layout === "studio" ? STUDIO_BACKGROUND[name] : undefined;
  return background ? { ...base, background, cursorAccent: background } : base;
}

/** Active match and other matches, per theme. */
const FIND_COLORS: Record<ThemeName, [string, string]> = {
  dark: ["#4c8dff", "#2b3a55"],
  light: ["#2563eb", "#cddcf7"],
  "high-contrast": ["#ffd400", "#444444"],
  ocean: ["#2ec4b6", "#1f4052"],
  paper: ["#b45309", "#ecdcbc"],
  ember: ["#f0883e", "#4a3424"],
  sandstone: ["#0f766e", "#cfe0d8"],
};

/** Match highlights for find, readable on each theme. */
export function findColors(name: ThemeName) {
  const [active, match] = FIND_COLORS[name] ?? FIND_COLORS.dark;
  return {
    matchBackground: match,
    matchBorder: name === "high-contrast" ? "#ffffff" : "#7d8799",
    matchOverviewRuler: "#7d8799",
    activeMatchBackground: active,
    activeMatchBorder: active,
    activeMatchColorOverviewRuler: active,
  };
}

/** Applies the app colours; terminals follow through `onSettingsChange`. */
export function applyTheme(name: ThemeName) {
  document.documentElement.dataset.theme = name;
}
