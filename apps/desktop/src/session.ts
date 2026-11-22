// What Setu remembers between starts: the open tabs, how they are split, and
// what runs in each pane. Saved as session.json in Setu's config folder.
//
// Never saved: passwords, passphrases, terminal output, or anything typed.

import { invoke } from "@tauri-apps/api/core";
import type { ConnectRequest } from "./ipc";
import type { DialogValues } from "./ssh-dialog";

export const SESSION_VERSION = 1;

/** How to reach an SSH server again, without any secret. */
export interface SavedConnection {
  host: string;
  port: number;
  user: string;
  auth: "default" | "agent" | "keyFile" | "password";
  keyPath?: string;
  jump?: string;
}

export type SavedPane =
  | { kind: "local"; shellId: string | null }
  | {
      kind: "ssh";
      connection: SavedConnection;
      /** The tab name, for saved hosts. */
      label?: string;
      /**
       * Panes of one tab with the same number shared one connection; on
       * restore they share one again, so you log in once.
       */
      group: number;
    };

export type SavedNode =
  | { pane: SavedPane }
  | { split: "row" | "column"; ratio: number; a: SavedNode; b: SavedNode };

export interface SavedTab {
  layout: SavedNode;
  /** Index of the focused pane, in left-to-right, top-to-bottom order. */
  focused: number;
}

export interface Session {
  version: number;
  tabs: SavedTab[];
  active: number;
}

/** Strips the secret parts of a connect request. */
export function connectionOf(request: ConnectRequest): SavedConnection {
  const saved: SavedConnection = {
    host: request.host.trim(),
    port: request.port ?? 22,
    user: request.user,
    auth: request.auth.kind,
  };
  if (request.auth.kind === "keyFile") saved.keyPath = request.auth.path;
  if (request.jump) saved.jump = request.jump;
  return saved;
}

/** Values for the connect dialog that reconnect a saved connection. */
export function dialogValues(c: SavedConnection): DialogValues {
  return {
    host: c.host,
    port: String(c.port),
    user: c.user,
    authKind: c.auth,
    keyPath: c.keyPath ?? "~/.ssh/id_ed25519",
    jump: c.jump ?? "",
  };
}

export function describe(c: SavedConnection): string {
  const port = c.port !== 22 ? `:${c.port}` : "";
  return `${c.user}@${c.host}${port}`;
}

function isNode(v: unknown, depth = 0): v is SavedNode {
  if (typeof v !== "object" || v === null || depth > 16) return false;
  const n = v as Record<string, unknown>;
  if ("pane" in n) {
    const p = n.pane as Record<string, unknown> | null;
    if (!p || typeof p !== "object") return false;
    if (p.kind === "local") return p.shellId === null || typeof p.shellId === "string";
    if (p.kind !== "ssh" || typeof p.group !== "number") return false;
    const c = p.connection as Record<string, unknown> | null;
    return (
      !!c &&
      typeof c.host === "string" &&
      c.host !== "" &&
      typeof c.port === "number" &&
      typeof c.user === "string" &&
      ["default", "agent", "keyFile", "password"].includes(c.auth as string)
    );
  }
  return (
    (n.split === "row" || n.split === "column") &&
    typeof n.ratio === "number" &&
    isNode(n.a, depth + 1) &&
    isNode(n.b, depth + 1)
  );
}

/** The saved session, or null when there is none or it is not one Setu understands. */
export async function loadSession(): Promise<Session | null> {
  try {
    const text = await invoke<string | null>("session_load");
    if (!text) return null;
    const s = JSON.parse(text) as Partial<Session>;
    if (s.version !== SESSION_VERSION || !Array.isArray(s.tabs)) return null;
    const tabs = s.tabs.filter((t): t is SavedTab => !!t && isNode(t.layout) && typeof t.focused === "number");
    if (tabs.length === 0) return null;
    const active = typeof s.active === "number" && s.active >= 0 && s.active < tabs.length ? s.active : 0;
    return { version: SESSION_VERSION, tabs, active };
  } catch {
    return null;
  }
}

let timer: number | undefined;
let pending: (() => Session) | null = null;

/** Saves the session shortly, so a burst of changes writes once. */
export function scheduleSave(snapshot: () => Session) {
  pending = snapshot;
  window.clearTimeout(timer);
  timer = window.setTimeout(flushSave, 400);
}

export function flushSave() {
  window.clearTimeout(timer);
  const snapshot = pending;
  pending = null;
  if (!snapshot) return;
  const session = snapshot();
  void invoke("session_save", { json: JSON.stringify(session) }).catch(() => {
    // Remembering tabs is a convenience; a failed save must not disturb work.
  });
}
