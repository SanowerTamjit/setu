// A tab that holds one or more terminal panes, split side by side or stacked.
//
// The layout is a binary tree: each split divides its area between two
// children at a ratio the user can drag. Panes stay where xterm created them
// (absolutely positioned in the tab), so splitting never moves a terminal in
// the DOM; only its rectangle changes.

import type { Tab } from "./tab";
import { TerminalTab, type TabEvents } from "./terminal-tab";

export type Direction = "right" | "down";

interface Leaf {
  kind: "leaf";
  pane: TerminalTab;
}

interface Split {
  kind: "split";
  /** "row": children side by side; "column": stacked. */
  axis: "row" | "column";
  /** Share of the first child, 0..1. */
  ratio: number;
  a: Node;
  b: Node;
  divider: HTMLDivElement;
}

type Node = Leaf | Split;

interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** No pane gets smaller than this share of its split. */
const MIN_RATIO = 0.15;

export interface PaneTabEvents {
  /** The title, focused pane or number of panes changed. */
  onChange: (tab: PaneTab) => void;
  /** The focused pane's size changed. */
  onResize: (tab: PaneTab) => void;
}

export class PaneTab implements Tab {
  readonly element: HTMLDivElement;
  private root: Node;
  private focusedPane: TerminalTab;
  private visible = false;

  constructor(
    container: HTMLElement,
    private readonly events: PaneTabEvents,
  ) {
    this.element = document.createElement("div");
    this.element.className = "pane-tab";
    container.appendChild(this.element);
    this.focusedPane = this.createPane();
    this.root = { kind: "leaf", pane: this.focusedPane };
    this.layout();
  }

  /** The pane that has (or last had) keyboard focus. */
  get focused() {
    return this.focusedPane;
  }

  get panes(): TerminalTab[] {
    const out: TerminalTab[] = [];
    const walk = (n: Node): void => {
      if (n.kind === "leaf") out.push(n.pane);
      else {
        walk(n.a);
        walk(n.b);
      }
    };
    walk(this.root);
    return out;
  }

  get paneCount() {
    return this.panes.length;
  }

  // ---- Tab ----

  get title() {
    return this.focusedPane.title;
  }

  set title(value: string) {
    this.focusedPane.title = value;
  }

  get detail() {
    return this.focusedPane.detail;
  }

  set detail(value: string) {
    this.focusedPane.detail = value;
  }

  get tooltip() {
    const n = this.paneCount;
    return n > 1 ? `${this.focusedPane.tooltip} (${n} panes)` : this.focusedPane.tooltip;
  }

  get sizeLabel() {
    return this.focusedPane.sizeLabel;
  }

  /** Exited only when every pane has. */
  get hasExited() {
    return this.panes.every((p) => p.hasExited);
  }

  show() {
    this.visible = true;
    this.element.classList.add("active");
    for (const p of this.panes) p.show(false);
    this.focusedPane.focus();
  }

  hide() {
    this.visible = false;
    this.element.classList.remove("active");
    for (const p of this.panes) p.hide();
  }

  refit() {
    for (const p of this.panes) p.refit();
  }

  focus() {
    this.focusedPane.focus();
  }

  async dispose() {
    await Promise.all(this.panes.map((p) => p.dispose()));
    this.element.remove();
  }

  // ---- Panes ----

  private createPane(): TerminalTab {
    const events: TabEvents = {
      onTitle: () => this.events.onChange(this),
      onExit: (pane) => this.paneExited(pane),
      onResize: (pane) => {
        if (pane === this.focusedPane) this.events.onResize(this);
      },
    };
    const pane = new TerminalTab(this.element, events);
    pane.element.addEventListener("focusin", () => this.setFocused(pane));
    pane.element.addEventListener("mousedown", () => this.setFocused(pane));
    return pane;
  }

  private setFocused(pane: TerminalTab) {
    if (pane === this.focusedPane) return;
    this.focusedPane = pane;
    this.markFocus();
    this.events.onChange(this);
  }

  private markFocus() {
    const several = this.paneCount > 1;
    for (const p of this.panes) p.element.classList.toggle("focused", several && p === this.focusedPane);
  }

  /** A shell that exits in a split closes its pane; a lone pane stays to show the exit code. */
  private paneExited(pane: TerminalTab) {
    if (this.paneCount > 1) void this.closePane(pane);
    else this.events.onChange(this);
  }

  /**
   * Splits the focused pane and starts a shell in the new one: another shell
   * on the same SSH connection for an SSH pane, otherwise the same local shell.
   */
  async split(direction: Direction): Promise<TerminalTab> {
    const from = this.focusedPane;
    const pane = this.createPane();
    const divider = document.createElement("div");
    divider.className = `pane-divider ${direction === "right" ? "vertical" : "horizontal"}`;
    this.element.append(divider);
    const split: Split = {
      kind: "split",
      axis: direction === "right" ? "row" : "column",
      ratio: 0.5,
      a: { kind: "leaf", pane: from },
      b: { kind: "leaf", pane },
      divider,
    };
    this.replace(from, split);
    this.wireDivider(split);
    this.focusedPane = pane;
    this.layout();
    if (this.visible) {
      pane.show(false);
      pane.focus();
    }
    this.events.onChange(this);
    try {
      if (from.isRemote && !from.hasExited && from.sessionId !== null) {
        await pane.startSshFrom(from);
      } else {
        await pane.start(from.isRemote ? null : (from.shell?.id ?? null));
      }
    } catch (err) {
      pane.term.write(`\x1b[31mCould not open the pane: ${String(err)}\x1b[0m\r\n`);
    }
    this.events.onChange(this);
    return pane;
  }

  /** Closes `pane`; its sibling takes the space. Returns false for the last pane. */
  async closePane(pane: TerminalTab = this.focusedPane): Promise<boolean> {
    if (this.paneCount < 2) return false;
    const parent = this.parentOf(pane);
    if (!parent) return false;
    const sibling = parent.a.kind === "leaf" && parent.a.pane === pane ? parent.b : parent.a;
    parent.divider.remove();
    this.replaceNode(parent, sibling);
    if (this.focusedPane === pane) {
      this.focusedPane = this.firstLeaf(sibling);
    }
    await pane.dispose();
    this.layout();
    if (this.visible) this.focusedPane.focus();
    this.events.onChange(this);
    return true;
  }

  /** Moves focus to the nearest pane in a direction (Alt+Arrow). */
  focusNeighbour(dx: number, dy: number) {
    const rects = this.rects();
    const from = rects.get(this.focusedPane);
    if (!from) return;
    const cx = from.x + from.w / 2;
    const cy = from.y + from.h / 2;
    let best: TerminalTab | null = null;
    let bestScore = Infinity;
    for (const [pane, r] of rects) {
      if (pane === this.focusedPane) continue;
      // Must lie in that direction and overlap on the other axis.
      const ahead =
        dx > 0 ? r.x >= from.x + from.w - 0.001 : dx < 0 ? r.x + r.w <= from.x + 0.001 : dy > 0 ? r.y >= from.y + from.h - 0.001 : r.y + r.h <= from.y + 0.001;
      const overlaps = dx !== 0 ? r.y < from.y + from.h && r.y + r.h > from.y : r.x < from.x + from.w && r.x + r.w > from.x;
      if (!ahead || !overlaps) continue;
      const score = Math.abs(r.x + r.w / 2 - cx) + Math.abs(r.y + r.h / 2 - cy);
      if (score < bestScore) {
        bestScore = score;
        best = pane;
      }
    }
    if (best) {
      this.setFocused(best);
      best.focus();
    }
  }

  // ---- Tree helpers ----

  private firstLeaf(n: Node): TerminalTab {
    return n.kind === "leaf" ? n.pane : this.firstLeaf(n.a);
  }

  private parentOf(pane: TerminalTab, n: Node = this.root): Split | null {
    if (n.kind === "leaf") return null;
    for (const child of [n.a, n.b]) {
      if (child.kind === "leaf" && child.pane === pane) return n;
      const found = this.parentOf(pane, child);
      if (found) return found;
    }
    return null;
  }

  /** Replaces the leaf holding `pane` with `node`. */
  private replace(pane: TerminalTab, node: Node) {
    if (this.root.kind === "leaf" && this.root.pane === pane) {
      this.root = node;
      return;
    }
    const parent = this.parentOf(pane);
    if (!parent) return;
    if (parent.a.kind === "leaf" && parent.a.pane === pane) parent.a = node;
    else parent.b = node;
  }

  private replaceNode(target: Node, node: Node, n: Node = this.root): boolean {
    if (this.root === target) {
      this.root = node;
      return true;
    }
    if (n.kind === "leaf") return false;
    if (n.a === target) {
      n.a = node;
      return true;
    }
    if (n.b === target) {
      n.b = node;
      return true;
    }
    return this.replaceNode(target, node, n.a) || this.replaceNode(target, node, n.b);
  }

  // ---- Layout ----

  /** Each pane's rectangle in percent of the tab. */
  private rects(): Map<TerminalTab, Rect> {
    const out = new Map<TerminalTab, Rect>();
    const walk = (n: Node, r: Rect) => {
      if (n.kind === "leaf") {
        out.set(n.pane, r);
        return;
      }
      const [ra, rb] = this.splitRect(n, r);
      walk(n.a, ra);
      walk(n.b, rb);
    };
    walk(this.root, { x: 0, y: 0, w: 100, h: 100 });
    return out;
  }

  private splitRect(n: Split, r: Rect): [Rect, Rect] {
    if (n.axis === "row") {
      const w = r.w * n.ratio;
      return [
        { ...r, w },
        { ...r, x: r.x + w, w: r.w - w },
      ];
    }
    const h = r.h * n.ratio;
    return [
      { ...r, h },
      { ...r, y: r.y + h, h: r.h - h },
    ];
  }

  private layout() {
    const place = (n: Node, r: Rect) => {
      if (n.kind === "leaf") {
        Object.assign(n.pane.element.style, {
          left: `${r.x}%`,
          top: `${r.y}%`,
          width: `${r.w}%`,
          height: `${r.h}%`,
        });
        return;
      }
      const [ra, rb] = this.splitRect(n, r);
      const d = n.divider.style;
      if (n.axis === "row") {
        Object.assign(d, { left: `${rb.x}%`, top: `${r.y}%`, height: `${r.h}%`, width: "" });
      } else {
        Object.assign(d, { top: `${rb.y}%`, left: `${r.x}%`, width: `${r.w}%`, height: "" });
      }
      place(n.a, ra);
      place(n.b, rb);
    };
    place(this.root, { x: 0, y: 0, w: 100, h: 100 });
    this.element.classList.toggle("split", this.paneCount > 1);
    this.markFocus();
    this.refit();
  }

  /** Drag to resize; double-click to share evenly again. */
  private wireDivider(split: Split) {
    const d = split.divider;
    d.addEventListener("dblclick", () => {
      split.ratio = 0.5;
      this.layout();
    });
    d.addEventListener("pointerdown", (e) => {
      e.preventDefault();
      d.setPointerCapture(e.pointerId);
      d.classList.add("dragging");
      // The split's area in pixels, from its two children's current extent.
      const area = this.areaOf(split);
      const move = (ev: PointerEvent) => {
        const ratio =
          split.axis === "row" ? (ev.clientX - area.left) / area.width : (ev.clientY - area.top) / area.height;
        split.ratio = Math.min(1 - MIN_RATIO, Math.max(MIN_RATIO, ratio));
        this.layout();
      };
      const up = () => {
        d.classList.remove("dragging");
        d.removeEventListener("pointermove", move);
        d.removeEventListener("pointerup", up);
        d.removeEventListener("pointercancel", up);
        this.focusedPane.focus();
      };
      d.addEventListener("pointermove", move);
      d.addEventListener("pointerup", up);
      d.addEventListener("pointercancel", up);
    });
  }

  private areaOf(split: Split): DOMRect {
    const host = this.element.getBoundingClientRect();
    let found: Rect = { x: 0, y: 0, w: 100, h: 100 };
    const walk = (n: Node, r: Rect): boolean => {
      if (n === split) {
        found = r;
        return true;
      }
      if (n.kind === "leaf") return false;
      const [ra, rb] = this.splitRect(n, r);
      return walk(n.a, ra) || walk(n.b, rb);
    };
    walk(this.root, found);
    return new DOMRect(
      host.left + (host.width * found.x) / 100,
      host.top + (host.height * found.y) / 100,
      (host.width * found.w) / 100,
      (host.height * found.h) / 100,
    );
  }
}
