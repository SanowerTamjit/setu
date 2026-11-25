// The DOM-free parts of the Monitoring view: a short history per figure,
// the words and numbers shown, and sparkline paths. Kept apart so they can
// be tested in Node (test/monitor-model.test.ts).

import type { MonitorAlert, Snapshot } from "./ipc";

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

export type Detail = "processes" | "cores" | "network" | "disks";

export const DETAILS: { id: Detail; label: string }[] = [
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
