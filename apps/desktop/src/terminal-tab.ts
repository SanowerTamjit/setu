// One terminal tab: an xterm.js instance wired to a shell in the Rust core.

import { Terminal } from "@xterm/xterm";
import type { Tab } from "./tab";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { FindBar } from "./find-bar";
import { onSettingsChange, settings } from "./settings";
import { terminalTheme } from "./themes";
import { connectionOf, describe, type SavedConnection } from "./session";
import {
  closeTerminal,
  openTerminal,
  resizeTerminal,
  sshConnect,
  sshOpenShell,
  writeTerminal,
  type ConnectRequest,
  type ShellProfile,
} from "./ipc";


export interface TabEvents {
  onTitle: (tab: TerminalTab) => void;
  onExit: (tab: TerminalTab, code: number) => void;
  onResize: (tab: TerminalTab) => void;
}

export class TerminalTab implements Tab {
  readonly element: HTMLDivElement;
  readonly term: Terminal;
  private readonly fit = new FitAddon();
  readonly findBar: FindBar;
  private id: number | null = null;
  private exited = false;
  private resizeTimer: number | undefined;
  private readonly unsubscribe: () => void;
  shell: ShellProfile | null = null;
  /** Shown in the status bar: the shell program or `ssh user@host`. */
  detail = "";
  isRemote = false;
  title = "Terminal";
  /** How this SSH pane connected (no secrets), to reconnect after a restart. */
  connection: SavedConnection | null = null;
  /** The tab name a saved host gave this pane. */
  label: string | undefined;
  /**
   * Panes holding the same object share one SSH connection (split panes);
   * session restore uses it to log in once for all of them.
   */
  connectionGroup: object | null = null;
  /** Set while the pane waits for Enter to reconnect after a restart. */
  private reconnect: (() => void) | null = null;

  constructor(
    container: HTMLElement,
    private readonly events: TabEvents,
  ) {
    this.element = document.createElement("div");
    this.element.className = "terminal-pane";
    container.appendChild(this.element);

    this.term = new Terminal({
      // The search addon highlights and counts matches with decorations,
      // which xterm 6 still treats as proposed API.
      allowProposedApi: true,
      cursorBlink: true,
      fontFamily:
        '"JetBrains Mono", "Cascadia Mono", "SF Mono", Menlo, Consolas, "DejaVu Sans Mono", monospace',
      fontSize: settings().fontSize,
      lineHeight: 1.15,
      scrollback: 10000,
      theme: terminalTheme(settings().theme),
    });
    this.term.loadAddon(this.fit);
    this.term.open(this.element);
    this.findBar = new FindBar(this.element, this.term);
    this.unsubscribe = onSettingsChange((s, changed) => {
      if (changed.theme) this.term.options.theme = terminalTheme(s.theme);
      if (changed.fontSize) {
        this.term.options.fontSize = s.fontSize;
        this.refit();
      }
    });

    this.term.onData((data) => {
      if (this.reconnect) {
        if (data.includes("\r")) this.reconnect();
        return;
      }
      if (this.id !== null && !this.exited) void writeTerminal(this.id, data);
    });
    this.term.onTitleChange((title) => {
      if (title.trim()) {
        this.title = title.trim();
        this.events.onTitle(this);
      }
    });
  }

  private rendererLoaded = false;

  /**
   * Uses the GPU (WebGL) renderer on macOS and Windows, and xterm's DOM
   * renderer on Linux: under WebKitGTK the WebGL canvas was presented one
   * frame late in testing, so the newest output stayed invisible until the
   * next keypress. Loaded the first time the tab is visible.
   */
  private loadRenderer() {
    if (this.rendererLoaded) return;
    this.rendererLoaded = true;
    if (/Linux/.test(navigator.userAgent) && !/Android/.test(navigator.userAgent)) return;
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => webgl.dispose());
      this.term.loadAddon(webgl);
    } catch {
      // WebGL is unavailable (for example some Linux VMs); the DOM renderer still works.
    }
  }

  async start(shellId: string | null) {
    this.fit.fit();
    const { id, shell } = await openTerminal(
      shellId,
      this.term.cols,
      this.term.rows,
      (data) => this.write(data),
      (code) => this.handleExit(code),
    );
    this.id = id;
    this.shell = shell;
    this.title = shell.name;
    this.detail = `${shell.name} · ${shell.program}`;
    this.events.onTitle(this);
  }

  /** Connects over SSH. Rejects with a ConnectFailure the caller can act on. */
  async startSsh(request: ConnectRequest) {
    this.fit.fit();
    const { id, title } = await sshConnect(
      request,
      this.term.cols,
      this.term.rows,
      (data) => this.write(data),
      (code) => this.handleExit(code),
    );
    this.id = id;
    this.isRemote = true;
    this.reconnect = null;
    this.connection = connectionOf(request);
    this.connectionGroup = {};
    this.title = title;
    const port = request.port && request.port !== 22 ? `:${request.port}` : "";
    const via = request.jump ? ` via ${request.jump}` : "";
    this.detail = `ssh ${title}${port}${via}`;
    this.events.onTitle(this);
    this.refit();
  }

  /** Opens another shell on `from`'s SSH connection, for a split pane. */
  async startSshFrom(from: TerminalTab) {
    if (from.id === null) throw new Error("that pane is not connected");
    this.fit.fit();
    this.id = await sshOpenShell(
      from.id,
      this.term.cols,
      this.term.rows,
      (data) => this.write(data),
      (code) => this.handleExit(code),
    );
    this.isRemote = true;
    this.reconnect = null;
    this.connection = from.connection;
    this.connectionGroup = from.connectionGroup;
    this.label = from.label;
    this.title = from.title;
    this.detail = from.detail;
    this.events.onTitle(this);
    this.refit();
  }

  /**
   * Shows a restored SSH pane that is not connected yet; Enter calls
   * `onEnter` (which connects, asking only for what it must).
   */
  waitForReconnect(connection: SavedConnection, label: string | undefined, group: object, onEnter: () => void) {
    this.connection = connection;
    this.label = label;
    this.connectionGroup = group;
    this.isRemote = true;
    this.title = label ?? describe(connection);
    const via = connection.jump ? ` via ${connection.jump}` : "";
    this.detail = `ssh ${describe(connection)}${via} · not connected`;
    this.reconnect = onEnter;
    this.term.write(
      `\x1b[2mRestored from your last session.\x1b[0m\r\n` +
        `Press \x1b[1;33mEnter\x1b[0m to reconnect to \x1b[1m${describe(connection)}\x1b[0m${via}.\r\n`,
    );
    this.events.onTitle(this);
  }

  /** Waiting for Enter to reconnect (a restored SSH pane). */
  get awaitingReconnect() {
    return this.reconnect !== null;
  }

  private firstOutput = true;

  private write(data: Uint8Array) {
    if (!this.firstOutput) {
      this.term.write(data);
      return;
    }
    // The WebGL renderer can drop the frame for the shell's first prompt, which
    // arrives while the layout is still settling. Repaint once shortly after.
    this.firstOutput = false;
    this.term.write(data, () => {
      window.setTimeout(() => this.term.refresh(0, this.term.rows - 1), 100);
    });
  }

  private handleExit(code: number) {
    this.exited = true;
    this.term.write(`\r\n\x1b[2m[process exited with code ${code}]\x1b[0m\r\n`);
    this.events.onExit(this, code);
  }

  get size() {
    return { cols: this.term.cols, rows: this.term.rows };
  }

  get sizeLabel() {
    return `${this.term.cols}×${this.term.rows}`;
  }

  get tooltip() {
    return this.shell?.program ?? this.title;
  }

  /** The backend session id, once connected (used to open Files on it). */
  get sessionId() {
    return this.id;
  }

  focus() {
    this.term.focus();
  }

  get hasExited() {
    return this.exited;
  }

  /** `focus: false` shows a pane of a split without taking focus from its sibling. */
  show(focus = true) {
    this.element.classList.add("active");
    this.loadRenderer();
    this.refit();
    if (focus) this.term.focus();
  }

  hide() {
    this.element.classList.remove("active");
  }

  /** Fits the terminal to its pane and tells the shell, debounced for window drags. */
  refit() {
    if (!this.element.classList.contains("active")) return;
    window.clearTimeout(this.resizeTimer);
    this.resizeTimer = window.setTimeout(() => {
      this.fit.fit();
      // Always tell the shell: the PTY may have been opened before the final layout.
      if (this.id !== null && !this.exited) {
        void resizeTerminal(this.id, this.term.cols, this.term.rows);
      }
      this.events.onResize(this);
    }, 30);
  }

  async dispose() {
    if (this.id !== null && !this.exited) await closeTerminal(this.id).catch(() => {});
    this.unsubscribe();
    this.findBar.dispose();
    this.term.dispose();
    this.element.remove();
  }
}
