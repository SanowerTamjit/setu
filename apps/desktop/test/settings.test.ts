import { beforeEach, describe, expect, it, vi } from "vitest";

const saved: string[] = [];
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string, args?: { json?: string }) => {
    if (cmd === "settings_save" && args?.json) saved.push(args.json);
    return null;
  }),
}));

const { DEFAULTS, sanitize, settings, updateSettings } = await import("../src/settings");
const { enabled, FLAGS, isOverridden, setEnabled } = await import("../src/features");
type FeatureFlag = import("../src/features").FeatureFlag;

describe("sanitize", () => {
  it("falls back to defaults for anything unusable", () => {
    expect(sanitize(null)).toEqual(DEFAULTS);
    expect(sanitize("x")).toEqual(DEFAULTS);
    const s = sanitize({ theme: "neon", fontSize: 400, renderer: "vulkan", editor: "x".repeat(501), maskSecrets: "yes" });
    expect(s.theme).toBe(DEFAULTS.theme);
    expect(s.fontSize).toBe(DEFAULTS.fontSize);
    expect(s.renderer).toBe(DEFAULTS.renderer);
    expect(s.editor).toBe("");
    expect(s.maskSecrets).toBe(true);
  });

  it("keeps valid values", () => {
    const s = sanitize({ theme: "light", keymap: "vim", fontSize: 16, editor: " code --wait ", maskSecrets: false });
    expect(s).toMatchObject({ theme: "light", keymap: "vim", fontSize: 16, editor: "code --wait", maskSecrets: false });
  });

  it("only takes boolean feature flags with plain names", () => {
    const s = sanitize({ features: { pasteGuard: false, "__proto__": true, "bad name": true, remoteEdit: "no" } });
    expect(s.features).toEqual({ pasteGuard: false });
  });

  it("drops malformed shell profiles and non-string environment values", () => {
    const s = sanitize({
      shellProfiles: [
        { id: "p", name: "P", base: "bash", args: ["-l", 3], env: { A: "1", B: 2 } },
        { id: "q" },
        "nope",
      ],
    });
    expect(s.shellProfiles).toEqual([{ id: "p", name: "P", base: "bash", args: ["-l"], cwd: "", env: { A: "1" } }]);
  });
});

describe("feature flags", () => {
  beforeEach(() => {
    for (const f of FLAGS) setEnabled(f.id, null);
  });

  it("start at their defaults", () => {
    for (const f of FLAGS) {
      expect(enabled(f.id)).toBe(f.default);
      expect(isOverridden(f.id)).toBe(false);
    }
  });

  it("remember a change and save it", () => {
    saved.length = 0;
    setEnabled("pasteGuard", false);
    expect(enabled("pasteGuard")).toBe(false);
    expect(isOverridden("pasteGuard")).toBe(true);
    expect(JSON.parse(saved.at(-1) ?? "{}").features).toEqual({ pasteGuard: false });
  });

  it("setting a flag back to its default clears the override", () => {
    setEnabled("remoteEdit", false);
    setEnabled("remoteEdit", true);
    expect(isOverridden("remoteEdit")).toBe(false);
    expect(settings().features).toEqual({});
  });

  it("have unique ids, a stage and a description", () => {
    expect(new Set(FLAGS.map((f) => f.id)).size).toBe(FLAGS.length);
    for (const f of FLAGS as readonly FeatureFlag[]) {
      expect(["experimental", "beta", "stable"]).toContain(f.stage);
      expect(f.description.length).toBeGreaterThan(20);
      // Experimental features start off; beta and stable ones start on.
      expect(f.default).toBe(f.stage !== "experimental");
    }
  });

  it("updateSettings ignores unknown fields", () => {
    updateSettings({ fontSize: 15, ...({ evil: 1 } as object) });
    expect(settings().fontSize).toBe(15);
    expect("evil" in settings()).toBe(false);
  });
});
