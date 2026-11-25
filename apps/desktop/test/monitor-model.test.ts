import { describe, expect, it } from "vitest";
import { HISTORY, History, alertText, bytes, level, parseLimit, pct, rate, sparkline, uptime } from "../src/monitor-model";
import type { Snapshot } from "../src/ipc";

const snap = (over: Partial<Snapshot> = {}): Snapshot => ({
  cpus: 2,
  memUsedKb: 0,
  memTotalKb: 0,
  disks: [],
  alerts: [],
  ...over,
});

describe("History", () => {
  it("keeps the last points and skips missing ones", () => {
    const h = new History();
    h.add(snap({ memPercent: 40 })); // the first sample has no CPU or network rate
    for (let i = 0; i < HISTORY + 5; i++) h.add(snap({ cpuPercent: i, memPercent: 50, netRxPerSec: 10, netTxPerSec: 1 }));
    expect(h.cpu.length).toBe(HISTORY);
    expect(h.cpu[0]).toBe(5);
    expect(h.mem.length).toBe(HISTORY);
    expect(h.rx.at(-1)).toBe(10);
  });
});

describe("sparkline", () => {
  it("draws right-aligned, scaled to the box", () => {
    expect(sparkline([], 100, 20)).toBe("");
    expect(sparkline([50], 100, 20, 100)).toBe("M100.0 10.0");
    const p = sparkline([0, 100], 118, 20, 100);
    expect(p).toBe("M116.0 20.0 L118.0 0.0");
    // Values above max are clipped, never drawn outside.
    expect(sparkline([500], 10, 20, 100)).toBe("M10.0 0.0");
  });
});

describe("words and numbers", () => {
  it("levels against the limit", () => {
    expect(level(null, 90)).toBe("ok");
    expect(level(50, 90)).toBe("ok");
    expect(level(80, 90)).toBe("warn");
    expect(level(90, 90)).toBe("high");
  });
  it("formats sizes, rates and uptime", () => {
    expect(pct(null)).toBe("…");
    expect(pct(42.6)).toBe("43%");
    expect(bytes(512)).toBe("512 B");
    expect(bytes(1536)).toBe("1.5 KB");
    expect(bytes(250 * 1024 * 1024)).toBe("250 MB");
    expect(rate(2048)).toBe("2.0 KB/s");
    expect(rate(undefined)).toBe("…");
    expect(uptime(90061)).toBe("up 1 day, 1 h");
    expect(uptime(3 * 86400)).toBe("up 3 days, 0 h");
    expect(uptime(3720)).toBe("up 1 h 2 min");
    expect(uptime(59)).toBe("up 0 min");
  });
  it("says what an alert means", () => {
    expect(alertText({ kind: "cpu", percent: 97.2 })).toBe("CPU at 97%");
    expect(alertText({ kind: "disk", mount: "/var", percent: 93 })).toBe("Disk /var is 93% full");
  });
  it("reads limits", () => {
    expect(parseLimit("85", 90)).toBe(85);
    expect(parseLimit("0", 90)).toBe(90);
    expect(parseLimit("150", 90)).toBe(90);
    expect(parseLimit("8.5", 90)).toBe(90);
  });
});
