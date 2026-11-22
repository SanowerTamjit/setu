// The Workflows view: a canvas to draw a workflow as boxes (steps) joined by
// arrows (links), an inspector to edit the selected step, and a live view
// of a run with each step's state and every host's output.
//
// Steps run commands on saved hosts (by name or `tag:`), wait for an
// approval, or wait for a while. An arrow says when the next step runs:
// after success (green), failure (red) or always (grey, dashed). Steps
// without arrows between them run at the same time.
//
// The canvas only edits a plain object; Rust checks it again on save and
// reads it back from disk to run it.

import { confirmDialog, el, errorText, field } from "./dom";
import { icon, type IconName } from "./icons";
import {
  workflowAnswer,
  workflowCancel,
  workflowDelete,
  workflowHistory,
  workflowHistoryClear,
  workflowRun,
  workflowSave,
  workflowsList,
  type Host,
  type LinkWhen,
  type RunEvent,
  type RunRecord,
  type StepKind,
  type StepRecord,
  type StepState,
  type Workflow,
  type WorkflowLink,
  type WorkflowStep,
  type WorkflowsView as WorkflowsData,
} from "./ipc";

export interface WorkflowsActions {
  hosts: () => Host[];
}

const NODE_W = 220;
const NODE_H = 96;
const GRID = 20;

const KIND: Record<StepKind, { label: string; icon: IconName; hint: string }> = {
  command: { label: "Command", icon: "terminal", hint: "Runs a command on hosts" },
  approval: { label: "Approval", icon: "lock", hint: "Waits until you approve" },
  wait: { label: "Wait", icon: "pause", hint: "Pauses for a while" },
};

const WHEN_LABEL: Record<LinkWhen, string> = { success: "on success", failure: "on failure", always: "always" };

const STATE_LABEL: Record<StepState, string> = {
  pending: "Waiting",
  running: "Running",
  waiting: "Needs approval",
  succeeded: "Done",
  failed: "Failed",
  skipped: "Skipped",
  cancelled: "Stopped",
};

const SVG = "http://www.w3.org/2000/svg";

function snap(n: number) {
  return Math.max(0, Math.round(n / GRID) * GRID);
}

function slug(name: string) {
  const s = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 48);
  return s || "workflow";
}

function clone<T>(v: T): T {
  return JSON.parse(JSON.stringify(v)) as T;
}

function duration(ms: number) {
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  return `${m} min ${s % 60} s`;
}

function when(link: WorkflowLink): LinkWhen {
  return link.when ?? "success";
}

/** Expands `tag:` targets against saved hosts; unknown names are returned separately. */
export function expandTargets(targets: string[], hosts: Host[]) {
  const found: string[] = [];
  const unknown: string[] = [];
  for (const raw of targets) {
    const t = raw.trim();
    if (t.startsWith("tag:")) {
      const tag = t.slice(4).trim().toLowerCase();
      const matched = hosts.filter((h) => (h.tags ?? []).some((x) => x.toLowerCase() === tag));
      if (!matched.length) unknown.push(t);
      for (const h of matched) if (!found.includes(h.name)) found.push(h.name);
    } else if (hosts.some((h) => h.name === t)) {
      if (!found.includes(t)) found.push(t);
    } else unknown.push(t);
  }
  return { found, unknown };
}

/** Whether linking `from` → `to` would close a loop. */
export function wouldLoop(links: WorkflowLink[], from: string, to: string) {
  if (from === to) return true;
  const stack = [to];
  const seen = new Set<string>();
  while (stack.length) {
    const n = stack.pop()!;
    if (n === from) return true;
    if (seen.has(n)) continue;
    seen.add(n);
    for (const l of links) if (l.from === n) stack.push(l.to);
  }
  return false;
}

/** Lays steps out left to right by depth, keeping their order within a column. */
export function arrange(wf: Workflow) {
  const depth = new Map<string, number>();
  const visit = (id: string, seen: Set<string>): number => {
    if (depth.has(id)) return depth.get(id)!;
    if (seen.has(id)) return 0;
    seen.add(id);
    const parents = wf.link.filter((l) => l.to === id).map((l) => visit(l.from, seen) + 1);
    const d = parents.length ? Math.max(...parents) : 0;
    depth.set(id, d);
    return d;
  };
  for (const s of wf.step) visit(s.id, new Set());
  const columns = new Map<number, WorkflowStep[]>();
  for (const s of [...wf.step].sort((a, b) => a.y - b.y)) {
    const d = depth.get(s.id) ?? 0;
    columns.set(d, [...(columns.get(d) ?? []), s]);
  }
  for (const [d, steps] of columns) {
    steps.forEach((s, i) => {
      s.x = 60 + d * (NODE_W + 60);
      s.y = 60 + i * (NODE_H + 60);
    });
  }
}

interface LiveRun {
  /** Empty while the run is starting. */
  id: string;
  record: RunRecord;
  targets: Record<string, string[]>;
  live: boolean;
}

type Selection = { kind: "step"; id: string } | { kind: "link"; index: number } | null;

export class WorkflowsView {
  private data: WorkflowsData | null = null;
  private history: RunRecord[] = [];
  /** The workflow being edited (a copy), and the id it was saved under. */
  private current: Workflow | null = null;
  private savedId: string | null = null;
  private dirty = false;
  private selection: Selection = null;
  private run: LiveRun | null = null;
  private error = "";
  /** Canvas scale; a newly opened workflow is fitted to the space. */
  private zoom = 1;
  private fitPending = true;
  private readonly sizer: HTMLElement;
  private readonly zoomLabel: HTMLElement;

  private readonly list: HTMLElement;
  private readonly main: HTMLElement;
  private readonly canvas: HTMLElement;
  private readonly surface: HTMLElement;
  private readonly edges: SVGSVGElement;
  private readonly inspector: HTMLElement;
  private readonly toolbar: HTMLElement;
  private readonly banner: HTMLElement;

  constructor(
    private readonly root: HTMLElement,
    private readonly actions: WorkflowsActions,
  ) {
    this.list = el("aside", { className: "wf-list" });
    this.list.setAttribute("aria-label", "Workflows");
    this.toolbar = el("div", { className: "wf-toolbar" });
    this.banner = el("div", { className: "wf-banner", hidden: true });
    this.edges = document.createElementNS(SVG, "svg");
    this.edges.classList.add("wf-edges");
    this.surface = el("div", { className: "wf-surface" });
    this.surface.append(this.edges);
    this.sizer = el("div", { className: "wf-sizer" }, this.surface);
    this.canvas = el("div", { className: "wf-canvas", tabIndex: 0 }, this.sizer);
    this.canvas.setAttribute("aria-label", "Workflow canvas");
    this.zoomLabel = el("span", { className: "wf-zoom-label" });
    const zoomButton = (label: string, title: string, run: () => void) => {
      const b = el("button", { type: "button", title }, label);
      b.setAttribute("aria-label", title);
      b.onclick = run;
      return b;
    };
    const zoomBar = el(
      "div",
      { className: "wf-zoom" },
      zoomButton("−", "Zoom out", () => this.setZoom(this.zoom - 0.1)),
      this.zoomLabel,
      zoomButton("+", "Zoom in", () => this.setZoom(this.zoom + 0.1)),
      zoomButton("Fit", "Fit the workflow in view", () => this.fit()),
    );
    this.canvas.addEventListener(
      "wheel",
      (e) => {
        if (!e.ctrlKey) return;
        e.preventDefault();
        this.setZoom(this.zoom * (e.deltaY < 0 ? 1.1 : 1 / 1.1));
      },
      { passive: false },
    );
    this.inspector = el("aside", { className: "wf-inspector" });
    this.main = el(
      "div",
      { className: "wf-main" },
      this.toolbar,
      this.banner,
      el("div", { className: "wf-body" }, el("div", { className: "wf-stage" }, this.canvas, zoomBar), this.inspector),
    );
    this.root.replaceChildren(this.list, this.main);

    this.canvas.onpointerdown = (e) => {
      if (e.target === this.canvas || e.target === this.surface || e.target === this.edges) {
        this.select(null);
      }
    };
    this.canvas.onkeydown = (e) => this.onKey(e);
    this.root.addEventListener("keydown", (e) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s" && this.current) {
        e.preventDefault();
        e.stopPropagation();
        void this.save();
      }
    });
  }

  /** Loads the list from disk; call when the view is shown. */
  async refresh() {
    try {
      const [data, history] = await Promise.all([workflowsList(), workflowHistory()]);
      this.data = data;
      this.history = history;
      this.error = "";
      if (!this.current && data.workflows.length) this.open(data.workflows[0]);
    } catch (err) {
      this.error = errorText(err);
    }
    this.render();
  }

  focus() {
    this.canvas.focus({ preventScroll: true });
  }

  /** Starts a new workflow from a template, or blank. */
  async create(template?: Workflow) {
    if (!(await this.leave())) return;
    const base = template ? clone(template) : { id: "", name: "New workflow", step: [], link: [] };
    const taken = new Set(this.data?.workflows.map((w) => w.id));
    let id = slug(base.name);
    for (let n = 2; taken.has(id); n++) id = `${slug(base.name)}-${n}`;
    this.current = { ...base, id, version: 1 };
    this.savedId = null;
    this.dirty = true;
    this.run = null;
    this.selection = null;
    this.fitPending = true;
    this.render();
    if (!template) this.addStep("command");
  }

  private open(w: Workflow) {
    this.current = clone(w);
    this.current.step ??= [];
    this.current.link ??= [];
    this.savedId = w.id;
    this.dirty = false;
    this.run = null;
    this.selection = null;
    this.fitPending = true;
  }

  private setZoom(z: number) {
    this.zoom = Math.min(1.5, Math.max(0.4, Math.round(z * 100) / 100));
    this.renderCanvas();
  }

  /** Scales the canvas so every step is in view (never above 100%). */
  private fit() {
    const wf = this.current;
    if (!wf || !wf.step.length || !this.canvas.clientWidth) return;
    const w = Math.max(...wf.step.map((s) => s.x + NODE_W)) + 60;
    const h = Math.max(...wf.step.map((s) => s.y + NODE_H)) + 60;
    this.setZoom(Math.min(1, (this.canvas.clientWidth - 16) / w, (this.canvas.clientHeight - 16) / h));
    this.canvas.scrollTo(0, 0);
  }

  /** Asks before throwing away unsaved changes. */
  private async leave() {
    if (this.run?.live) {
      await confirmDialog("A run is going", "Stop the run before opening another workflow.", "OK");
      return false;
    }
    if (!this.dirty || !this.current) return true;
    return confirmDialog(
      "Discard changes?",
      `“${this.current.name}” has changes that are not saved.`,
      "Discard",
      true,
      true,
    );
  }

  private change() {
    this.dirty = true;
    this.render();
  }

  private get editable() {
    return !!this.current && !this.run;
  }

  // ---- Rendering ----

  render() {
    this.renderList();
    this.renderToolbar();
    this.renderBanner();
    this.renderCanvas();
    this.renderInspector();
  }

  private renderList() {
    const header = el(
      "div",
      { className: "wf-list-header" },
      el("h2", {}, "Workflows"),
      this.newMenuButton(),
    );
    const items = (this.data?.workflows ?? []).map((w) => {
      const last = this.history.find((r) => r.workflow === w.id);
      const current = this.current && (this.savedId ?? this.current.id) === w.id;
      const steps = current ? this.current!.step.length : w.step.length;
      const b = el(
        "button",
        { type: "button", className: "wf-item" + (current ? " current" : "") },
        el("span", { className: `wf-dot ${last?.status ?? "none"}` }),
        el(
          "span",
          { className: "wf-item-text" },
          el("span", { className: "wf-item-name" }, current && this.dirty ? `${this.current!.name} •` : w.name),
          el("small", { className: "muted" }, `${steps} step${steps === 1 ? "" : "s"}${last ? " · " + relative(last.startedMs) : ""}`),
        ),
      );
      b.onclick = async () => {
        if (current) return;
        if (!(await this.leave())) return;
        this.open(w);
        this.render();
      };
      return b;
    });
    if (this.current && !this.savedId) {
      items.unshift(
        el(
          "button",
          { type: "button", className: "wf-item current" },
          el("span", { className: "wf-dot none" }),
          el("span", { className: "wf-item-text" }, el("span", { className: "wf-item-name" }, `${this.current.name} •`), el("small", { className: "muted" }, "Not saved yet")),
        ),
      );
    }
    const runs = this.history.filter((r) => r.workflow === (this.savedId ?? "")).slice(0, 6);
    const runList = runs.length
      ? el(
          "div",
          { className: "wf-runs" },
          el("h3", {}, "Recent runs"),
          ...runs.map((r) => {
            const b = el(
              "button",
              { type: "button", className: "wf-run-item" + (this.run?.record.id === r.id ? " current" : "") },
              el("span", { className: `wf-dot ${r.status}` }),
              el("span", {}, new Date(r.startedMs).toLocaleString([], { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" })),
              el("small", { className: "muted" }, r.finishedMs ? duration(r.finishedMs - r.startedMs) : ""),
            );
            b.onclick = () => this.showRecord(r);
            return b;
          }),
        )
      : null;
    const broken = (this.data?.broken ?? []).map((b) =>
      el("div", { className: "wf-broken", title: b.error }, icon("close", 13), `${b.file}: ${b.error}`),
    );
    const footer = el("div", { className: "wf-list-footer muted" });
    if (this.data) {
      footer.append(el("span", { title: this.data.dir }, "Saved as TOML files in your config folder"));
      if (this.history.length) {
        const clear = el("button", { type: "button", className: "link-button" }, "Clear run history");
        clear.onclick = async () => {
          if (!(await confirmDialog("Clear run history?", "Removes every past run and its output.", "Clear", true, true))) return;
          await workflowHistoryClear();
          this.history = [];
          if (this.run && !this.run.live) this.run = null;
          this.render();
        };
        footer.append(clear);
      }
    }
    const empty =
      this.data && !this.data.workflows.length && !this.current
        ? el("p", { className: "muted wf-list-empty" }, "No workflows yet. Start from a template or a blank canvas.")
        : null;
    this.list.replaceChildren(
      header,
      el("div", { className: "wf-items" }, ...items, ...(empty ? [empty] : [])),
      ...broken,
      ...(runList ? [runList] : []),
      footer,
    );
  }

  private newMenuButton() {
    const wrap = el("div", { className: "wf-new" });
    const b = el("button", { type: "button", className: "primary with-icon" }, icon("plus", 15), "New");
    b.setAttribute("aria-haspopup", "menu");
    const menu = el("div", { className: "menu wf-new-menu", role: "menu", hidden: true });
    const item = (label: string, sub: string, run: () => void) => {
      const i = el("button", { type: "button", className: "menu-item", role: "menuitem" }, el("span", {}, label), el("small", { className: "muted" }, sub));
      i.onclick = () => {
        menu.hidden = true;
        run();
      };
      return i;
    };
    menu.append(
      item("Blank workflow", "Start with one command step", () => void this.create()),
      ...(this.data?.templates ?? []).map((t) => item(t.name, t.description ?? "", () => void this.create(t))),
    );
    b.onclick = (e) => {
      e.stopPropagation();
      menu.hidden = !menu.hidden;
      if (!menu.hidden) {
        // Fixed, so the scrolling list beside it cannot clip the menu.
        const r = b.getBoundingClientRect();
        menu.style.top = `${r.bottom + 4}px`;
        menu.style.left = `${Math.max(8, Math.min(r.left, window.innerWidth - menu.offsetWidth - 8))}px`;
        const away = () => {
          menu.hidden = true;
          document.removeEventListener("click", away);
        };
        document.addEventListener("click", away);
      }
    };
    wrap.append(b, menu);
    return wrap;
  }

  private renderToolbar() {
    const wf = this.current;
    if (!wf) {
      this.toolbar.replaceChildren();
      this.toolbar.hidden = true;
      return;
    }
    this.toolbar.hidden = false;
    const name = el("input", { className: "wf-name", value: wf.name, spellcheck: false, disabled: !this.editable });
    name.setAttribute("aria-label", "Workflow name");
    name.oninput = () => {
      wf.name = name.value;
      if (!this.savedId) wf.id = slug(name.value);
      this.dirty = true;
      this.renderList();
      this.renderSaveState();
    };
    const tool = (iconName: IconName, label: string, run: () => void, opts: { primary?: boolean; disabled?: boolean; title?: string; danger?: boolean } = {}) => {
      const b = el(
        "button",
        {
          type: "button",
          className: ["with-icon", opts.primary ? "primary" : "", opts.danger ? "danger" : ""].join(" ").trim(),
          disabled: !!opts.disabled,
          title: opts.title ?? label,
        },
        icon(iconName, 15),
        el("span", { className: "wf-label" }, label),
      );
      b.setAttribute("aria-label", label);
      b.onclick = run;
      return b;
    };
    const add = el(
      "div",
      { className: "wf-add", role: "group" },
      ...(Object.keys(KIND) as StepKind[]).map((k) =>
        tool(KIND[k].icon, KIND[k].label, () => this.addStep(k), { disabled: !this.editable, title: `Add a step: ${KIND[k].hint.toLowerCase()}` }),
      ),
    );
    add.setAttribute("aria-label", "Add a step");
    const right = el("div", { className: "wf-toolbar-right" });
    if (this.run?.live) {
      right.append(tool("close", "Stop run", () => void this.stop(), { danger: true }));
    } else if (this.run) {
      right.append(tool("edit", "Back to editing", () => {
        this.run = null;
        this.render();
      }));
      right.append(tool("retry", "Run again", () => void this.start(), { primary: true }));
    } else {
      right.append(
        el("span", { className: "wf-save-state muted" }),
        tool("layout", "Tidy", () => {
          arrange(wf);
          this.change();
        }, { title: "Arrange the steps left to right" }),
        tool("trash", "Delete", () => void this.remove(), { disabled: !this.savedId }),
        tool("import", "Save", () => void this.save(), { title: "Save (Ctrl+S)" }),
        tool("play", "Run", () => void this.start(), { primary: true, disabled: !wf.step.length }),
      );
    }
    this.toolbar.replaceChildren(name, add, right);
    this.renderSaveState();
  }

  private renderSaveState() {
    const s = this.toolbar.querySelector(".wf-save-state");
    if (s) s.textContent = this.dirty ? "Not saved" : "Saved";
  }

  private renderBanner() {
    const b = this.banner;
    const r = this.run;
    if (this.error) {
      b.hidden = false;
      b.className = "wf-banner error";
      b.replaceChildren(this.error);
      return;
    }
    if (!r) {
      b.hidden = true;
      return;
    }
    b.hidden = false;
    const status = r.record.status;
    b.className = `wf-banner ${status}`;
    const done = r.record.steps.filter((s) => s.state !== "pending" && s.state !== "running" && s.state !== "waiting").length;
    const waiting = r.record.steps.filter((s) => s.state === "waiting");
    const when = new Date(r.record.startedMs).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    const label = {
      running: waiting.length ? `Waiting for your approval: ${waiting.map((s) => s.name).join(", ")}` : `Running… ${done} of ${r.record.steps.length} steps finished`,
      succeeded: `Run from ${when} finished`,
      failed: `Run from ${when} failed`,
      cancelled: `Run from ${when} was stopped`,
    }[status];
    const elapsed = (r.record.finishedMs ?? Date.now()) - r.record.startedMs;
    b.replaceChildren(
      el("span", { className: `wf-dot ${status}` }),
      el("strong", {}, label),
      el("span", { className: "muted" }, r.live ? "" : duration(elapsed)),
    );
  }

  private renderCanvas() {
    const wf = this.current;
    if (!wf) {
      this.surface.replaceChildren(this.edges);
      this.edges.replaceChildren();
      this.canvas.classList.add("empty");
      this.surface.append(
        el(
          "div",
          { className: "wf-empty" },
          icon("jump", 40),
          el("h2", {}, "Automate the steps you run by hand"),
          el("p", { className: "muted" }, "Draw a workflow: run commands on your saved hosts in order or side by side, stop for an approval before risky steps, and roll back when something fails."),
          this.newMenuButton(),
        ),
      );
      return;
    }
    this.canvas.classList.remove("empty");
    this.canvas.classList.toggle("running", !!this.run);
    const width = Math.max(...wf.step.map((s) => s.x + NODE_W + 80), 600);
    const height = Math.max(...wf.step.map((s) => s.y + NODE_H + 80), 400);
    this.surface.style.width = `${width}px`;
    this.surface.style.height = `${height}px`;
    this.surface.style.transform = `scale(${this.zoom})`;
    this.sizer.style.width = `${width * this.zoom}px`;
    this.sizer.style.height = `${height * this.zoom}px`;
    this.zoomLabel.textContent = `${Math.round(this.zoom * 100)}%`;
    if (this.fitPending && this.canvas.clientWidth) {
      this.fitPending = false;
      queueMicrotask(() => this.fit());
    }
    this.edges.setAttribute("width", String(width));
    this.edges.setAttribute("height", String(height));
    this.renderEdges();
    const nodes = wf.step.map((s) => this.node(s));
    this.surface.replaceChildren(this.edges, ...nodes);
  }

  private stepRecord(id: string): StepRecord | undefined {
    return this.run?.record.steps.find((s) => s.step === id);
  }

  private node(step: WorkflowStep) {
    const k = KIND[step.kind];
    const rec = this.stepRecord(step.id);
    const selected = this.selection?.kind === "step" && this.selection.id === step.id;
    const n = el("div", {
      className: ["wf-node", step.kind, selected ? "selected" : "", rec ? `state-${rec.state}` : ""].join(" "),
      tabIndex: 0,
    });
    n.style.left = `${step.x}px`;
    n.style.top = `${step.y}px`;
    n.dataset.step = step.id;
    n.setAttribute("role", "button");
    n.setAttribute("aria-label", `${k.label} step ${step.name}`);
    const detail = (() => {
      if (step.kind === "command") {
        const { found, unknown } = expandTargets(step.targets ?? [], this.actions.hosts());
        const targets = (step.targets ?? []).join(", ") || "no hosts";
        const count = found.length ? ` · ${found.length} host${found.length === 1 ? "" : "s"}` : "";
        return [
          el("code", { className: "wf-node-command" }, step.command || "no command yet"),
          el(
            "small",
            { className: unknown.length || !found.length ? "wf-node-meta warn" : "wf-node-meta muted" },
            icon("hosts", 12),
            `${targets}${count}${step.one_at_a_time ? " · one at a time" : ""}`,
          ),
        ];
      }
      if (step.kind === "approval") return [el("small", { className: "wf-node-meta muted" }, step.message || "Ask before going on")];
      return [el("small", { className: "wf-node-meta muted" }, `Wait ${step.seconds ?? 0} s`)];
    })();
    const head = el(
      "div",
      { className: "wf-node-head" },
      el("span", { className: "wf-node-icon" }, icon(k.icon, 14)),
      el("span", { className: "wf-node-name" }, step.name || step.id),
    );
    if (rec) head.append(el("span", { className: `wf-badge ${rec.state}` }, STATE_LABEL[rec.state]));
    n.append(head, ...detail);
    if (rec?.note && rec.state !== "waiting") n.append(el("small", { className: "wf-node-note" }, rec.note));
    if (rec?.state === "waiting" && this.run?.live) {
      const approve = el("button", { type: "button", className: "primary" }, "Approve");
      const reject = el("button", { type: "button", className: "danger" }, "Reject");
      approve.onclick = (e) => {
        e.stopPropagation();
        void this.answer(step.id, true);
      };
      reject.onclick = (e) => {
        e.stopPropagation();
        void this.answer(step.id, false);
      };
      n.append(el("div", { className: "wf-node-approve" }, reject, approve));
    }
    // Ports: drag from the right one onto another step to link them.
    const input = el("span", { className: "wf-port in" });
    const output = el("span", { className: "wf-port out", title: "Drag to another step to link it" });
    n.append(input, output);
    if (this.editable) {
      output.onpointerdown = (e) => this.startLink(e, step.id);
      n.onpointerdown = (e) => this.startMove(e, step);
    } else {
      n.onpointerdown = () => this.select({ kind: "step", id: step.id });
    }
    n.onfocus = () => {
      if (!(this.selection?.kind === "step" && this.selection.id === step.id)) this.select({ kind: "step", id: step.id }, false);
    };
    return n;
  }

  private renderEdges() {
    const wf = this.current!;
    const byId = new Map(wf.step.map((s) => [s.id, s]));
    const defs = document.createElementNS(SVG, "defs");
    for (const w of ["success", "failure", "always", "selected"]) {
      const m = document.createElementNS(SVG, "marker");
      m.setAttribute("id", `wf-arrow-${w}`);
      m.setAttribute("viewBox", "0 0 10 10");
      m.setAttribute("refX", "9");
      m.setAttribute("refY", "5");
      m.setAttribute("markerWidth", "7");
      m.setAttribute("markerHeight", "7");
      m.setAttribute("orient", "auto-start-reverse");
      const p = document.createElementNS(SVG, "path");
      p.setAttribute("d", "M0 0L10 5L0 10z");
      p.classList.add("wf-arrow", w);
      m.append(p);
      defs.append(m);
    }
    const parts: SVGElement[] = [defs];
    wf.link.forEach((link, index) => {
      const a = byId.get(link.from);
      const b = byId.get(link.to);
      if (!a || !b) return;
      const w = when(link);
      const selected = this.selection?.kind === "link" && this.selection.index === index;
      const d = curve(a.x + NODE_W, a.y + NODE_H / 2, b.x, b.y + NODE_H / 2);
      const g = document.createElementNS(SVG, "g");
      g.classList.add("wf-link", w);
      if (selected) g.classList.add("selected");
      const from = this.stepRecord(link.from)?.state;
      const fired = from && ((from === "succeeded" && w !== "failure") || (from === "failed" && w !== "success"));
      if (this.run && from && from !== "pending" && from !== "running" && from !== "waiting") g.classList.add(fired ? "fired" : "idle");
      const hit = document.createElementNS(SVG, "path");
      hit.setAttribute("d", d);
      hit.classList.add("wf-link-hit");
      const line = document.createElementNS(SVG, "path");
      line.setAttribute("d", d);
      line.classList.add("wf-link-line");
      line.setAttribute("marker-end", `url(#wf-arrow-${selected ? "selected" : w})`);
      g.append(hit, line);
      if (w !== "success") {
        const mx = (a.x + NODE_W + b.x) / 2;
        const my = (a.y + b.y) / 2 + NODE_H / 2;
        const t = document.createElementNS(SVG, "text");
        t.setAttribute("x", String(mx));
        t.setAttribute("y", String(my - 6));
        t.setAttribute("text-anchor", "middle");
        t.classList.add("wf-link-label");
        t.textContent = WHEN_LABEL[w];
        g.append(t);
      }
      if (this.editable) {
        g.onpointerdown = (e) => {
          e.stopPropagation();
          this.select({ kind: "link", index });
          this.focus();
        };
      }
      parts.push(g);
    });
    this.edges.replaceChildren(...parts);
  }

  // ---- Editing ----

  private select(sel: Selection, rerender = true) {
    this.selection = sel;
    if (rerender) {
      this.renderCanvas();
      if (sel?.kind === "step") this.surface.querySelector<HTMLElement>(`[data-step="${CSS.escape(sel.id)}"]`)?.focus({ preventScroll: true });
    } else {
      for (const n of this.surface.querySelectorAll<HTMLElement>(".wf-node")) {
        n.classList.toggle("selected", sel?.kind === "step" && n.dataset.step === sel.id);
      }
    }
    this.renderInspector();
  }

  private addStep(kind: StepKind) {
    const wf = this.current;
    if (!wf || !this.editable) return;
    let n = wf.step.length + 1;
    while (wf.step.some((s) => s.id === `step-${n}`)) n++;
    const sel = this.selection;
    const from = sel?.kind === "step" ? wf.step.find((s) => s.id === sel.id) : wf.step[wf.step.length - 1];
    let x = from ? from.x + NODE_W + 60 : 60;
    let y = from ? from.y : 60;
    while (wf.step.some((s) => Math.abs(s.x - x) < NODE_W && Math.abs(s.y - y) < NODE_H)) y += NODE_H + 40;
    x = snap(x);
    y = snap(y);
    const defaults: Record<StepKind, Partial<WorkflowStep>> = {
      command: { name: "Run command", command: "", targets: [] },
      approval: { name: "Approve", message: "" },
      wait: { name: "Wait", seconds: 30 },
    };
    const step: WorkflowStep = { id: `step-${n}`, kind, x, y, name: "", ...defaults[kind] } as WorkflowStep;
    wf.step.push(step);
    if (from && from !== step) wf.link.push({ from: from.id, to: step.id });
    this.selection = { kind: "step", id: step.id };
    this.change();
    this.surface.querySelector<HTMLElement>(`[data-step="${CSS.escape(step.id)}"]`)?.scrollIntoView({ block: "nearest", inline: "nearest" });
    this.inspector.querySelector<HTMLElement>(kind === "command" ? "textarea" : "input")?.focus();
  }

  private deleteSelection() {
    const wf = this.current;
    const sel = this.selection;
    if (!wf || !sel || !this.editable) return;
    if (sel.kind === "step") {
      wf.step = wf.step.filter((s) => s.id !== sel.id);
      wf.link = wf.link.filter((l) => l.from !== sel.id && l.to !== sel.id);
    } else {
      wf.link.splice(sel.index, 1);
    }
    this.selection = null;
    this.change();
    this.focus();
  }

  private onKey(e: KeyboardEvent) {
    if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
    if ((e.key === "Delete" || e.key === "Backspace") && this.selection) {
      e.preventDefault();
      this.deleteSelection();
    } else if (this.selection?.kind === "step" && e.key.startsWith("Arrow") && this.editable) {
      const s = this.current!.step.find((x) => x.id === (this.selection as { id: string }).id);
      if (!s) return;
      e.preventDefault();
      const d = { ArrowLeft: [-GRID, 0], ArrowRight: [GRID, 0], ArrowUp: [0, -GRID], ArrowDown: [0, GRID] }[e.key];
      if (!d) return;
      s.x = Math.max(0, s.x + d[0]);
      s.y = Math.max(0, s.y + d[1]);
      this.change();
    }
  }

  private startMove(e: PointerEvent, step: WorkflowStep) {
    if (e.button !== 0 || (e.target as HTMLElement).closest("button, .wf-port")) return;
    e.preventDefault();
    this.select({ kind: "step", id: step.id }, false);
    const node = e.currentTarget as HTMLElement;
    node.setPointerCapture(e.pointerId);
    const start = { x: e.clientX, y: e.clientY, sx: step.x, sy: step.y };
    let moved = false;
    node.onpointermove = (m) => {
      const dx = (m.clientX - start.x) / this.zoom;
      const dy = (m.clientY - start.y) / this.zoom;
      if (!moved && Math.hypot(dx, dy) < 3) return;
      moved = true;
      step.x = snap(start.sx + dx);
      step.y = snap(start.sy + dy);
      node.style.left = `${step.x}px`;
      node.style.top = `${step.y}px`;
      this.renderEdges();
    };
    node.onpointerup = () => {
      node.onpointermove = null;
      node.onpointerup = null;
      if (moved) this.change();
      node.focus({ preventScroll: true });
    };
  }

  private canvasPoint(e: PointerEvent) {
    const r = this.surface.getBoundingClientRect();
    return { x: (e.clientX - r.left) / this.zoom, y: (e.clientY - r.top) / this.zoom };
  }

  private startLink(e: PointerEvent, from: string) {
    e.preventDefault();
    e.stopPropagation();
    const wf = this.current!;
    const a = wf.step.find((s) => s.id === from)!;
    const line = document.createElementNS(SVG, "path");
    line.classList.add("wf-link-draft");
    this.edges.append(line);
    const x0 = a.x + NODE_W;
    const y0 = a.y + NODE_H / 2;
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture(e.pointerId);
    const over = (m: PointerEvent) => {
      const p = this.canvasPoint(m);
      return wf.step.find((s) => s.id !== from && p.x >= s.x - 10 && p.x <= s.x + NODE_W && p.y >= s.y && p.y <= s.y + NODE_H);
    };
    target.onpointermove = (m) => {
      const p = this.canvasPoint(m);
      line.setAttribute("d", curve(x0, y0, p.x, p.y));
      const hit = over(m);
      for (const n of this.surface.querySelectorAll<HTMLElement>(".wf-node")) {
        const isHit = n.dataset.step === hit?.id;
        n.classList.toggle("drop", isHit && !wouldLoop(wf.link, from, hit!.id));
        n.classList.toggle("no-drop", isHit && wouldLoop(wf.link, from, hit!.id));
      }
    };
    target.onpointerup = (m) => {
      target.onpointermove = null;
      target.onpointerup = null;
      line.remove();
      const hit = over(m);
      for (const n of this.surface.querySelectorAll<HTMLElement>(".wf-node")) n.classList.remove("drop", "no-drop");
      if (!hit) return;
      if (wouldLoop(wf.link, from, hit.id)) {
        this.flash("That link would make a loop: a workflow must always move forward.");
        return;
      }
      const existing = wf.link.findIndex((l) => l.from === from && l.to === hit.id);
      if (existing >= 0) {
        this.select({ kind: "link", index: existing });
        return;
      }
      wf.link.push({ from, to: hit.id });
      this.selection = { kind: "link", index: wf.link.length - 1 };
      this.change();
    };
  }

  private flash(message: string) {
    this.error = message;
    this.renderBanner();
    window.setTimeout(() => {
      if (this.error === message) {
        this.error = "";
        this.renderBanner();
      }
    }, 4000);
  }

  // ---- Inspector ----

  private renderInspector() {
    const wf = this.current;
    const sel = this.selection;
    if (!wf) {
      this.inspector.hidden = true;
      return;
    }
    this.inspector.hidden = false;
    if (this.run) {
      this.inspector.replaceChildren(...this.runDetails());
      return;
    }
    if (sel?.kind === "link" && wf.link[sel.index]) {
      this.inspector.replaceChildren(...this.linkEditor(wf.link[sel.index]));
      return;
    }
    const step = sel?.kind === "step" ? wf.step.find((s) => s.id === sel.id) : undefined;
    if (!step) {
      this.inspector.replaceChildren(...this.workflowEditor(wf));
      return;
    }
    this.inspector.replaceChildren(...this.stepEditor(step));
  }

  private workflowEditor(wf: Workflow) {
    const desc = el("textarea", { rows: 3, value: wf.description ?? "", placeholder: "What it is for, when to run it" });
    desc.oninput = () => {
      wf.description = desc.value;
      this.dirty = true;
      this.renderSaveState();
    };
    const commands = wf.step.filter((s) => s.kind === "command").length;
    return [
      el("h3", {}, "Workflow"),
      field("Description", desc),
      el(
        "div",
        { className: "wf-help" },
        el("p", {}, `${wf.step.length} steps, ${wf.link.length} links, ${commands} run commands.`),
        el("p", { className: "muted" }, "Add steps from the toolbar. Drag the dot on a step's right edge onto another step to link them; click a link to choose when it runs."),
        el(
          "ul",
          { className: "wf-legend" },
          el("li", {}, el("span", { className: "wf-swatch success" }), "on success"),
          el("li", {}, el("span", { className: "wf-swatch failure" }), "on failure"),
          el("li", {}, el("span", { className: "wf-swatch always" }), "always"),
        ),
        el("p", { className: "muted" }, "Steps that are not linked to each other run at the same time."),
      ),
    ];
  }

  private stepEditor(step: WorkflowStep) {
    const k = KIND[step.kind];
    const set = <K extends keyof WorkflowStep>(key: K, value: WorkflowStep[K], redraw = true) => {
      step[key] = value;
      this.dirty = true;
      this.renderSaveState();
      if (redraw) {
        const node = this.surface.querySelector<HTMLElement>(`[data-step="${CSS.escape(step.id)}"]`);
        node?.replaceWith(this.node(step));
        this.renderList();
      }
    };
    const name = el("input", { value: step.name, spellcheck: false });
    name.oninput = () => set("name", name.value);
    const parts: HTMLElement[] = [
      el("div", { className: "wf-inspector-head" }, el("span", { className: `wf-node-icon ${step.kind}` }, icon(k.icon, 14)), el("h3", {}, `${k.label} step`)),
      field("Name", name),
    ];
    if (step.kind === "command") {
      const cmd = el("textarea", { rows: 4, value: step.command ?? "", spellcheck: false, className: "mono", placeholder: "sudo systemctl restart nginx" });
      cmd.oninput = () => set("command", cmd.value);
      parts.push(field("Command", cmd, "Runs through the host's login shell, like ssh host 'command'. Exit code 0 is success."));
      parts.push(this.targetsEditor(step, () => set("targets", step.targets)));
      const serial = el("input", { type: "checkbox", checked: !!step.one_at_a_time });
      serial.onchange = () => set("one_at_a_time", serial.checked);
      parts.push(el("label", { className: "check-row" }, serial, el("span", {}, "One host at a time, stop at the first failure")));
      const timeout = el("input", { type: "number", min: "1", max: "86400", value: step.timeout_seconds ? String(step.timeout_seconds) : "", placeholder: "No limit" });
      timeout.oninput = () => set("timeout_seconds", timeout.value ? Math.max(1, Number(timeout.value)) : null, false);
      parts.push(field("Timeout (seconds)", timeout));
    } else if (step.kind === "approval") {
      const msg = el("textarea", { rows: 3, value: step.message ?? "", placeholder: "Check the dashboards before going on" });
      msg.oninput = () => set("message", msg.value);
      parts.push(field("What to check", msg, "The run pauses here until you approve. Rejecting fails the step, so a failure link can roll back."));
    } else {
      const secs = el("input", { type: "number", min: "1", max: "86400", value: String(step.seconds ?? 30) });
      secs.oninput = () => set("seconds", Math.max(1, Number(secs.value) || 1));
      parts.push(field("Seconds", secs));
    }
    const del = el("button", { type: "button", className: "danger with-icon" }, icon("trash", 14), "Delete step");
    del.onclick = () => this.deleteSelection();
    parts.push(el("div", { className: "wf-inspector-actions" }, del));
    return parts;
  }

  private targetsEditor(step: WorkflowStep, changed: () => void) {
    const hosts = this.actions.hosts();
    const targets = (step.targets ??= []);
    const chips = el("div", { className: "wf-chips" });
    const draw = () => {
      const { unknown } = expandTargets(targets, hosts);
      chips.replaceChildren(
        ...targets.map((t, i) => {
          const x = el("button", { type: "button", className: "wf-chip-x", title: "Remove" }, icon("close", 11));
          x.setAttribute("aria-label", `Remove ${t}`);
          x.onclick = () => {
            targets.splice(i, 1);
            draw();
            changed();
          };
          return el("span", { className: "wf-chip" + (unknown.includes(t) ? " unknown" : ""), title: unknown.includes(t) ? "Not a saved host or tag" : "" }, t.startsWith("tag:") ? icon("snippet", 11) : icon("hosts", 11), t, x);
        }),
      );
      const { found } = expandTargets(targets, hosts);
      count.textContent = targets.length ? `Runs on ${found.length} host${found.length === 1 ? "" : "s"}${found.length ? ": " + found.slice(0, 6).join(", ") + (found.length > 6 ? "…" : "") : ""}` : "Pick at least one host or tag";
    };
    const count = el("small", { className: "muted" });
    const picker = el("select", {});
    picker.setAttribute("aria-label", "Add a host or tag");
    const tags = [...new Set(hosts.flatMap((h) => h.tags ?? []))].sort();
    picker.append(
      el("option", { value: "" }, "Add a host or tag…"),
      ...(tags.length ? [el("optgroup", { label: "Tags" }, ...tags.map((t) => el("option", { value: `tag:${t}` }, `tag:${t}`)))] : []),
      el("optgroup", { label: "Hosts" }, ...hosts.map((h) => el("option", { value: h.name }, h.name))),
    );
    picker.onchange = () => {
      const v = picker.value;
      picker.value = "";
      if (!v || targets.includes(v)) return;
      targets.push(v);
      draw();
      changed();
    };
    draw();
    return el("div", { className: "field" }, el("span", {}, "Hosts"), chips, picker, count);
  }

  private linkEditor(link: WorkflowLink) {
    const wf = this.current!;
    const name = (id: string) => wf.step.find((s) => s.id === id)?.name || id;
    const options = (Object.keys(WHEN_LABEL) as LinkWhen[]).map((w) => {
      const input = el("input", { type: "radio", name: "wf-when", value: w, checked: when(link) === w });
      input.onchange = () => {
        if (w === "success") delete link.when;
        else link.when = w;
        this.change();
      };
      const text = {
        success: `Run “${name(link.to)}” when “${name(link.from)}” succeeds`,
        failure: `Run “${name(link.to)}” only when “${name(link.from)}” fails (for alerts and rollbacks)`,
        always: `Run “${name(link.to)}” after “${name(link.from)}” either way`,
      }[w];
      return el("label", { className: "wf-when" }, input, el("span", { className: `wf-swatch ${w}` }), el("span", {}, el("strong", {}, WHEN_LABEL[w]), el("small", { className: "muted" }, text)));
    });
    const del = el("button", { type: "button", className: "danger with-icon" }, icon("trash", 14), "Delete link");
    del.onclick = () => this.deleteSelection();
    return [el("h3", {}, "Link"), el("p", { className: "muted" }, `${name(link.from)} → ${name(link.to)}`), ...options, el("div", { className: "wf-inspector-actions" }, del)];
  }

  // ---- Saving ----

  private async save(): Promise<boolean> {
    const wf = this.current;
    if (!wf || this.run) return false;
    try {
      const data = await workflowSave(wf, this.savedId);
      this.data = data;
      this.savedId = wf.id;
      this.dirty = false;
      this.error = "";
      this.render();
      return true;
    } catch (err) {
      this.error = `Could not save: ${errorText(err)}`;
      this.render();
      return false;
    }
  }

  private async remove() {
    const wf = this.current;
    if (!wf || !this.savedId) return;
    if (!(await confirmDialog("Delete workflow?", `“${wf.name}” is removed from your workflows folder. Its run history stays.`, "Delete", true, true))) return;
    try {
      this.data = await workflowDelete(this.savedId);
      this.current = null;
      this.savedId = null;
      this.dirty = false;
      if (this.data.workflows.length) this.open(this.data.workflows[0]);
    } catch (err) {
      this.error = errorText(err);
    }
    this.render();
  }

  // ---- Running ----

  /** Saves, shows what will run where, and starts the run. */
  private async start() {
    const wf = this.current;
    if (!wf) return;
    if (this.run?.live) return;
    this.run = null;
    if (this.dirty && !(await this.save())) return;
    const hosts = this.actions.hosts();
    const commands = wf.step.filter((s) => s.kind === "command");
    const problems = commands.flatMap((s) => {
      const { unknown } = expandTargets(s.targets ?? [], hosts);
      return unknown.length ? [`${s.name}: ${unknown.join(", ")} ${unknown.length === 1 ? "is" : "are"} not a saved host or tag`] : [];
    });
    if (problems.length) {
      this.error = problems.join(". ");
      this.render();
      return;
    }
    const all = new Set(commands.flatMap((s) => expandTargets(s.targets ?? [], hosts).found));
    const summary = el(
      "div",
      { className: "wf-confirm" },
      el("p", {}, `This runs ${commands.length} command step${commands.length === 1 ? "" : "s"} on ${all.size} host${all.size === 1 ? "" : "s"}, as you, over SSH:`),
      el(
        "ol",
        {},
        ...commands.map((s) =>
          el("li", {}, el("strong", {}, s.name), el("code", {}, s.command ?? ""), el("small", { className: "muted" }, expandTargets(s.targets ?? [], hosts).found.join(", "))),
        ),
      ),
      el("p", { className: "muted" }, "Hosts must already be trusted (connect once from Hosts). Passwords and passphrases come from the vault."),
    );
    if (!(await confirmDialog(`Run “${wf.name}”?`, summary, "Run", false, true))) return;
    const record: RunRecord = {
      id: "",
      workflow: wf.id,
      workflowName: wf.name,
      status: "running",
      startedMs: Date.now(),
      steps: wf.step.map((s) => ({ step: s.id, name: s.name, state: "pending" as StepState, hosts: [] })),
    };
    const run: LiveRun = { id: "", record, targets: {}, live: true };
    this.run = run;
    this.selection = null;
    this.error = "";
    this.render();
    try {
      const started = await workflowRun(wf.id, (e) => this.onEvent(run, e));
      run.id = started.runId;
      run.record.id = started.runId;
      run.targets = started.targets;
    } catch (err) {
      this.run = null;
      this.error = `Could not start: ${errorText(err)}`;
    }
    this.render();
  }

  private onEvent(run: LiveRun, e: RunEvent) {
    const rec = run.record;
    if (e.event === "finished") {
      rec.status = e.status;
      rec.finishedMs = Date.now();
      run.live = false;
      void workflowHistory().then((h) => {
        this.history = h;
        this.renderList();
      });
    } else {
      const s = rec.steps.find((x) => x.step === e.step);
      if (!s) return;
      if (e.event === "step") {
        if (e.state === "running" || e.state === "waiting") s.startedMs = Date.now();
        else if (s.startedMs) s.finishedMs = Date.now();
        s.state = e.state;
        if (e.note) s.note = e.note;
        if (e.state === "waiting" || (e.state === "failed" && !this.selection)) this.selection = { kind: "step", id: s.step };
      } else {
        const { event: _event, step: _step, ...host } = e;
        const hosts = (s.hosts ??= []);
        const i = hosts.findIndex((h) => h.host === host.host);
        if (i >= 0) hosts[i] = host;
        else hosts.push(host);
      }
    }
    if (this.run === run) {
      this.renderToolbar();
      this.renderBanner();
      this.renderCanvas();
      this.renderInspector();
    }
  }

  private async answer(step: string, approve: boolean) {
    if (!this.run?.id) return;
    try {
      await workflowAnswer(this.run.id, step, approve);
    } catch (err) {
      this.flash(errorText(err));
    }
  }

  private async stop() {
    if (!this.run?.id) return;
    await workflowCancel(this.run.id);
  }

  /** Shows a past run on the canvas (if the workflow still has those steps). */
  private async showRecord(r: RunRecord) {
    if (this.run?.live) return;
    if (this.dirty && this.current && this.current.id === r.workflow) {
      if (!(await this.leave())) return;
      const saved = this.data?.workflows.find((w) => w.id === r.workflow);
      if (saved) this.open(saved);
    }
    this.run = { id: r.id, record: clone(r), targets: {}, live: false };
    this.selection = null;
    const failed = r.steps.find((s) => s.state === "failed");
    if (failed) this.selection = { kind: "step", id: failed.step };
    this.render();
  }

  private runDetails(): HTMLElement[] {
    const run = this.run!;
    const sel = this.selection;
    const step = sel?.kind === "step" ? run.record.steps.find((s) => s.step === sel.id) : undefined;
    if (!step) {
      return [
        el("h3", {}, "Run"),
        el("p", { className: "muted" }, "Click a step to see what it printed on each host."),
        el(
          "ul",
          { className: "wf-run-steps" },
          ...run.record.steps.map((s) => {
            const li = el("li", {}, el("span", { className: `wf-dot ${s.state}` }), el("span", {}, s.name), el("small", { className: "muted" }, STATE_LABEL[s.state]));
            li.onclick = () => this.select({ kind: "step", id: s.step });
            return li;
          }),
        ),
      ];
    }
    const def = this.current?.step.find((s) => s.id === step.step);
    const parts: HTMLElement[] = [
      el("div", { className: "wf-inspector-head" }, el("span", { className: `wf-badge ${step.state}` }, STATE_LABEL[step.state]), el("h3", {}, step.name)),
    ];
    if (step.startedMs) {
      const end = step.finishedMs ?? (run.live ? Date.now() : step.startedMs);
      parts.push(el("small", { className: "muted" }, `${step.note ? step.note + " · " : ""}${duration(end - step.startedMs)}`));
    } else if (step.note) parts.push(el("small", { className: "muted" }, step.note));
    if (def?.kind === "command") parts.push(el("code", { className: "wf-run-command" }, def.command ?? ""));
    if (def?.kind === "approval" && def.message) parts.push(el("p", { className: "wf-approval-message" }, def.message));
    if (step.state === "waiting" && run.live) {
      const approve = el("button", { type: "button", className: "primary" }, "Approve");
      const reject = el("button", { type: "button", className: "danger" }, "Reject");
      approve.onclick = () => void this.answer(step.step, true);
      reject.onclick = () => void this.answer(step.step, false);
      parts.push(el("div", { className: "wf-inspector-actions" }, reject, approve));
    }
    for (const h of step.hosts ?? []) {
      const head = el(
        "div",
        { className: "wf-host-head" },
        el("span", { className: `wf-dot ${h.state}` }),
        el("strong", {}, h.host),
        el("small", { className: "muted" }, h.state === "running" ? "running…" : h.code !== undefined ? `exit ${h.code}` : STATE_LABEL[h.state]),
      );
      const out = el("pre", { className: "wf-output" });
      // Server output is untrusted: shown as text only.
      out.textContent = (h.output ?? "") + (h.error ? (h.output ? "\n" : "") + h.error : "") || (h.state === "running" ? "" : "(no output)");
      if (h.error) out.classList.add("has-error");
      parts.push(el("div", { className: "wf-host" }, head, out));
    }
    const back = el("button", { type: "button", className: "link-button" }, "All steps");
    back.onclick = () => this.select(null);
    parts.push(back);
    return parts;
  }
}

function curve(x1: number, y1: number, x2: number, y2: number) {
  const dx = Math.max(40, Math.abs(x2 - x1) / 2);
  return `M${x1} ${y1} C${x1 + dx} ${y1}, ${x2 - dx} ${y2}, ${x2} ${y2}`;
}

function relative(ms: number) {
  const s = Math.round((Date.now() - ms) / 1000);
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return new Date(ms).toLocaleDateString();
}
