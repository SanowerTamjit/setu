import { describe, expect, it } from "vitest";
import { HISTORY, HOUR_MS, History, HourHistory, activeText, chartMinutes, durationWords, holdLabel, seriesPath, alertText, bytes, level, parseLimit, pct, procCpu, rate, sparkline, uptime } from "../src/monitor-model";
import type { Snapshot } from "../src/ipc";

const snap = (over: Partial<Snapshot> = {}): Snapshot => ({
  cpus: 2,
  memUsedKb: 0,
  memTotalKb: 0,
  disks: [],
  alerts: [],
  corePercents: [],
  interfaces: [],
  diskIo: [],
  processes: [],
  processCount: 0,
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

describe("details", () => {
  it("formats process CPU like top", () => {
    expect(procCpu(null)).toBe("…");
    expect(procCpu(3.25)).toBe("3.3%");
    expect(procCpu(142.4)).toBe("142%");
  });
});

describe("alerts that last", () => {
  it("says how long an alert has fired, or when it will", () => {
    expect(activeText({ kind: "cpu", percent: 97.4, secs: 30, firing: false }, 120)).toBe("CPU at 97% · alerts in 1 min");
    expect(activeText({ kind: "cpu", percent: 97.4, secs: 100, firing: false }, 120)).toBe("CPU at 97% · alerts in 20 s");
    expect(activeText({ kind: "memory", percent: 93, secs: 10, firing: true }, 0)).toBe("Memory at 93%");
    expect(activeText({ kind: "disk", mount: "/var", percent: 95, secs: 3900, firing: true }, 120)).toBe("Disk /var is 95% full · 1 h 5 min");
  });

  it("words durations and holds like the notifications", () => {
    expect(durationWords(0)).toBe("0 s");
    expect(durationWords(59.9)).toBe("59 s");
    expect(durationWords(120)).toBe("2 min");
    expect(durationWords(7200)).toBe("2 h");
    expect(holdLabel(0)).toBe("right away");
    expect(holdLabel(300)).toBe("for 5 min");
  });
});

describe("HourHistory", () => {
  it("keeps an hour and slices it by the highest reading", () => {
    const h = new HourHistory();
    const t0 = 1_000_000_000;
    h.add(t0, snap({ cpuPercent: 10, memPercent: 40 }));
    h.add(t0 + 1000, snap({ cpuPercent: 80, memPercent: 41 })); // same slice: the spike wins
    h.add(t0 + HOUR_MS / 2, snap({ cpuPercent: 20 }));
    const now = t0 + HOUR_MS - 1;
    const b = h.buckets(now, 4);
    expect(b.cpu).toEqual([80, null, 20, null]);
    expect(b.mem).toEqual([41, null, null, null]);
    expect(h.span(now)).toBe(HOUR_MS - 1);

    h.add(t0 + HOUR_MS + 5000, snap({ cpuPercent: 5 }));
    expect(h.length).toBe(2); // the first two are over an hour old
  });

  it("charts a shorter window while less is kept", () => {
    expect(chartMinutes(0)).toBe(5);
    expect(chartMinutes(7 * 60_000)).toBe(15);
    expect(chartMinutes(16 * 60_000)).toBe(30);
    expect(chartMinutes(HOUR_MS)).toBe(60);
    const h = new HourHistory();
    h.add(0, snap({ cpuPercent: 50 }));
    h.add(150_000, snap({ cpuPercent: 70 }));
    expect(h.buckets(300_000, 2, 300_000).cpu).toEqual([50, 70]);
  });

  it("skips readings that are missing or not numbers", () => {
    const h = new HourHistory();
    h.add(0, snap({ cpuPercent: Number.NaN, memPercent: null }));
    expect(h.buckets(1, 2)).toEqual({ cpu: [null, null], mem: [null, null] });
  });
});

describe("seriesPath", () => {
  it("breaks at gaps and draws lone readings as dots", () => {
    expect(seriesPath([0, 100, null, 50, null], 40, 10, 100)).toBe("M0.0 10.0 L10.0 0.0 M30.0 5.0 h0.1");
    expect(seriesPath([], 40, 10, 100)).toBe("");
    expect(seriesPath([200], 40, 10, 100)).toBe("M0.0 0.0 h0.1");
  });
});
