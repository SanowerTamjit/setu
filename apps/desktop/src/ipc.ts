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
  /** Set by builds with extensions, e.g. "Enterprise". */
  edition?: string;
}

export type PanelStatus = "ok" | "warning" | "error";

export interface PanelSection {
  title: string;
  status?: PanelStatus;
  note?: string;
  rows: [string, string][];
  lines?: string[];
  actions?: { id: string; label: string }[];
}

export interface PanelView {
  title: string;
  sections: PanelSection[];
}

/** The settings page a build with extensions adds; `null` in the open-source build. */
export const extensionPanel = () => invoke<PanelView | null>("extension_panel");
export const extensionAction = (id: string) => invoke<{ message: string }>("extension_action", { id });

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

export interface RemoteStat {
  kind: EntryKind;
  size: number;
  modified: number | null;
  permissions: number | null;
  uid: number | null;
  gid: number | null;
  user: string | null;
  group: string | null;
}

export const filesStat = (id: number, path: string) => invoke<RemoteStat>("files_stat", { id, path });
export const filesSetAttributes = (
  id: number,
  dir: string,
  names: string[],
  change: { mode: number | null; user: string | null; group: string | null; recursive: boolean },
) => invoke<number>("files_set_attributes", { id, dir, names, ...change });

// ---- Remote edit ----

export type RemoteEditEvent =
  | { state: "saved"; editId: number; at: number; bytes: number }
  | { state: "conflict"; editId: number }
  | { state: "error"; editId: number; message: string };

export const remoteEditOpen = (id: number, remote: string, editor: string) =>
  invoke<{ editId: number; local: string }>("remote_edit_open", { id, remote, editor });
export const remoteEditPush = (editId: number) => invoke<RemoteEditEvent>("remote_edit_push", { editId });
export const remoteEditStop = (editId: number) => invoke<void>("remote_edit_stop", { editId });

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
  securityKey: boolean;
}

export interface KeyList {
  dir: string;
  keys: KeyView[];
  agent: "none" | "unreachable" | "running";
  agentOnly: string[];
}

export type KeyKind = "ed25519" | "ecdsa" | "rsa" | "ed25519-sk" | "ecdsa-sk";

export interface SecurityKeyChoices {
  resident: boolean;
  verifyRequired: boolean;
  pin: string | null;
}

export const sshKeysList = () => invoke<KeyList>("ssh_keys_list");

export const sshKeyGenerate = (
  name: string,
  kind: KeyKind,
  comment: string,
  passphrase: string | null,
  securityKey: SecurityKeyChoices | null = null,
) =>
  invoke<{ path: string; fingerprint: string; publicLine: string }>("ssh_key_generate", {
    name,
    kind,
    comment,
    passphrase,
    securityKey,
  });

/** Copies the keys stored on the plugged-in security key into ~/.ssh. */
export const sshKeyLoadResident = (pin: string | null) => invoke<string[]>("ssh_key_load_resident", { pin });

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

// ---- Workflows (src-tauri/src/workflows.rs, crates/setu-workflow) ----

export type StepKind = "command" | "approval" | "wait";
export type LinkWhen = "success" | "failure" | "always";

export interface WorkflowStep {
  id: string;
  name: string;
  kind: StepKind;
  command?: string;
  /** Saved host names, or `tag:<tag>`. */
  targets?: string[];
  one_at_a_time?: boolean;
  timeout_seconds?: number | null;
  message?: string;
  seconds?: number;
  x: number;
  y: number;
}

export interface WorkflowLink {
  from: string;
  to: string;
  when?: LinkWhen;
}

export interface Workflow {
  version?: number;
  id: string;
  name: string;
  description?: string;
  step: WorkflowStep[];
  link: WorkflowLink[];
}

export interface WorkflowsView {
  workflows: Workflow[];
  broken: { file: string; error: string }[];
  templates: Workflow[];
  dir: string;
}

export type StepState = "pending" | "running" | "waiting" | "succeeded" | "failed" | "skipped" | "cancelled";
export type RunStatus = "running" | "succeeded" | "failed" | "cancelled";

export interface HostRecord {
  host: string;
  state: StepState;
  code?: number;
  output?: string;
  error?: string;
}

export interface StepRecord {
  step: string;
  name: string;
  state: StepState;
  startedMs?: number;
  finishedMs?: number;
  note?: string;
  hosts?: HostRecord[];
}

export interface RunRecord {
  id: string;
  workflow: string;
  workflowName: string;
  status: RunStatus;
  startedMs: number;
  finishedMs?: number;
  steps: StepRecord[];
}

export type RunEvent =
  | { event: "step"; step: string; state: StepState; note?: string }
  | ({ event: "host"; step: string } & HostRecord)
  | { event: "finished"; status: RunStatus };

export const workflowsList = () => invoke<WorkflowsView>("workflows_list");
export const workflowSave = (workflow: Workflow, previous: string | null) =>
  invoke<WorkflowsView>("workflow_save", { workflow, previous });
export const workflowDelete = (id: string) => invoke<WorkflowsView>("workflow_delete", { id });
export const workflowHistory = () => invoke<RunRecord[]>("workflow_history");
export const workflowHistoryClear = () => invoke<void>("workflow_history_clear");
export const workflowAnswer = (runId: string, step: string, approve: boolean) =>
  invoke<void>("workflow_answer", { runId, step, approve });
export const workflowCancel = (runId: string) => invoke<void>("workflow_cancel", { runId });

/** Starts a saved workflow; progress arrives on `onEvent`. */
export function workflowRun(id: string, onEvent: (e: RunEvent) => void) {
  const channel = new Channel<RunEvent>();
  channel.onmessage = onEvent;
  return invoke<{ runId: string; targets: Record<string, string[]> }>("workflow_run", { id, onEvent: channel });
}

// ---- Task board (board.toml) ----

export type BoardColumn = "todo" | "doing" | "done";
export type CardPriority = "low" | "normal" | "high";

export interface BoardCard {
  id: string;
  title: string;
  column: BoardColumn;
  notes?: string;
  hosts?: string[];
  workflow?: string | null;
  priority?: CardPriority;
  due?: string | null;
}

export interface Board {
  version?: number;
  card: BoardCard[];
}

export const boardLoad = () => invoke<{ board: Board; path: string }>("board_load");
export const boardSave = (board: Board) => invoke<void>("board_save", { board });

// ---- Work memory ----

export interface MemorySettings {
  recording: boolean;
  retention_days: number;
  ignore: string[];
}

export interface MemoryEntry {
  id: number;
  at: number;
  command: string;
  host?: string;
  session: string;
  masked?: boolean;
}

export interface MemorySnippet {
  id: string;
  title: string;
  command: string;
  created: number;
}

export type SuggestionKind = "snippet" | "workflow" | "fleet";

export interface MemorySuggestion {
  id: string;
  kind: SuggestionKind;
  title: string;
  reason: string;
  commands: string[];
  hosts: string[];
  tag?: string;
  count: number;
  last: number;
}

export interface MemoryView {
  settings: MemorySettings;
  entries: MemoryEntry[];
  total: number;
  snippets: MemorySnippet[];
  suggestions: MemorySuggestion[];
  path: string;
}

export type Recorded = "kept" | "keptMasked" | "off" | "skipped";

export const memoryView = () => invoke<MemoryView>("memory_view");
export const memoryRecord = (command: string, host: string | null, session: string) =>
  invoke<Recorded>("memory_record", { command, host, session });
export const memorySettings = (settings: MemorySettings) => invoke<void>("memory_settings", { settings });
export const memoryDelete = (id: number) => invoke<void>("memory_delete", { id });
export const memoryClear = (everything: boolean) => invoke<void>("memory_clear", { everything });
export const memoryDismiss = (id: string) => invoke<void>("memory_dismiss", { id });
export const memorySnippetSave = (title: string, command: string) =>
  invoke<MemorySnippet>("memory_snippet_save", { title, command });
export const memorySnippetDelete = (id: string) => invoke<void>("memory_snippet_delete", { id });
export const memoryWorkflowDraft = (id: string) => invoke<Workflow>("memory_workflow_draft", { id });
export const memorySummary = () => invoke<string>("memory_summary");

/** A colour scheme read from another terminal's file (see color_schemes.rs). */
export interface ParsedScheme {
  name: string;
  foreground: string;
  background: string;
  cursor: string | null;
  selection: string | null;
  ansi: string[];
}

export const colorSchemeParse = (fileName: string, bytes: Uint8Array) =>
  invoke<{ format: string; schemes: ParsedScheme[] }>("color_scheme_parse", { fileName, bytes: Array.from(bytes) });

/** What prompt_detect found for one prompt tool on this computer. */
export interface DetectedPrompt {
  id: string;
  installed: boolean;
  enabled: boolean;
  detail: string | null;
}

export const promptDetect = () => invoke<DetectedPrompt[]>("prompt_detect");

// ---- Tunnels (src-tauri/src/tunnels.rs, crates/setu-ssh/src/tunnel.rs) ----

export type TunnelKind = "local" | "remote" | "dynamic";
export type TunnelPhase = "stopped" | "connecting" | "running" | "reconnecting" | "failed";

export interface TunnelStatus {
  phase: TunnelPhase;
  message?: string;
  listening?: string;
  active: number;
  total: number;
  failed: number;
  reconnects: number;
}

export interface Tunnel {
  /** Empty for a new tunnel. */
  id: string;
  name: string;
  /** The saved host whose login carries it. */
  host: string;
  kind: TunnelKind;
  bindAddress: string;
  bindPort: number;
  targetHost?: string | null;
  targetPort?: number | null;
  autoStart: boolean;
}

export interface TunnelEntry extends Tunnel {
  status: TunnelStatus;
  /** `-L 5433:127.0.0.1:5432`, as OpenSSH writes it. */
  summary: string;
  /** Listens on more than loopback. */
  shared: boolean;
}

export interface TunnelsView {
  tunnels: TunnelEntry[];
  path: string;
  problem?: string;
}

export const tunnelsList = () => invoke<TunnelsView>("tunnels_list");
export const tunnelSave = (tunnel: Tunnel) => invoke<string>("tunnel_save", { tunnel });
export const tunnelDelete = (id: string) => invoke<void>("tunnel_delete", { id });
export const tunnelStart = (id: string) => invoke<void>("tunnel_start", { id });
export const tunnelStop = (id: string) => invoke<void>("tunnel_stop", { id });

// ---- Network tools (src-tauri/src/nettools.rs, crates/setu-net) ----

export type PortAttempt =
  | { result: "open"; micros: number; address: string }
  | { result: "refused"; address: string }
  | { result: "timedOut" }
  | { result: "failed"; reason: string };

export interface PortSummary {
  sent: number;
  open: number;
  minMs?: number | null;
  avgMs?: number | null;
  maxMs?: number | null;
}

export interface PortReport {
  attempts: PortAttempt[];
  summary: PortSummary;
  /** "this computer" or the saved host it ran through. */
  from: string;
}

export interface LookupReport {
  addresses: string[];
  from: string;
}

export interface PingReport {
  /** What the server printed, as plain text. */
  output: string;
  ok: boolean;
  from: string;
}

/** `through` is a saved host's name; empty or missing means this computer. */
export const netCheckPort = (host: string, port: number, attempts: number, through?: string) =>
  invoke<PortReport>("net_check_port", { host, port, attempts, through: through || null });
export const netLookup = (name: string, through?: string) =>
  invoke<LookupReport>("net_lookup", { name, through: through || null });
export const netPing = (host: string, through: string) => invoke<PingReport>("net_ping", { host, through });
export const netWake = (mac: string, broadcast?: string) =>
  invoke<void>("net_wake", { mac, broadcast: broadcast || null });

// ---- Monitoring (src-tauri/src/monitor.rs, crates/setu-monitor) ----

export interface DiskUse {
  mount: string;
  totalKb: number;
  usedKb: number;
  percent: number;
}

export type MonitorAlert =
  | { kind: "cpu"; percent: number }
  | { kind: "memory"; percent: number }
  | { kind: "disk"; mount: string; percent: number };

export interface Snapshot {
  cpuPercent?: number | null;
  cpus: number;
  memPercent?: number | null;
  memUsedKb: number;
  memTotalKb: number;
  swapPercent?: number | null;
  load?: [number, number, number] | null;
  uptimeSecs?: number | null;
  netRxPerSec?: number | null;
  netTxPerSec?: number | null;
  disks: DiskUse[];
  alerts: MonitorAlert[];
  /** Each core, 0-100; empty for the first reading. */
  corePercents: number[];
  interfaces: { name: string; rxPerSec?: number | null; txPerSec?: number | null }[];
  diskIo: { device: string; readPerSec?: number | null; writePerSec?: number | null }[];
  /** The busiest processes: CPU like top (100 is one core). */
  processes: { pid: number; command: string; cpuPercent?: number | null; rssKb: number; memPercent?: number | null }[];
  processCount: number;
}

export type MonitorEvent =
  | { kind: "connecting" }
  | { kind: "sample"; snapshot: Snapshot }
  | { kind: "problem"; message: string; stopped: boolean }
  | { kind: "stopped" };

export interface MonitorLimits {
  cpu: number;
  mem: number;
  disk: number;
}

/** Starts watching a saved host; returns an id for monitorStop. */
export function monitorStart(host: string, interval: number, limits: MonitorLimits, onEvent: (e: MonitorEvent) => void) {
  const channel = new Channel<MonitorEvent>();
  channel.onmessage = onEvent;
  return invoke<string>("monitor_start", { host, interval, cpu: limits.cpu, mem: limits.mem, disk: limits.disk, onEvent: channel });
}
export const monitorStop = (id: string) => invoke<void>("monitor_stop", { id });

export interface CertInfo {
  subject: string;
  issuer: string;
  names: string[];
  /** Seconds since 1970. */
  notBefore: number;
  notAfter: number;
  daysLeft: number;
  sha256: string;
}

export interface TlsReport {
  trusted: boolean;
  problem?: string | null;
  protocol: string;
  cipher: string;
  /** Leaf first. */
  chain: CertInfo[];
  /** The name the certificate was checked for. */
  name: string;
  from: string;
}

export const netTls = (host: string, port: number, name?: string, through?: string) =>
  invoke<TlsReport>("net_tls", { host, port, name: name || null, through: through || null });
