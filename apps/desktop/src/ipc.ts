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
