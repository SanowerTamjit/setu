// Server management logic that needs no DOM: what a service's state means
// in words, which changes make sense for it, filtering the list, and the
// wording of the question asked before a change. server-view.ts draws it.

import type { Service, ServiceAction, ServiceDetails } from "./ipc";

export type ServiceFilter = "all" | "running" | "failed" | "stopped";
export type StateKind = "running" | "failed" | "stopped" | "busy" | "other";

export const LOG_LINES = [50, 200, 1000, 5000] as const;
export const DEFAULT_LOG_LINES = 200;

/** The kind of state, for its colour, and a short label. */
export function serviceState(s: Pick<Service, "active" | "sub" | "load"> & { startup?: string }): { kind: StateKind; label: string } {
  if (s.load === "not-found") return { kind: "other", label: "Not found" };
  if (s.load === "masked" || (s.active !== "active" && s.startup?.startsWith("masked"))) return { kind: "other", label: "Masked" };
  switch (s.active) {
    case "active":
      if (s.sub === "running") return { kind: "running", label: "Running" };
      if (s.sub === "exited") return { kind: "running", label: "Done (ran once)" };
      return { kind: "running", label: s.sub ? capital(s.sub) : "Active" };
    case "failed":
      return { kind: "failed", label: "Failed" };
    case "activating":
      return { kind: "busy", label: "Starting" };
    case "deactivating":
      return { kind: "busy", label: "Stopping" };
    case "reloading":
      return { kind: "busy", label: "Reloading" };
    case "inactive":
      return { kind: "stopped", label: "Stopped" };
    default:
      return { kind: "other", label: s.active ? capital(s.active) : "Unknown" };
  }
}

/** Whether it starts at boot, in words; empty when there is nothing to say. */
export function startupLabel(startup: string): string {
  switch (startup) {
    case "enabled":
    case "enabled-runtime":
      return "Starts at boot";
    case "disabled":
      return "Not at boot";
    case "static":
      return "Started by others";
    case "indirect":
      return "Started indirectly";
    case "masked":
    case "masked-runtime":
      return "Masked";
    case "generated":
    case "transient":
      return "Made by systemd";
    case "alias":
      return "Alias";
    default:
      return startup ? capital(startup) : "";
  }
}

export function matchesFilter(s: Service, f: ServiceFilter): boolean {
  if (f === "all") return true;
  const k = serviceState(s).kind;
  if (f === "running") return k === "running" || k === "busy";
  if (f === "failed") return k === "failed";
  return k === "stopped";
}

/** Services matching the filter and every word typed (in name or description). */
export function filterServices(list: Service[], query: string, f: ServiceFilter): Service[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  return list.filter((s) => {
    if (!matchesFilter(s, f)) return false;
    const hay = `${s.name} ${s.description}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  });
}

export function counts(list: Service[]): Record<ServiceFilter, number> {
  const c = { all: list.length, running: 0, failed: 0, stopped: 0 };
  for (const s of list) {
    for (const f of ["running", "failed", "stopped"] as const) if (matchesFilter(s, f)) c[f]++;
  }
  return c;
}

/** The name without ".service", as people say it. */
export function shortName(unit: string): string {
  return unit.endsWith(".service") ? unit.slice(0, -".service".length) : unit;
}

export interface ActionInfo {
  action: ServiceAction;
  label: string;
  /** Whether it makes sense in the service's current state. */
  available: boolean;
  /** Changes that stop something people may rely on. */
  danger: boolean;
}

/** The changes offered for a service, in button order. */
export function actionsFor(s: Pick<Service, "active" | "sub" | "load" | "startup">): ActionInfo[] {
  const kind = serviceState(s).kind;
  const masked = s.load === "masked" || s.startup.startsWith("masked");
  const missing = s.load === "not-found";
  const can = !masked && !missing;
  const up = kind === "running" || kind === "busy";
  return [
    { action: "start", label: "Start", available: can && !up, danger: false },
    { action: "restart", label: "Restart", available: can && (up || kind === "failed"), danger: true },
    { action: "reload", label: "Reload", available: can && kind === "running" && s.sub === "running", danger: false },
    { action: "stop", label: "Stop", available: can && up, danger: true },
    { action: "enable", label: "Start at boot", available: can && s.startup === "disabled", danger: false },
    { action: "disable", label: "Not at boot", available: can && (s.startup === "enabled" || s.startup === "enabled-runtime"), danger: true },
  ];
}

/** Names that, stopped, could cut off the login Setu (and you) use. */
export function cutsAccess(unit: string): boolean {
  return /^(ssh|sshd|openssh-server|openssh|dropbear|networking|network|NetworkManager|systemd-networkd|firewalld|ufw|iptables|nftables)\.service$/.test(unit);
}

/** The question asked before a change: its title, what will happen, and the button. */
export function confirmText(action: ServiceAction, unit: string, host: string): { title: string; what: string; button: string; warning: string | null } {
  const name = shortName(unit);
  const words: Record<ServiceAction, [string, string, string]> = {
    start: [`Start ${name}?`, `Starts ${unit} on ${host} now.`, "Start"],
    stop: [`Stop ${name}?`, `Stops ${unit} on ${host}. Anything it serves stops answering until it is started again.`, "Stop"],
    restart: [`Restart ${name}?`, `Stops and starts ${unit} on ${host}. Connections to it drop for a moment.`, "Restart"],
    reload: [`Reload ${name}?`, `Asks ${unit} on ${host} to read its settings again without stopping.`, "Reload"],
    enable: [`Start ${name} at boot?`, `${unit} will start each time ${host} boots. It does not start it now.`, "Start at boot"],
    disable: [`Stop starting ${name} at boot?`, `${unit} will no longer start when ${host} boots. It keeps running now.`, "Not at boot"],
  };
  const [title, what, button] = words[action];
  const risky = action === "stop" || action === "restart" || action === "disable";
  const warning =
    risky && cutsAccess(unit)
      ? `${unit} may be how you reach ${host}. If it does not come back, you could lose access until someone fixes it at the console.`
      : null;
  return { title, what, button, warning };
}

/** Facts about a service, as label and value pairs, leaving out unknowns. */
export function facts(d: ServiceDetails): [string, string][] {
  const out: [string, string][] = [];
  if (d.mainPid) out.push(["Main process", String(d.mainPid)]);
  if (d.since) out.push([d.active === "active" ? "Running since" : "Last started", d.since]);
  if (d.memoryBytes !== null) out.push(["Memory", bytes(d.memoryBytes)]);
  if (d.tasks !== null) out.push(["Tasks", String(d.tasks)]);
  if (d.restarts) out.push(["Restarts", String(d.restarts)]);
  const boot = startupLabel(d.startup);
  if (boot) out.push(["At boot", boot]);
  if (d.path) out.push(["Unit file", d.path]);
  return out;
}

export function bytes(n: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${i === 0 ? v : v.toFixed(v >= 10 ? 0 : 1)} ${units[i]}`;
}

function capital(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

// ---- Processes ----

export type ProcessSort = "cpu" | "mem";

/** Processes matching every word typed (command, user or pid), sorted. */
export function filterProcesses<T extends { pid: number; user: string; command: string; cpu: number; rssKb: number }>(list: T[], query: string, sort: ProcessSort): T[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const shown = list.filter((p) => {
    const hay = `${p.command} ${p.user} ${p.pid}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  });
  return shown.sort((a, b) => (sort === "cpu" ? b.cpu - a.cpu || b.rssKb - a.rssKb : b.rssKb - a.rssKb || b.cpu - a.cpu));
}

/** How long a process has run, in short words. */
export function elapsedWords(secs: number): string {
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`;
  return `${Math.floor(secs / 86400)}d ${Math.floor((secs % 86400) / 3600)}h`;
}

/** Processes whose stopping could cut off access or bring the server down. */
export function criticalProcess(command: string): boolean {
  return /^(sshd|systemd|init|dbus-daemon|systemd-journal|systemd-logind|NetworkManager|systemd-network)$/.test(command);
}

/** The question asked before signalling a process. */
export function signalText(signal: "term" | "kill", p: { pid: number; command: string }, host: string): { title: string; what: string; button: string; warning: string | null } {
  const name = `${p.command} (${p.pid})`;
  const warning = criticalProcess(p.command) ? `${p.command} keeps ${host} running or reachable. Stopping it could cut off your access or stop the server.` : null;
  return signal === "term"
    ? { title: `Stop ${p.command}?`, what: `Asks ${name} on ${host} to stop. It can finish what it is doing and clean up first.`, button: "Stop", warning }
    : { title: `Kill ${p.command}?`, what: `Stops ${name} on ${host} at once, with no chance to clean up. Unsaved work in it is lost. Use this when Stop did not work.`, button: "Kill", warning };
}

// ---- Disk ----

export function usedPercent(f: { totalKb: number; usedKb: number }): number {
  return f.totalKb > 0 ? Math.min(100, (f.usedKb / f.totalKb) * 100) : 0;
}

/** warn from 80 %, high from 90 %. */
export function fullness(percent: number): "ok" | "warn" | "high" {
  return percent >= 90 ? "high" : percent >= 80 ? "warn" : "ok";
}

/** A path inside `base`, and the folder above a path (null at the top). */
export function childPath(base: string, name: string): string {
  return base === "/" ? `/${name}` : `${base}/${name}`;
}

export function parentPath(path: string): string | null {
  if (path === "/") return null;
  const i = path.lastIndexOf("/");
  return i <= 0 ? "/" : path.slice(0, i);
}

/** The parts of a path for a breadcrumb: label and full path. */
export function crumbs(path: string): [string, string][] {
  const out: [string, string][] = [["/", "/"]];
  let at = "";
  for (const part of path.split("/").filter(Boolean)) {
    at += `/${part}`;
    out.push([part, at]);
  }
  return out;
}

export function kbText(kb: number): string {
  return bytes(kb * 1024);
}

// ---- authorized_keys ----

export function filterKeys<T extends { kind: string; fingerprint: string; comment: string }>(list: T[], query: string): T[] {
  const q = query.trim().toLowerCase();
  if (!q) return list;
  return list.filter((k) => k.comment.toLowerCase().includes(q) || k.fingerprint.toLowerCase().includes(q) || k.kind.toLowerCase().includes(q));
}

/** Short words for a key's options (`from="…"`, `command="…"`, …). */
export function optionWords(options: string): string[] {
  const out: string[] = [];
  // Options are comma-separated, but quoted values may hold commas.
  const parts = options.match(/(?:[^,"]|"(?:[^"\\]|\\.)*")+/g) ?? [];
  for (const raw of parts) {
    const [name, ...rest] = raw.split("=");
    const key = name.trim().toLowerCase();
    const value = rest.join("=").trim().replace(/^"|"$/g, "");
    if (key === "from") out.push(`only from ${value}`);
    else if (key === "command") out.push("runs one fixed command");
    else if (key === "restrict") out.push("restricted");
    else if (key === "no-pty") out.push("no terminal");
    else if (key === "expiry-time") out.push(`expires ${value}`);
    else if (key === "principals") out.push(`principals ${value}`);
    else if (key === "cert-authority") out.push("certificate authority");
    else if (key) out.push(key);
  }
  return out;
}

/** The words of the "remove this key?" dialog. */
export function removeKeyText(k: { kind: string; fingerprint: string; comment: string }, keysLeft: number, host: string): { title: string; what: string; button: string; warning: string } {
  const name = k.comment || k.fingerprint;
  return {
    title: `Remove ${k.comment ? `“${k.comment}”` : "this key"}?`,
    what: `Removes ${name} from authorized_keys on ${host}. Whoever holds the matching private key can no longer log in with it. Sessions already open stay open.`,
    button: "Remove",
    warning:
      keysLeft <= 1
        ? `This is the only key in the file. Once it is gone, nobody can log in to ${host} as this user with a key, and that may include you.`
        : `If this is the key you or Setu log in to ${host} with, that login stops working. Check the fingerprint first.`,
  };
}

export function addKeyText(k: { kind: string; fingerprint: string; comment: string }, host: string): { title: string; what: string; button: string } {
  return {
    title: "Add this key?",
    what: `Lets whoever holds the matching private key log in to ${host} as this user${k.comment ? ` (the key says it is “${k.comment}”)` : ""}.`,
    button: "Add key",
  };
}
