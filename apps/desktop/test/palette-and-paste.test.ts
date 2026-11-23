import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => null) }));
const { fuzzyScore } = await import("../src/commands");
const { needsPasteCheck, runningLines } = await import("../src/paste-guard");

describe("fuzzyScore", () => {
  it("matches letters in order, case-insensitively", () => {
    expect(fuzzyScore("spr", "Split pane right")).not.toBeNull();
    expect(fuzzyScore("SPR", "split pane right")).not.toBeNull();
    expect(fuzzyScore("rps", "split pane right")).toBeNull();
    expect(fuzzyScore("", "anything")).toBe(0);
  });

  it("prefers word starts and shorter texts", () => {
    const a = fuzzyScore("ft", "Files tab") ?? -1;
    const b = fuzzyScore("ft", "Soft reset") ?? -1;
    expect(a).toBeGreaterThan(b);
    expect(fuzzyScore("web", "web") ?? 0).toBeGreaterThan(fuzzyScore("web", "web-production-01") ?? 0);
  });
});

describe("paste guard", () => {
  const term = (bracketed: boolean) => ({ modes: { bracketedPasteMode: bracketed } }) as never;

  it("counts the lines that would run", () => {
    expect(runningLines("ls")).toBe(0);
    expect(runningLines("ls\n")).toBe(1);
    expect(runningLines("a\r\nb\rc\nd")).toBe(3);
  });

  it("asks only when lines would run unseen", () => {
    expect(needsPasteCheck(term(false), "echo hi")).toBe(false);
    expect(needsPasteCheck(term(false), "echo hi\nrm -rf ~")).toBe(true);
    // Bracketed paste: the shell waits for Enter, nothing to ask.
    expect(needsPasteCheck(term(true), "echo hi\nrm -rf ~")).toBe(false);
  });
});
