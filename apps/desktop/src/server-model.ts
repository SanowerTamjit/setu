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
