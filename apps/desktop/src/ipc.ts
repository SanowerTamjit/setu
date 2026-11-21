// Typed wrappers around the Rust commands in src-tauri/src/terminals.rs.

import { Channel, invoke } from "@tauri-apps/api/core";

export type ShellKind =
  | "bash"
  | "zsh"
  | "fish"
  | "sh"
  | "pwsh"
  | "windowspowershell"
  | "cmd"
  | "wsl"
  | "other";

export interface ShellProfile {
  id: string;
  name: string;
  kind: ShellKind;
  program: string;
  args: string[];
}

export interface AppInfo {
  version: string;
  os: string;
  /** Local login name, the default SSH user. */
  user: string;
}

export const appInfo = () => invoke<AppInfo>("app_info");

export const listShells = () => invoke<ShellProfile[]>("list_shells");

export interface OpenedTerminal {
  id: number;
  shell: ShellProfile;
}

/**
 * Starts a shell in a new pseudo-terminal. Output arrives as raw bytes on
 * `onOutput`; `onExit` fires once with the exit code when the shell ends.
 */
export function openTerminal(
  shellId: string | null,
  cols: number,
  rows: number,
  onOutput: (data: Uint8Array) => void,
  onExit: (code: number) => void,
): Promise<OpenedTerminal> {
  const output = new Channel<ArrayBuffer | number[]>();
  output.onmessage = (msg) =>
    onOutput(msg instanceof ArrayBuffer ? new Uint8Array(msg) : Uint8Array.from(msg));
  const exit = new Channel<number>();
  exit.onmessage = onExit;
  return invoke<OpenedTerminal>("open_terminal", {
    shellId,
    cols,
    rows,
    onOutput: output,
    onExit: exit,
  });
}

export const writeTerminal = (id: number, data: string) =>
  invoke<void>("write_terminal", { id, data });

export const resizeTerminal = (id: number, cols: number, rows: number) =>
  invoke<void>("resize_terminal", { id, cols, rows });

export const closeTerminal = (id: number) => invoke<void>("close_terminal", { id });

// ---- SSH ----

export type AuthRequest =
  | { kind: "default" }
  | { kind: "agent" }
  | { kind: "keyFile"; path: string; passphrase: string | null }
  | { kind: "password"; password: string };

export interface ConnectRequest {
  host: string;
  port: number | null;
  user: string;
  auth: AuthRequest;
  jump: string | null;
}

export interface HostKeyInfo {
  host: string;
  port: number;
  algorithm: string;
  fingerprint: string;
  openssh: string;
}

export type ConnectFailure =
  | { kind: "unknownHostKey"; key: HostKeyInfo }
  | { kind: "hostKeyChanged"; key: HostKeyInfo; knownHosts: string; line: number }
  | { kind: "needsPassphrase"; path: string }
  | { kind: "error"; message: string };

export interface OpenedSsh {
  id: number;
  title: string;
}

/** Connects over SSH and opens a remote shell. Rejects with a `ConnectFailure`. */
export function sshConnect(
  request: ConnectRequest,
  cols: number,
  rows: number,
  onOutput: (data: Uint8Array) => void,
  onExit: (code: number) => void,
): Promise<OpenedSsh> {
  const output = new Channel<ArrayBuffer | number[]>();
  output.onmessage = (msg) =>
    onOutput(msg instanceof ArrayBuffer ? new Uint8Array(msg) : Uint8Array.from(msg));
  const exit = new Channel<number>();
  exit.onmessage = onExit;
  return invoke<OpenedSsh>("ssh_connect", { request, cols, rows, onOutput: output, onExit: exit });
}

export const sshTrustHost = (key: HostKeyInfo) => invoke<void>("ssh_trust_host", { key });

// ---- Saved hosts (src-tauri/src/hosts.rs) ----

export type HostAuth = "auto" | "agent" | "key" | "password";

/** A saved host, exactly as stored in hosts.toml. */
export interface Host {
  name: string;
  host: string;
  port?: number;
  user?: string | null;
  folder?: string | null;
  tags?: string[];
  auth?: HostAuth;
  key_file?: string | null;
  jump?: string | null;
  notes?: string | null;
}

export interface HostsView {
  hosts: Host[];
  folders: string[];
  path: string;
}

export const hostsList = () => invoke<HostsView>("hosts_list");

export const hostSave = (host: Host, previous: string | null) =>
  invoke<HostsView>("host_save", { host, previous });

export const hostDelete = (name: string) => invoke<HostsView>("host_delete", { name });

export interface ImportCandidate {
  host: Host;
  status: "new" | "unchanged" | "changed" | "invalid";
  notes: string[];
}

export interface SshConfigScan {
  path: string;
  found: boolean;
  candidates: ImportCandidate[];
  warnings: string[];
}

export const sshConfigScan = () => invoke<SshConfigScan>("ssh_config_scan");

export interface Imported {
  summary: { added: number; updated: number };
  view: HostsView;
}

export const sshConfigImport = (hosts: Host[], folder: string | null) =>
  invoke<Imported>("ssh_config_import", { hosts, folder });
