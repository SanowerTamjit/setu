import { describe, expect, it } from "vitest";
import { bindingsFor, chordOf, KEYMAPS, prefixBytes, prefixOf } from "../src/keymap";

const key = (code: string, mods: Partial<KeyboardEvent> = {}, k = "x") =>
  ({ code, key: k, ctrlKey: false, shiftKey: false, altKey: false, metaKey: false, ...mods }) as KeyboardEvent;

describe("chordOf", () => {
  it("names chords by physical key", () => {
    expect(chordOf(key("KeyT", { ctrlKey: true, shiftKey: true }))).toBe("Ctrl+Shift+T");
    expect(chordOf(key("Digit1", { altKey: true }))).toBe("Alt+1");
    expect(chordOf(key("F4"))).toBe("F4");
    // Cmd on macOS counts as Ctrl.
    expect(chordOf(key("KeyC", { metaKey: true, shiftKey: true }))).toBe("Ctrl+Shift+C");
  });

  it("reads Shift+; as :", () => {
    expect(chordOf(key("Semicolon", { shiftKey: true }))).toBe(":");
  });

  it("ignores lone modifiers and unknown keys", () => {
    expect(chordOf(key("ShiftLeft", { shiftKey: true }, "Shift"))).toBeNull();
    expect(chordOf(key("IntlRo"))).toBeNull();
  });
});

describe("keymaps", () => {
  it("every profile binds the basics without clashes", () => {
    for (const { id } of KEYMAPS) {
      const b = bindingsFor(id);
      expect(b["terminal.paste"]?.length, id).toBeGreaterThan(0);
      const seen = new Map<string, string>();
      for (const [command, chords] of Object.entries(b)) {
        for (const chord of chords) {
          expect(seen.get(chord), `${id}: ${chord} is bound to ${seen.get(chord)} and ${command}`).toBeUndefined();
          seen.set(chord, command);
        }
      }
    }
  });

  it("prefix profiles send the prefix byte when pressed twice", () => {
    for (const { id } of KEYMAPS) {
      const prefix = prefixOf(id);
      if (prefix) expect(prefixBytes(prefix).length, id).toBe(1);
    }
    expect(prefixBytes("Ctrl+Q")).toBe("");
  });
});
