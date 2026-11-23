import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => null) }));

const { DEFAULTS, sanitize } = await import("../src/settings");
const { LAYOUTS } = await import("../src/layouts");
const { terminalTheme } = await import("../src/themes");

describe("layout setting", () => {
  it("defaults to Classic so nobody's window changes on upgrade", () => {
    expect(DEFAULTS.layout).toBe("classic");
    expect(sanitize({}).layout).toBe("classic");
  });

  it("keeps Studio and refuses anything else", () => {
    expect(sanitize({ layout: "studio" }).layout).toBe("studio");
    for (const bad of ["futuristic", "", 1, null, ["studio"]]) {
      expect(sanitize({ layout: bad }).layout).toBe("classic");
    }
  });

  it("lists exactly the layouts the setting accepts", () => {
    expect(LAYOUTS.map((l) => l.id)).toEqual(["classic", "studio"]);
    for (const l of LAYOUTS) expect(sanitize({ layout: l.id }).layout).toBe(l.id);
  });
});

describe("terminal colours per layout", () => {
  it("leaves Classic's terminal colours as they were", () => {
    expect(terminalTheme("dark")).toEqual(terminalTheme("dark", "classic"));
    expect(terminalTheme("dark", "classic").background).toBe("#111214");
  });

  it("gives Studio a deeper background, with the cursor accent matching it", () => {
    for (const name of ["dark", "light"] as const) {
      const studio = terminalTheme(name, "studio");
      expect(studio.background).not.toBe(terminalTheme(name).background);
      expect(studio.cursorAccent).toBe(studio.background);
      expect(studio.foreground).toBe(terminalTheme(name).foreground);
    }
  });

  it("keeps High contrast pure black in both layouts", () => {
    expect(terminalTheme("high-contrast", "studio")).toEqual(terminalTheme("high-contrast", "classic"));
  });
});

describe("Ocean and Paper themes", () => {
  it("are kept by settings and have terminal colours of their own", () => {
    for (const name of ["ocean", "paper"] as const) {
      expect(sanitize({ theme: name }).theme).toBe(name);
      const t = terminalTheme(name);
      expect(t).not.toEqual(terminalTheme("dark"));
      expect(t.background).toMatch(/^#[0-9a-f]{6}$/i);
      expect(terminalTheme(name, "studio").cursorAccent).toBe(terminalTheme(name, "studio").background);
    }
  });
});
