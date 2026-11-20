// One terminal tab: an xterm.js instance wired to a shell in the Rust core.

import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import {
  closeTerminal,
  openTerminal,
  resizeTerminal,
  writeTerminal,
  type ShellProfile,
} from "./ipc";

const THEME = {
  background: "#11151c",
  foreground: "#d8dee9",
  cursor: "#f5a524",
  cursorAccent: "#11151c",
  selectionBackground: "#3b4252",
  black: "#1c2129",
  red: "#e06c75",
  green: "#98c379",
  yellow: "#e5c07b",
  blue: "#61afef",
  magenta: "#c678dd",
  cyan: "#56b6c2",
  white: "#d8dee9",
  brightBlack: "#5c6370",
  brightRed: "#ff7a85",
  brightGreen: "#b5e890",
  brightYellow: "#ffd68a",
  brightBlue: "#82c4ff",
  brightMagenta: "#de9bf0",
  brightCyan: "#7fd6e0",
  brightWhite: "#ffffff",
};

export interface TabEvents {
  onTitle: (tab: TerminalTab) => void;
  onExit: (tab: TerminalTab, code: number) => void;
  onResize: (tab: TerminalTab) => void;
}

export class TerminalTab {
  readonly element: HTMLDivElement;
  readonly term: Terminal;
  private readonly fit = new FitAddon();
  private id: number | null = null;
  private exited = false;
  private resizeTimer: number | undefined;
  shell: ShellProfile | null = null;
  title = "Terminal";

  constructor(
    container: HTMLElement,
    private readonly events: TabEvents,
  ) {
    this.element = document.createElement("div");
    this.element.className = "terminal-pane";
    container.appendChild(this.element);

    this.term = new Terminal({
      allowProposedApi: false,
      cursorBlink: true,
      fontFamily:
        '"JetBrains Mono", "Cascadia Mono", "SF Mono", Menlo, Consolas, "DejaVu Sans Mono", monospace',
      fontSize: 14,
      lineHeight: 1.15,
      scrollback: 10000,
      theme: THEME,
    });
    this.term.loadAddon(this.fit);
    this.term.open(this.element);

    this.term.onData((data) => {
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
   * Uses the GPU renderer when WebGL is available, the DOM renderer otherwise.
   * Loaded the first time the tab is visible: attaching WebGL to a hidden pane
   * left the first prompt unpainted in testing.
   */
  private loadRenderer() {
    if (this.rendererLoaded) return;
    this.rendererLoaded = true;
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
    this.events.onTitle(this);
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

  get hasExited() {
    return this.exited;
  }

  show() {
    this.element.classList.add("active");
    this.loadRenderer();
    this.refit();
    this.term.focus();
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
    this.term.dispose();
    this.element.remove();
  }
}
