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
function channels(onOutput: (data: Uint8Array) => void, onExit: (code: number) => void) {
  const output = new Channel<ArrayBuffer | number[]>();
  output.onmessage = (msg) =>
    onOutput(msg instanceof ArrayBuffer ? new Uint8Array(msg) : Uint8Array.from(msg));
  const exit = new Channel<number>();
  exit.onmessage = onExit;
  return { onOutput: output, onExit: exit };
}

/** Keeps these sessions' output while their tab moves to another window. */
export const terminalHold = (ids: number[]) => invoke<void>("terminal_hold", { ids });

/** Shows a running session in this window, starting with what it printed meanwhile. */
export function terminalAttach(id: number, onOutput: (data: Uint8Array) => void, onExit: (code: number) => void) {
  return invoke<void>("terminal_attach", { id, ...channels(onOutput, onExit) });
}

/** Opens a new window for a tab; `payload` is handed to it once. */
export const windowDetach = (payload: string, title: string) => invoke<void>("window_detach", { payload, title });

export const windowTake = (token: string) => invoke<string | null>("window_take", { token });

/** What a shell profile changes about its base shell. */
export interface Launch {
  args: string[];
  cwd: string;
  env: [string, string][];
}

export function openTerminal(
  shellId: string | null,
  launch: Launch | null,
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
    launch,
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
  | { kind: "keyFile"; path: string; passphrase: string | null; save: boolean }
  | { kind: "password"; password: string; save: boolean };

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
  | { kind: "needsPassphrase"; path: string; vault: VaultAvailability }
  | { kind: "needsPassword"; vault: VaultAvailability }
  | { kind: "error"; message: string };

export interface OpenedSsh {
  id: number;
  title: string;
  savedToVault: boolean;
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

/** Opens another shell on SSH terminal `terminalId`'s connection (for a split pane). */
export function sshOpenShell(
  terminalId: number,
  cols: number,
  rows: number,
  onOutput: (data: Uint8Array) => void,
  onExit: (code: number) => void,
): Promise<number> {
  const output = new Channel<ArrayBuffer | number[]>();
  output.onmessage = (msg) =>
    onOutput(msg instanceof ArrayBuffer ? new Uint8Array(msg) : Uint8Array.from(msg));
  const exit = new Channel<number>();
  exit.onmessage = onExit;
  return invoke<number>("ssh_open_shell", { terminalId, cols, rows, onOutput: output, onExit: exit });
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

export interface FileScan {
  format: string;
  candidates: ImportCandidate[];
  skipped: string[];
}

export const hostsParseFile = (fileName: string, bytes: Uint8Array) =>
  invoke<FileScan>("hosts_parse_file", { fileName, bytes: Array.from(bytes) });

export interface Imported {
  summary: { added: number; updated: number };
  view: HostsView;
}

export const sshConfigImport = (hosts: Host[], folder: string | null) =>
  invoke<Imported>("ssh_config_import", { hosts, folder });

// ---- Vault (src-tauri/src/vault.rs). Stored secrets never come back to the UI. ----

export type VaultAvailability = "none" | "locked" | "unlocked";
export type SecretKind = "sshPassword" | "keyPassphrase";

export interface VaultEntry {
  kind: SecretKind;
  target: string;
  /** Seconds since the Unix epoch. */
  updated: number;
}

export interface VaultStatus {
  exists: boolean;
  unlocked: boolean;
  path: string;
  entries: VaultEntry[];
  autoLockMinutes: number;
  /** "Unlock automatically on this computer" (the OS keychain). */
  remember: {
    available: boolean;
    /** Such as "macOS Keychain". */
    store: string;
    enabled: boolean;
    problem?: string;
  };
}

export const vaultStatus = () => invoke<VaultStatus>("vault_status");
export const vaultCreate = (password: string, remember?: boolean) =>
  invoke<VaultStatus>("vault_create", { password, remember });
export const vaultUnlock = (password: string, remember?: boolean) =>
  invoke<VaultStatus>("vault_unlock", { password, remember });
export const vaultRemember = (enabled: boolean) => invoke<VaultStatus>("vault_remember", { enabled });
export const vaultLock = () => invoke<VaultStatus>("vault_lock");
export const vaultRemove = (kind: SecretKind, target: string) =>
  invoke<VaultStatus>("vault_remove", { kind, target });
export const vaultChangePassword = (current: string, next: string) =>
  invoke<VaultStatus>("vault_change_password", { current, new: next });

// ---- Files / SFTP (src-tauri/src/files.rs) ----

export type EntryKind = "file" | "dir" | "symlink" | "other";

export interface FileEntry {
  name: string;
  kind: EntryKind;
  size: number;
  modified: number | null;
  permissions: number | null;
}

export interface Listing {
  path: string;
  parent: string | null;
  entries: FileEntry[];
}

export interface TransferProgress {
  doneBytes: number;
  totalBytes: number;
  filesDone: number;
  filesTotal: number;
  current: string;
  /** Bytes already there from an earlier, stopped copy. */
  resumedBytes: number;
}

export const filesOpen = (terminalId: number) =>
  invoke<{ id: number; home: string }>("files_open", { terminalId });
export const filesClose = (id: number) => invoke<void>("files_close", { id });
export const filesList = (id: number, path: string) => invoke<Listing>("files_list", { id, path });
export const filesMkdir = (id: number, dir: string, name: string) =>
  invoke<void>("files_mkdir", { id, dir, name });
export const filesRename = (id: number, dir: string, from: string, to: string) =>
  invoke<void>("files_rename", { id, dir, from, to });
export const filesRemove = (id: number, dir: string, name: string) =>
  invoke<void>("files_remove", { id, dir, name });
export const localList = (path: string) => invoke<Listing>("local_list", { path });

export function transfer(
  id: number,
  transferId: number,
  direction: "upload" | "download",
  source: string,
  targetDir: string,
  resume: boolean,
  onProgress: (p: TransferProgress) => void,
): Promise<TransferProgress> {
  const channel = new Channel<TransferProgress>();
  channel.onmessage = onProgress;
  return invoke<TransferProgress>("transfer", {
    id,
    transferId,
    direction,
    source,
    targetDir,
    resume,
    onProgress: channel,
  });
}

export const transferCancel = (transferId: number) => invoke<void>("transfer_cancel", { transferId });
export const transferPause = (transferId: number) => invoke<void>("transfer_pause", { transferId });

// ---- SSH keys ----

export interface KeyView {
  path: string;
  name: string;
  state: "plain" | "encrypted" | "unreadable";
  problem: string | null;
  algorithm: string | null;
  fingerprint: string | null;
  comment: string | null;
  publicLine: string | null;
  inAgent: boolean;
}

export interface KeyList {
  dir: string;
  keys: KeyView[];
  agent: "none" | "unreachable" | "running";
  agentOnly: string[];
}

export type KeyKind = "ed25519" | "ecdsa" | "rsa";

export const sshKeysList = () => invoke<KeyList>("ssh_keys_list");

export const sshKeyGenerate = (name: string, kind: KeyKind, comment: string, passphrase: string | null) =>
  invoke<{ path: string; fingerprint: string; publicLine: string }>("ssh_key_generate", {
    name,
    kind,
    comment,
    passphrase,
  });

export const sshKeyAddToAgent = (path: string, passphrase: string | null) =>
  invoke<void>("ssh_key_add_to_agent", { path, passphrase });

export interface KnownHost {
  line: number;
  hosts: string[];
  hashed: boolean;
  marker: string | null;
  algorithm: string;
  fingerprint: string;
  comment: string;
  raw: string;
}

export const knownHostsList = () => invoke<{ path: string; entries: KnownHost[] }>("known_hosts_list");

export const knownHostsRemove = (line: number, raw: string) => invoke<void>("known_hosts_remove", { line, raw });

// ---- Packs ----

export interface Snippet {
  id: string;
  title: string;
  command: string;
  description?: string;
  tags?: string[];
}

export interface PackManifest {
  format: number;
  id: string;
  name: string;
  version: string;
  description: string;
  kind: "role-pack" | "snippets";
  roles: string[];
  os: string[];
  permissions: string[];
  contributes: { snippets: Snippet[] };
}

export interface PackView extends PackManifest {
  builtIn: boolean;
}

export const packsList = () => invoke<PackView[]>("packs_list");
export const packInspect = (text: string) =>
  invoke<{ manifest: PackManifest; replaces: string | null }>("pack_inspect", { text });
export const packInstall = (text: string) => invoke<PackManifest>("pack_install", { text });
export const packRemove = (id: string) => invoke<void>("pack_remove", { id });
