// The DOM-free parts of the Monitoring view: a short history per figure,
// the words and numbers shown, and sparkline paths. Kept apart so they can
// be tested in Node (test/monitor-model.test.ts).

import type { ActiveAlert, MonitorAlert, Snapshot } from "./ipc";

/** Points kept per figure (two minutes at the default interval). */
export const HISTORY = 60;

export class History {
  readonly cpu: number[] = [];
  readonly mem: number[] = [];
  readonly rx: number[] = [];
  readonly tx: number[] = [];

  add(s: Snapshot) {
    const push = (list: number[], v: number | null | undefined) => {
      if (v == null || !Number.isFinite(v)) return;
      list.push(v);
      if (list.length > HISTORY) list.splice(0, list.length - HISTORY);
    };
    push(this.cpu, s.cpuPercent);
    push(this.mem, s.memPercent);
    push(this.rx, s.netRxPerSec);
    push(this.tx, s.netTxPerSec);
  }
}

/** An SVG path for `values` in a `width` × `height` box; `max` defaults to the largest value. */
export function sparkline(values: number[], width: number, height: number, max?: number): string {
  if (values.length === 0) return "";
  const top = Math.max(max ?? Math.max(...values), 1e-9);
  const step = values.length > 1 ? width / (HISTORY - 1) : 0;
  const start = width - step * (values.length - 1);
  return values
    .map((v, i) => {
      const x = start + i * step;
      const y = height - (Math.min(Math.max(v, 0), top) / top) * height;
      return `${i === 0 ? "M" : "L"}${x.toFixed(1)} ${y.toFixed(1)}`;
    })
    .join(" ");
}

export type Level = "ok" | "warn" | "high";

/** "high" at the alert limit, "warn" within 15 points of it. */
export function level(percent: number | null | undefined, limit: number): Level {
  if (percent == null) return "ok";
  if (percent >= limit) return "high";
  return percent >= limit - 15 ? "warn" : "ok";
}

export function pct(v: number | null | undefined): string {
  return v == null ? "…" : `${Math.round(v)}%`;
}

const UNITS = ["B", "KB", "MB", "GB", "TB", "PB"];

export function bytes(n: number): string {
  let v = Math.max(0, n);
  let u = 0;
  while (v >= 1024 && u < UNITS.length - 1) {
    v /= 1024;
    u++;
  }
  return `${v >= 100 || u === 0 ? Math.round(v) : v.toFixed(1)} ${UNITS[u]}`;
}

export const kb = (n: number) => bytes(n * 1024);

export function rate(n: number | null | undefined): string {
  return n == null ? "…" : `${bytes(n)}/s`;
}

export function uptime(secs: number | null | undefined): string {
  if (secs == null) return "";
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d > 0) return `up ${d} day${d === 1 ? "" : "s"}, ${h} h`;
  if (h > 0) return `up ${h} h ${m} min`;
  return `up ${m} min`;
}

export function alertText(a: MonitorAlert): string {
  switch (a.kind) {
    case "cpu":
      return `CPU at ${Math.round(a.percent)}%`;
    case "memory":
      return `Memory at ${Math.round(a.percent)}%`;
    case "disk":
      return `Disk ${a.mount} is ${Math.round(a.percent)}% full`;
  }
}

/** Reads a limit typed in Settings: a whole percent from 1 to 100. */
export function parseLimit(text: string, fallback: number): number {
  const n = Number(text.trim());
  return Number.isInteger(n) && n >= 1 && n <= 100 ? n : fallback;
}

export type Detail = "hour" | "processes" | "cores" | "network" | "disks";

export const DETAILS: { id: Detail; label: string }[] = [
  { id: "hour", label: "Last hour" },
  { id: "processes", label: "Processes" },
  { id: "cores", label: "CPU cores" },
  { id: "network", label: "Interfaces" },
  { id: "disks", label: "Disk activity" },
];

/** A process's CPU, like top: "142%" is one and a half cores. */
export function procCpu(v: number | null | undefined): string {
  if (v == null) return "…";
  return v < 10 ? `${v.toFixed(1)}%` : `${Math.round(v)}%`;
}

/** How long a reading must stay over a limit before it alerts, in seconds. */
export const HOLDS = [0, 60, 120, 300, 600];
export const DEFAULT_HOLD = 120;

export function holdLabel(secs: number): string {
  return secs === 0 ? "right away" : `for ${durationWords(secs)}`;
}

/** "45 s", "2 min", "1 h 5 min", as the notifications say it. */
export function durationWords(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min`;
  return m % 60 ? `${Math.floor(m / 60)} h ${m % 60} min` : `${m / 60} h`;
}

/** The words on an alert chip: how long it has fired, or when it will. */
export function activeText(a: ActiveAlert, hold: number): string {
  if (a.firing) return a.secs >= 60 ? `${alertText(a)} · ${durationWords(a.secs)}` : alertText(a);
  return `${alertText(a)} · alerts in ${durationWords(Math.max(1, hold - a.secs))}`;
}

/** An hour of CPU and memory readings, by time, while a panel is open. */
export const HOUR_MS = 3_600_000;

interface Point {
  t: number;
  cpu: number | null;
  mem: number | null;
}

export class HourHistory {
  private readonly points: Point[] = [];

  add(t: number, s: Snapshot) {
    const ok = (v: number | null | undefined) => (v != null && Number.isFinite(v) ? v : null);
    this.points.push({ t, cpu: ok(s.cpuPercent), mem: ok(s.memPercent) });
    const cut = t - HOUR_MS;
    let drop = 0;
    while (drop < this.points.length && this.points[drop].t < cut) drop++;
    if (drop) this.points.splice(0, drop);
  }

  get length() {
    return this.points.length;
  }

  /** Milliseconds since the first reading kept. */
  span(now: number) {
    return this.points.length ? now - this.points[0].t : 0;
  }

  /**
   * The `window` before `now` in `n` equal slices, each the highest reading
   * in it (so a short spike still shows), or null where there was none.
   */
  buckets(now: number, n = 120, window = HOUR_MS): { cpu: (number | null)[]; mem: (number | null)[] } {
    const cpu: (number | null)[] = new Array(n).fill(null);
    const mem: (number | null)[] = new Array(n).fill(null);
    const start = now - window;
    for (const p of this.points) {
      if (p.t < start || p.t > now) continue;
      const i = Math.min(n - 1, Math.floor(((p.t - start) / window) * n));
      if (p.cpu != null) cpu[i] = Math.max(cpu[i] ?? 0, p.cpu);
      if (p.mem != null) mem[i] = Math.max(mem[i] ?? 0, p.mem);
    }
    return { cpu, mem };
  }
}

/** The chart's time scale in minutes: the shortest of 5, 15, 30 or 60 that holds what is kept. */
export function chartMinutes(spanMs: number): number {
  return [5, 15, 30].find((m) => spanMs <= m * 60_000) ?? 60;
}

/** An SVG path across `width` for evenly spaced values, broken where a value is missing. */
export function seriesPath(values: (number | null)[], width: number, height: number, max: number): string {
  if (values.length === 0) return "";
  const step = values.length > 1 ? width / (values.length - 1) : 0;
  const top = Math.max(max, 1e-9);
  const parts: string[] = [];
  values.forEach((v, i) => {
    if (v == null) return;
    const x = (i * step).toFixed(1);
    const y = (height - (Math.min(Math.max(v, 0), top) / top) * height).toFixed(1);
    const joined = i > 0 && values[i - 1] != null;
    parts.push(`${joined ? "L" : "M"}${x} ${y}`);
    // A lone reading becomes a dot (with round line caps).
    if (!joined && values[i + 1] == null) parts.push("h0.1");
  });
  return parts.join(" ");
}
