// Covers secrets in terminal output with a bar. The text underneath is
// untouched, so selecting and copying still gives the real value; clicking
// a bar uncovers that one secret.

import type { IDecoration, IMarker, Terminal } from "@xterm/xterm";
import { findSecretsInLines, keyBlockOpen } from "./secrets";

interface Segment {
  x: number;
  width: number;
  kind: string;
}

interface Covered {
  marker: IMarker;
  decorations: IDecoration[];
  /** The segments drawn on this row, to skip redrawing an unchanged row. */
  sig: string;
}

/** How far back a rescan starts, to catch a secret split by a late write. */
const BACK = 2;
/** How far back to look for the start of a private key block. */
const KEY_LOOKBACK = 120;

export class SecretMasker {
  private enabled: boolean;
  private covered: Covered[] = [];
  private revealed = new Set<string>();
  /** Absolute row up to which the normal buffer has been scanned. */
  private scannedTo = 0;
  private timer: number | undefined;
  private overlay: HTMLDivElement | null = null;
  private readonly subs: { dispose(): void }[] = [];

  constructor(
    private readonly term: Terminal,
    enabled: boolean,
  ) {
    this.enabled = enabled;
    this.subs.push(
      term.onWriteParsed(() => this.schedule()),
      term.onResize(() => this.rescanAll()),
      term.buffer.onBufferChange(() => this.rescanAll()),
    );
  }

  setEnabled(on: boolean) {
    if (on === this.enabled) return;
    this.enabled = on;
    if (on) this.rescanAll();
    else this.clear();
  }

  /** How many bars are showing, for tests and the status bar. */
  get count() {
    return this.covered.reduce((n, c) => n + c.decorations.length, 0);
  }

  dispose() {
    window.clearTimeout(this.timer);
    this.clear();
    for (const s of this.subs) s.dispose();
    this.overlay?.remove();
  }

  private clear() {
    this.clearOverlay();
    for (const c of this.covered) {
      for (const d of c.decorations) d.dispose();
      c.marker.dispose();
    }
    this.covered = [];
  }

  private schedule() {
    if (!this.enabled || this.timer !== undefined) return;
    this.timer = window.setTimeout(() => {
      this.timer = undefined;
      this.scanRecent();
    }, 40);
  }

  private rescanAll() {
    if (!this.enabled) return;
    this.clear();
    const buf = this.term.buffer.active;
    // The alternate screen (vim, less, top) has no scrollback: scan what is shown.
    this.scan(buf.type === "alternate" ? buf.viewportY : 0, buf.length - 1);
    this.scannedTo = buf.baseY + buf.cursorY;
  }

  private scanRecent() {
    const buf = this.term.buffer.active;
    const cursor = buf.baseY + buf.cursorY;
    if (buf.type === "alternate") {
      // Full-screen programs redraw anywhere, so look at the whole screen.
      this.scan(buf.viewportY, buf.viewportY + this.term.rows - 1);
      return;
    }
    // Scrollback trimming moves rows up; never start past the cursor.
    const from = Math.max(0, Math.min(this.scannedTo, cursor) - BACK);
    this.scan(from, cursor);
    this.scannedTo = cursor;
  }

  /** The logical line (wrapped rows joined) that contains `row`, from its first row. */
  private lineStart(row: number) {
    const buf = this.term.buffer.active;
    while (row > 0 && buf.getLine(row)?.isWrapped) row--;
    return row;
  }

  private scan(from: number, to: number) {
    const want = this.compute(from, to);
    if (!want) return;
    if (this.term.buffer.active.type === "alternate") this.drawOverlay(want);
    else this.decorate(want);
  }

  /** Bars wanted per row (absolute rows) for the lines covering `from`..`to`. */
  private compute(from: number, to: number) {
    const buf = this.term.buffer.active;
    const cols = this.term.cols;
    const first = this.lineStart(Math.max(0, from));
    const last = Math.min(to, buf.length - 1);
    if (last < first) return null;

    // Join wrapped rows into logical lines, remembering where each starts.
    const lines: { row: number; rows: number; text: string }[] = [];
    for (let row = first; row <= last; ) {
      let text = "";
      let rows = 0;
      do {
        const line = buf.getLine(row + rows);
        if (!line) break;
        const next = buf.getLine(row + rows + 1);
        // A row continued by the next keeps its full width so offsets map to cells.
        text += line.translateToString(!next?.isWrapped);
        rows++;
      } while (buf.getLine(row + rows)?.isWrapped);
      lines.push({ row, rows: Math.max(rows, 1), text });
      row += Math.max(rows, 1);
    }

    const before: string[] = [];
    for (let r = Math.max(0, first - KEY_LOOKBACK); r < first; r++) {
      before.push(buf.getLine(r)?.translateToString(true) ?? "");
    }
    const { matches } = findSecretsInLines(
      lines.map((l) => l.text),
      keyBlockOpen(before),
    );

    // Segments per row.
    const want = new Map<number, Segment[]>();
    lines.forEach((l, i) => {
      for (let r = 0; r < l.rows; r++) want.set(l.row + r, []);
      for (const m of matches[i]) {
        for (let off = m.start; off < m.end; ) {
          const r = Math.floor(off / cols);
          const x = off % cols;
          const width = Math.min(m.end - off, cols - x);
          want.get(l.row + r)?.push({ x, width, kind: m.kind });
          off += width;
        }
      }
    });

    return want;
  }

  private decorate(want: Map<number, Segment[]>) {
    const buf = this.term.buffer.active;
    this.covered = this.covered.filter((c) => !c.marker.isDisposed);
    const cursorRow = buf.baseY + buf.cursorY;
    for (const [row, segments] of want) {
      const text = buf.getLine(row)?.translateToString(true) ?? "";
      const shown = segments.filter((s) => !this.revealed.has(this.key(text, s.x, s.width)));
      const sig = JSON.stringify(shown);
      const existing = this.covered.filter((c) => c.marker.line === row);
      if (existing.length === 1 && existing[0].sig === sig) continue;
      for (const c of existing) {
        for (const d of c.decorations) d.dispose();
        c.marker.dispose();
      }
      this.covered = this.covered.filter((c) => !existing.includes(c));
      if (!shown.length) continue;
      const marker = this.term.registerMarker(row - cursorRow);
      if (!marker) continue;
      const decorations = shown.flatMap((s) => {
        const d = this.term.registerDecoration({ marker, x: s.x, width: s.width, layer: "top" });
        if (!d) return [];
        d.onRender((el) => {
          el.classList.add("secret-bar");
          el.title = `Hidden: ${s.kind}. Click to show; copying gives the real text.`;
          el.onclick = () => {
            this.revealed.add(this.key(buf.getLine(marker.line)?.translateToString(true) ?? "", s.x, s.width));
            d.dispose();
          };
        });
        return [d];
      });
      this.covered.push({ marker, decorations, sig });
    }
  }

  /**
   * xterm markers and decorations only exist in the normal buffer, so the
   * alternate screen (less, vim, top) gets plain positioned bars instead.
   * It has no scrollback, so row n is always the nth row on screen.
   */
  private drawOverlay(want: Map<number, Segment[]>) {
    const screen = this.term.element?.querySelector<HTMLElement>(".xterm-screen");
    if (!screen) return;
    if (!this.overlay || this.overlay.parentElement !== screen) {
      this.overlay?.remove();
      this.overlay = document.createElement("div");
      this.overlay.className = "secret-overlay";
      screen.append(this.overlay);
    }
    const cellW = screen.clientWidth / this.term.cols;
    const cellH = screen.clientHeight / this.term.rows;
    const buf = this.term.buffer.active;
    const keep = [...this.overlay.children].filter((c) => {
      const row = Number((c as HTMLElement).dataset.row);
      return !want.has(row);
    });
    const bars: HTMLElement[] = [];
    for (const [row, segments] of want) {
      const text = buf.getLine(row)?.translateToString(true) ?? "";
      for (const s of segments) {
        const id = this.key(text, s.x, s.width);
        if (this.revealed.has(id)) continue;
        const bar = document.createElement("div");
        bar.className = "secret-bar";
        bar.dataset.row = String(row);
        bar.title = `Hidden: ${s.kind}. Click to show; copying gives the real text.`;
        Object.assign(bar.style, {
          left: `${s.x * cellW}px`,
          top: `${(row - buf.viewportY) * cellH}px`,
          width: `${s.width * cellW}px`,
          height: `${cellH}px`,
        });
        bar.onclick = () => {
          this.revealed.add(id);
          bar.remove();
        };
        bars.push(bar);
      }
    }
    this.overlay.replaceChildren(...keep, ...bars);
  }

  private clearOverlay() {
    this.overlay?.replaceChildren();
  }

  private key(text: string, x: number, width: number) {
    return `${x}:${text.slice(x, x + width)}`;
  }
}
