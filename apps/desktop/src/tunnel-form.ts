// The DOM-free parts of the Tunnels view: reading the form, checking it,
// and the words shown for each tunnel. Kept apart so they can be tested in
// Node (test/tunnel-form.test.ts). Rust checks everything again on save.

import type { Tunnel, TunnelEntry, TunnelKind, TunnelStatus } from "./ipc";

export const LOOPBACK = "127.0.0.1";
export const EVERYWHERE = "0.0.0.0";

export const KIND_LABEL: Record<TunnelKind, string> = {
  local: "Local",
  remote: "Remote",
  dynamic: "SOCKS proxy",
};

export const KIND_HELP: Record<TunnelKind, string> = {
  local: "A port on this computer reaches a host and port the server can reach (ssh -L). For databases and admin pages that only listen inside the network.",
  remote: "A port on the server reaches a host and port this computer can reach (ssh -R). For showing a local dev server to the server.",
  dynamic: "A SOCKS5 proxy on this computer (ssh -D). Point a browser or app at it and its traffic leaves from the server.",
};

/** What the person typed, as strings, straight from the inputs. */
export interface TunnelForm {
  id: string;
  name: string;
  host: string;
  kind: TunnelKind;
  bindPort: string;
  /** Let other computers on the network (or, for remote, the server's network) connect. */
  shared: boolean;
  targetHost: string;
  targetPort: string;
  autoStart: boolean;
}

export function parsePort(text: string): number | null {
  const t = text.trim();
  if (!/^\d{1,5}$/.test(t)) return null;
  const n = Number(t);
  return n >= 1 && n <= 65535 ? n : null;
}

const HOST_RE = /^[A-Za-z0-9._:-]{1,253}$/;

/** A Tunnel to save, or the first problem to show. */
export function readForm(f: TunnelForm): { tunnel: Tunnel } | { error: string } {
  const name = f.name.trim();
  if (!name) return { error: "Give the tunnel a name." };
  if (!f.host) return { error: "Choose the saved host to tunnel through." };
  const bindPort = parsePort(f.bindPort);
  if (bindPort === null) return { error: "The listening port must be a number from 1 to 65535." };
  const tunnel: Tunnel = {
    id: f.id,
    name,
    host: f.host,
    kind: f.kind,
    bindAddress: f.shared ? EVERYWHERE : LOOPBACK,
    bindPort,
    autoStart: f.autoStart,
  };
  if (f.kind !== "dynamic") {
    const targetHost = f.targetHost.trim();
    if (!HOST_RE.test(targetHost) || targetHost.startsWith("-"))
      return { error: "The destination must be a host name or IP address, like 127.0.0.1 or db.internal." };
    const targetPort = parsePort(f.targetPort);
    if (targetPort === null) return { error: "The destination port must be a number from 1 to 65535." };
    tunnel.targetHost = targetHost;
    tunnel.targetPort = targetPort;
  }
  return { tunnel };
}

/** The form for an existing tunnel, or a new one. */
export function formFor(t: Tunnel | null, host: string): TunnelForm {
  if (!t)
    return { id: "", name: "", host, kind: "local", bindPort: "", shared: false, targetHost: "127.0.0.1", targetPort: "", autoStart: false };
  return {
    id: t.id,
    name: t.name,
    host: t.host,
    kind: t.kind,
    bindPort: String(t.bindPort),
    shared: !isLoopback(t.bindAddress),
    targetHost: t.targetHost ?? "",
    targetPort: t.targetPort ? String(t.targetPort) : "",
    autoStart: t.autoStart,
  };
}

export function isLoopback(address: string) {
  const a = address.trim().toLowerCase();
  return a === "localhost" || a === "::1" || /^127\./.test(a);
}

/** One line on how to use a tunnel: what to connect to, from where. */
export function usage(t: Tunnel): string {
  const here = isLoopback(t.bindAddress) ? "localhost" : "this computer's address";
  const target = `${t.targetHost}:${t.targetPort}`;
  switch (t.kind) {
    case "local":
      return `Connect to ${here}:${t.bindPort} to reach ${target} through ${t.host}.`;
    case "remote":
      return `On ${t.host}, connections to port ${t.bindPort} reach ${target} on this computer.`;
    case "dynamic":
      return `Set an app's SOCKS5 proxy to ${here}:${t.bindPort}; its traffic leaves from ${t.host}.`;
  }
}

/** The badge text for a tunnel's state. */
export function phaseLabel(s: TunnelStatus): string {
  switch (s.phase) {
    case "stopped":
      return "Stopped";
    case "connecting":
      return "Connecting…";
    case "running":
      return "Running";
    case "reconnecting":
      return "Reconnecting…";
    case "failed":
      return "Failed";
  }
}

/** "2 open · 14 so far" while running, else nothing. */
export function countsLabel(s: TunnelStatus): string {
  if (s.phase !== "running") return "";
  const parts = [`${s.active} open`, `${s.total} so far`];
  if (s.failed) parts.push(`${s.failed} refused`);
  if (s.reconnects) parts.push(`reconnected ${s.reconnects}×`);
  return parts.join(" · ");
}

/** Whether Start (rather than Stop) is the button to show. */
export function canStart(t: TunnelEntry) {
  return t.status.phase === "stopped" || t.status.phase === "failed";
}
