// The Problems view's logic, kept out of DOM code so it can be tested:
// filtering, grouping by host, and the words for times and actions.

import type { Finding, ProblemAction, ProblemCounts, ProblemsList, ProblemSource, Severity } from "./ipc";

export type SeverityFilter = "all" | Severity;

export interface ProblemFilter {
  severity: SeverityFilter;
  /** A saved host name, or "" for every host. */
  host: string;
  text: string;
}

const RANK: Record<Severity, number> = { error: 0, warning: 1, info: 2 };

/** Findings that match the filter. Text matches title, detail, host and source. */
export function filterFindings(list: Finding[], f: ProblemFilter): Finding[] {
  const words = f.text.toLowerCase().split(/\s+/).filter(Boolean);
  return list.filter((p) => {
    if (f.severity !== "all" && p.severity !== f.severity) return false;
    if (f.host && p.host !== f.host) return false;
    if (!words.length) return true;
    const hay = `${p.title} ${p.detail} ${p.host ?? ""} ${sourceLabel(p.source)}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  });
}

export interface ProblemGroup {
  /** The host, or null for problems not about one host (a workflow on several). */
  host: string | null;
  worst: Severity;
  findings: Finding[];
}

/**
 * Groups by host: the hosts with the worst problems first, then by name;
 * problems not about one host come last.
 */
export function groupByHost(list: Finding[]): ProblemGroup[] {
  const groups = new Map<string | null, Finding[]>();
  for (const f of list) {
    const key = f.host ?? null;
    const g = groups.get(key);
    if (g) g.push(f);
    else groups.set(key, [f]);
  }
  const out: ProblemGroup[] = [...groups].map(([host, findings]) => ({
    host,
    findings: [...findings].sort((a, b) => RANK[a.severity] - RANK[b.severity]),
    worst: findings.reduce<Severity>((w, f) => (RANK[f.severity] < RANK[w] ? f.severity : w), "info"),
  }));
  return out.sort((a, b) => {
    if ((a.host === null) !== (b.host === null)) return a.host === null ? 1 : -1;
    return RANK[a.worst] - RANK[b.worst] || (a.host ?? "").localeCompare(b.host ?? "");
  });
}

/** The number on the rail: errors and warnings. */
export function badgeText(c: ProblemCounts): string {
  const n = c.errors + c.warnings;
  if (n <= 0) return "";
  return n > 99 ? "99+" : String(n);
}

/** "just now", "5 min", "3 h", "2 days": how long ago `ms` was. */
export function agoWords(ms: number, now: number): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 60) return "just now";
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h} h`;
  return `${Math.floor(h / 24)} days`;
}

/** "in 45 min", "in 3 h", "in 6 days": how long until `ms`. */
export function untilWords(ms: number, now: number): string {
  const m = Math.max(1, Math.round((ms - now) / 60000));
  if (m < 60) return `in ${m} min`;
  const h = Math.round(m / 60);
  if (h < 48) return `in ${h} h`;
  return `in ${Math.round(h / 24)} days`;
}

export function sourceLabel(s: ProblemSource): string {
  return s === "check" ? "Health check" : s === "monitor" ? "Monitoring" : "Workflow";
}

export function severityLabel(s: Severity): string {
  return s === "error" ? "Error" : s === "warning" ? "Warning" : "Info";
}

/** The button that opens what a problem is about. */
export function actionLabel(a: ProblemAction): string {
  switch (a.kind) {
    case "service":
      return "Open service";
    case "disk":
      return "Open Disk";
    case "monitor":
      return "Open in Monitoring";
    case "workflow":
      return "Open workflow";
    case "connect":
      return "Connect";
  }
}

/** Snooze lengths offered, in minutes. */
export const SNOOZES: [number, string][] = [
  [60, "1 hour"],
  [24 * 60, "1 day"],
  [7 * 24 * 60, "1 week"],
];

/** The line under the title: when hosts were last checked. */
export function checkedWords(v: Pick<ProblemsList, "checked" | "checkedAtMs" | "checking">, hostCount: number, now: number): string {
  if (!hostCount) return "No saved hosts to check.";
  if (!v.checked || v.checkedAtMs === undefined) return v.checking ? "Checking your hosts…" : "Your hosts have not been checked since Setu started.";
  const hosts = v.checked === 1 ? "1 host" : `${v.checked} hosts`;
  const when = agoWords(v.checkedAtMs, now);
  return `${hosts} of ${hostCount} checked${when === "just now" ? " just now" : `, oldest ${when} ago`}.`;
}

/** "Checking… 3 of 12 done" while a check runs. */
export function progressWords(done: number, total: number): string {
  return total ? `Checking… ${Math.min(done, total)} of ${total} done` : "Checking…";
}
