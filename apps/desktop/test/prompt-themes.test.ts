import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => null) }));

const { sanitize, settings, updateSettings } = await import("../src/settings");
const { activeScheme, addSchemes, glyphTestCard, MAX_SCHEMES } = await import("../src/prompt-themes");
const { schemeTheme } = await import("../src/themes");
type ColorSchemeSetting = import("../src/settings").ColorSchemeSetting;

const scheme = (name: string, bg = "#101010"): ColorSchemeSetting => ({
  name,
  foreground: "#eeeeee",
  background: bg,
  cursor: null,
  selection: null,
  ansi: Array.from({ length: 16 }, (_, i) => `#${i.toString(16).padStart(2, "0")}0000`),
});

describe("colour schemes in settings.json", () => {
  it("keeps valid schemes and the chosen one", () => {
    const s = sanitize({ colorSchemes: [scheme("Dracula")], colorScheme: "Dracula" });
    expect(s.colorSchemes).toHaveLength(1);
    expect(s.colorScheme).toBe("Dracula");
  });

  it("drops schemes with anything but plain colours", () => {
    const bad = { ...scheme("Evil"), background: "red;background:url(x)" };
    const short = { ...scheme("Short"), ansi: ["#000000"] };
    const s = sanitize({ colorSchemes: [bad, short, scheme("Ok")], colorScheme: "Evil" });
    expect(s.colorSchemes.map((c) => c.name)).toEqual(["Ok"]);
    expect(s.colorScheme).toBe("");
  });

  it("strips control characters from names and keeps the first of duplicates", () => {
    const s = sanitize({ colorSchemes: [scheme("A\u001b]0;x\u0007"), scheme("A]0;x"), scheme("A]0;x", "#202020")] });
    expect(s.colorSchemes).toHaveLength(1);
    expect(s.colorSchemes[0].name).toBe("A]0;x");
    expect(s.colorSchemes[0].background).toBe("#101010");
  });
});

describe("addSchemes", () => {
  it("replaces by name, keeps at most the limit and returns only kept names", () => {
    updateSettings({ colorSchemes: [scheme("Old"), scheme("Keep")], colorScheme: "" });
    expect(addSchemes([scheme("Keep", "#303030")])).toEqual(["Keep"]);
    expect(settings().colorSchemes.map((c) => c.name)).toEqual(["Old", "Keep"]);
    expect(settings().colorSchemes[1].background).toBe("#303030");

    const many = Array.from({ length: MAX_SCHEMES + 5 }, (_, i) => scheme(`S${i}`));
    const kept = addSchemes(many);
    expect(settings().colorSchemes).toHaveLength(MAX_SCHEMES);
    expect(kept).toHaveLength(MAX_SCHEMES);
    expect(kept[0]).toBe("S5");
    expect(settings().colorSchemes.some((c) => c.name === "Old")).toBe(false);
  });

  it("the active scheme follows the choice", () => {
    updateSettings({ colorSchemes: [scheme("Dracula")], colorScheme: "Dracula" });
    expect(activeScheme()?.name).toBe("Dracula");
    updateSettings({ colorScheme: "" });
    expect(activeScheme()).toBeNull();
  });
});

describe("schemeTheme", () => {
  it("maps the 16 colours and fills cursor and selection", () => {
    const t = schemeTheme(scheme("X"));
    expect(t.black).toBe("#000000");
    expect(t.brightWhite).toBe("#0f0000");
    expect(t.cursor).toBe("#eeeeee");
    expect(t.cursorAccent).toBe("#101010");
    expect(t.selectionBackground).toBe("#eeeeee40");
  });
});

describe("glyphTestCard", () => {
  it("draws Powerline and Nerd Font glyphs and never sends a newline alone", () => {
    const card = glyphTestCard();
    expect(card).toContain("");
    expect(card).toContain("");
    expect(card.replace(/\r\n/g, "")).not.toMatch(/[\r\n]/);
  });
});
