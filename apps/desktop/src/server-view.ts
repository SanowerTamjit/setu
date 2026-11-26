// The Server view: a saved Linux host's systemd services, one service's
// status and logs, and changes to it. A change always shows the exact
// command first (asked from Rust, which builds the same command again when
// it runs) and waits for a yes. src-tauri/src/server.rs does the work.

import { confirmDialog, el, errorText } from "./dom";
import { icon } from "./icons";
import {
  serverDisconnect,
  serverServiceAction,
  serverServiceCommand,
  serverServiceDetails,
  serverServiceLogs,
  serverServices,
  type ActionResult,
  type Host,
  type LogLevel,
  type Service,
  type ServiceAction,
  type ServiceDetails,
} from "./ipc";
import { DiskPane, ProcessesPane, confirmBody, resultBox } from "./server-panes";
import { DEFAULT_LOG_LINES, LOG_LINES, actionsFor, confirmText, counts, facts, filterServices, serviceState, startupLabel, type ServiceFilter } from "./server-model";

export interface ServerActions {
  hosts: () => Host[];
  /** Opens the hosts view, for an empty inventory. */
  showHosts: () => void;
}

const PREFS = "setu.server";

type Tab = "services" | "processes" | "disk";
const TABS: [Tab, string][] = [
  ["services", "Services"],
  ["processes", "Processes"],
  ["disk", "Disk"],
];

interface Prefs {
  tab: Tab;
  host: string;
  sudo: boolean;
  lines: number;
  level: LogLevel;
}

function loadPrefs(): Prefs {
  const d: Prefs = { tab: "services", host: "", sudo: false, lines: DEFAULT_LOG_LINES, level: "all" };
  try {
    const p = JSON.parse(localStorage.getItem(PREFS) ?? "{}") as Partial<Record<keyof Prefs, unknown>>;
    return {
      tab: TABS.some(([t]) => t === p.tab) ? (p.tab as Tab) : d.tab,
      host: typeof p.host === "string" ? p.host : d.host,
      sudo: typeof p.sudo === "boolean" ? p.sudo : d.sudo,
      lines: LOG_LINES.includes(Number(p.lines) as (typeof LOG_LINES)[number]) ? Number(p.lines) : d.lines,
      level: p.level === "warnings" || p.level === "errors" ? p.level : "all",
    };
  } catch {
    return d;
  }
}

export class ServerView {
  private prefs = loadPrefs();
  private readonly hostSelect: HTMLSelectElement;
  private readonly sudoBox: HTMLInputElement;
  private readonly search: HTMLInputElement;
  private readonly filters = new Map<ServiceFilter, HTMLButtonElement>();
  private readonly list: HTMLElement;
  private readonly listNote: HTMLElement;
  private readonly detail: HTMLElement;
  private readonly body: HTMLElement;
  private readonly stage: HTMLElement;
  private readonly tabButtons = new Map<Tab, HTMLButtonElement>();
  private readonly processes: ProcessesPane;
  private readonly disk: DiskPane;
  /** The host the panes show; another one in the picker means start again. */
  private shownHost = "";
  private filter: ServiceFilter = "all";
  private services: Service[] = [];
  private selected: string | null = null;
  /** Bumped on every load, so a slow answer for an old host is dropped. */
  private generation = 0;
  private loadedHost = "";

  constructor(
    private readonly root: HTMLElement,
    private readonly actions: ServerActions,
  ) {
    this.hostSelect = el("select", { className: "net-from-select" });
    this.hostSelect.setAttribute("aria-label", "Server");
    this.hostSelect.onchange = () => {
      this.save({ host: this.hostSelect.value });
      this.hostChanged();
    };
    this.sudoBox = el("input", { type: "checkbox", checked: this.prefs.sudo });
    this.sudoBox.onchange = () => {
      this.save({ sudo: this.sudoBox.checked });
      if (this.selected) void this.select(this.selected);
    };
    const sudo = el(
      "label",
      { className: "srv-sudo muted", title: "Run reads and changes with sudo -n. Setu never types a password into sudo, so this works where your user may use sudo without one." },
      this.sudoBox,
      el("span", {}, "Use sudo"),
    );
    const reload = el("button", { type: "button", className: "card-icon", title: "Read again" }, icon("retry", 15));
    reload.setAttribute("aria-label", "Read again");
    reload.onclick = () => {
      if (this.prefs.tab === "processes") void this.processes.load(true);
      else if (this.prefs.tab === "disk") void this.disk.load(true);
      else void this.load();
    };
    const ctx = { host: () => this.host, sudo: () => this.sudoBox.checked };
    this.processes = new ProcessesPane(ctx);
    this.disk = new DiskPane(ctx);
    const tabs = el("div", { className: "srv-tabs", role: "tablist" });
    tabs.setAttribute("aria-label", "Server");
    for (const [t, label] of TABS) {
      const b = el("button", { type: "button", className: "srv-tab", role: "tab" }, label);
      b.onclick = () => this.showTab(t);
      this.tabButtons.set(t, b);
      tabs.append(b);
    }
    this.search = el("input", { type: "search", className: "srv-search", placeholder: "Filter services", spellcheck: false, autocomplete: "off" });
    this.search.oninput = () => this.renderList();
    const chips = el("div", { className: "srv-filters", role: "group" });
    chips.setAttribute("aria-label", "Show");
    for (const f of ["all", "running", "failed", "stopped"] as const) {
      const b = el("button", { type: "button", className: "srv-chip" });
      b.onclick = () => {
        this.filter = f;
        this.renderList();
      };
      this.filters.set(f, b);
      chips.append(b);
    }
    this.list = el("div", { className: "srv-list", role: "listbox" });
    this.list.setAttribute("aria-label", "Services");
    this.list.onkeydown = (e) => this.onListKey(e);
    this.listNote = el("p", { className: "muted srv-list-note", hidden: true });
    this.detail = el("section", { className: "host-card srv-detail" });
    this.stage = el("div", { className: "srv-stage" });
    this.body = el(
      "div",
      { className: "srv-body" },
      el("div", { className: "host-card srv-side" }, this.search, chips, this.list, this.listNote),
      this.detail,
    );
    this.root.replaceChildren(
      el(
        "div",
        { className: "home-inner srv-inner" },
        el(
          "header",
          { className: "home-header" },
          el("div", { className: "home-title" }, el("h1", {}, "Server")),
          el("div", { className: "home-actions srv-actions" }, el("label", { className: "net-from" }, el("span", { className: "muted" }, "Host"), this.hostSelect), sudo, reload),
        ),
        el(
          "p",
          { className: "muted tunnel-intro" },
          "A Linux server's services (systemd) with their logs, its processes and what fills its disks. Every change shows the exact command and asks first. Setu logs in the way the host's tunnels and monitoring do.",
        ),
        tabs,
        this.stage,
      ),
    );
    this.renderDetail(null);
    this.markTab();
  }

  focus() {
    (this.services.length ? this.search : this.hostSelect).focus();
  }

  /** Fills the host list; loads services when the host changed. */
  refresh() {
    const names = this.actions.hosts().map((h) => h.name);
    const keep = names.includes(this.hostSelect.value) ? this.hostSelect.value : names.includes(this.prefs.host) ? this.prefs.host : (names[0] ?? "");
    this.hostSelect.replaceChildren(...names.map((n) => el("option", { value: n }, n)));
    this.hostSelect.value = keep;
    this.hostSelect.disabled = !names.length;
    if (!names.length) {
      this.loadedHost = "";
      this.shownHost = "";
      this.services = [];
      this.renderEmpty();
      return;
    }
    if (keep !== this.shownHost) this.hostChanged();
  }

  /** Starts again for the host in the picker, on the open tab. */
  private hostChanged() {
    const host = this.host;
    if (this.shownHost && this.shownHost !== host) void serverDisconnect(this.shownHost).catch(() => {});
    this.shownHost = host;
    this.processes.reset();
    this.disk.reset();
    this.showTab(this.prefs.tab);
  }

  private markTab() {
    for (const [t, b] of this.tabButtons) {
      b.classList.toggle("current", t === this.prefs.tab);
      b.setAttribute("aria-selected", String(t === this.prefs.tab));
    }
  }

  private showTab(t: Tab) {
    this.save({ tab: t });
    this.markTab();
    if (!this.host) return;
    if (t === "processes") {
      this.stage.replaceChildren(this.processes.root);
      void this.processes.load();
    } else if (t === "disk") {
      this.stage.replaceChildren(this.disk.root);
      void this.disk.load();
    } else {
      this.stage.replaceChildren(this.body);
      if (this.loadedHost !== this.host) void this.load();
    }
  }

  private save(p: Partial<Prefs>) {
    this.prefs = { ...this.prefs, ...p };
    try {
      localStorage.setItem(PREFS, JSON.stringify(this.prefs));
    } catch {
      // Storage off: the choice lasts until Setu closes.
    }
  }

  private get host(): string {
    return this.hostSelect.value;
  }

  private renderEmpty() {
    const add = el("button", { type: "button", className: "primary" }, "Add a host");
    add.onclick = () => this.actions.showHosts();
    this.stage.replaceChildren(el("div", { className: "home-empty" }, el("p", {}, "Save a host first: the Server view manages saved hosts."), add));
  }

  private async load() {
    const host = this.host;
    if (!host) return;
    this.loadedHost = host;
    const gen = ++this.generation;
    this.services = [];
    this.list.replaceChildren(el("p", { className: "muted srv-loading" }, `Reading services on ${host}…`));
    this.renderDetail(null);
    try {
      const r = await serverServices(host);
      if (gen !== this.generation) return;
      this.services = r.services;
      this.listNote.hidden = !r.truncated;
      this.listNote.textContent = r.truncated ? "This server has more services than Setu lists; filter to find one." : "";
      if (this.selected && !this.services.some((s) => s.name === this.selected)) this.selected = null;
      this.renderList();
      if (this.selected) void this.select(this.selected);
      else this.renderDetail(null);
    } catch (err) {
      if (gen !== this.generation) return;
      this.list.replaceChildren(el("p", { className: "srv-problem" }, errorText(err)));
    }
  }

  private renderList() {
    const c = counts(this.services);
    const words: Record<ServiceFilter, string> = { all: "All", running: "Running", failed: "Failed", stopped: "Stopped" };
    for (const [f, b] of this.filters) {
      b.textContent = `${words[f]} ${c[f]}`;
      b.classList.toggle("current", f === this.filter);
      b.classList.toggle("srv-chip-failed", f === "failed" && c.failed > 0);
      b.setAttribute("aria-pressed", String(f === this.filter));
    }
    const shown = filterServices(this.services, this.search.value, this.filter);
    if (!shown.length) {
      this.list.replaceChildren(el("p", { className: "muted srv-loading" }, this.services.length ? "No service matches." : "No services."));
      return;
    }
    this.list.replaceChildren(
      ...shown.map((s) => {
        const st = serviceState(s);
        const row = el(
          "button",
          { type: "button", className: `srv-row${s.name === this.selected ? " current" : ""}`, role: "option" },
          el("span", { className: `srv-dot state-${st.kind}`, title: st.label }),
          el("span", { className: "srv-row-text" }, el("span", { className: "srv-row-name" }, s.name), el("span", { className: "srv-row-desc muted" }, s.description || startupLabel(s.startup))),
          el("span", { className: `srv-state state-${st.kind}` }, st.label),
        );
        row.setAttribute("aria-selected", String(s.name === this.selected));
        row.dataset.unit = s.name;
        row.onclick = () => void this.select(s.name);
        return row;
      }),
    );
  }

  private onListKey(e: KeyboardEvent) {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    const rows = [...this.list.querySelectorAll<HTMLButtonElement>(".srv-row")];
    const i = rows.indexOf(document.activeElement as HTMLButtonElement);
    const next = rows[Math.max(0, Math.min(rows.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)))];
    if (next) {
      e.preventDefault();
      next.focus();
      void this.select(next.dataset.unit!);
    }
  }

  private async select(unit: string) {
    this.selected = unit;
    for (const r of this.list.querySelectorAll<HTMLElement>(".srv-row")) {
      const on = r.dataset.unit === unit;
      r.classList.toggle("current", on);
      r.setAttribute("aria-selected", String(on));
    }
    const host = this.host;
    const gen = this.generation;
    const listed = this.services.find((s) => s.name === unit);
    this.renderDetail(listed ? { ...listed, mainPid: null, since: null, memoryBytes: null, tasks: null, restarts: null, path: null } : null, true);
    try {
      const d = await serverServiceDetails(host, unit, this.sudoBox.checked);
      if (gen !== this.generation || this.selected !== unit) return;
      this.updateListed(d);
      this.renderDetail(d);
    } catch (err) {
      if (gen !== this.generation || this.selected !== unit) return;
      this.renderDetail(listed ? { ...listed, mainPid: null, since: null, memoryBytes: null, tasks: null, restarts: null, path: null } : null, false, errorText(err));
    }
  }

  /** Keeps the list row in step with a fresher status. */
  private updateListed(d: ServiceDetails) {
    const i = this.services.findIndex((s) => s.name === d.name);
    if (i < 0) return;
    const s = this.services[i];
    if (s.active === d.active && s.sub === d.sub && s.startup === d.startup) return;
    this.services[i] = { ...s, active: d.active || s.active, sub: d.sub || s.sub, startup: d.startup || s.startup };
    this.renderList();
  }

  private renderDetail(d: ServiceDetails | null, loading = false, problem?: string, result?: ActionResult) {
    if (!d) {
      this.detail.replaceChildren(el("div", { className: "srv-placeholder muted" }, icon("server", 28), el("p", {}, this.services.length ? "Pick a service to see its status and logs." : "")));
      return;
    }
    const st = serviceState(d);
    const buttons = actionsFor(d).map((a) => {
      const b = el("button", { type: "button", className: a.danger ? "srv-action risky" : "srv-action", disabled: !a.available || loading }, a.label);
      b.onclick = () => void this.change(d, a.action);
      return b;
    });
    const factList = el("dl", { className: "srv-facts" }, ...facts(d).flatMap(([k, v]) => [el("dt", {}, k), el("dd", {}, v)]));
    const logs = el("pre", { className: "srv-logs", tabIndex: 0 }, loading ? "" : "Reading logs…");
    logs.setAttribute("aria-label", `Logs of ${d.name}`);
    const ran = el("p", { className: "srv-ran muted" });
    const lines = el("select", { className: "srv-select" }, ...LOG_LINES.map((n) => el("option", { value: String(n), selected: n === this.prefs.lines }, `Last ${n}`)));
    lines.setAttribute("aria-label", "Lines");
    const level = el(
      "select",
      { className: "srv-select" },
      ...(
        [
          ["all", "All lines"],
          ["warnings", "Warnings and errors"],
          ["errors", "Errors only"],
        ] as const
      ).map(([v, t]) => el("option", { value: v, selected: v === this.prefs.level }, t)),
    );
    level.setAttribute("aria-label", "Level");
    const again = el("button", { type: "button", className: "card-icon", title: "Read the logs again" }, icon("retry", 14));
    again.setAttribute("aria-label", "Read the logs again");
    const readLogs = () => void this.readLogs(d.name, logs, ran);
    lines.onchange = () => {
      this.save({ lines: Number(lines.value) });
      readLogs();
    };
    level.onchange = () => {
      this.save({ level: level.value as LogLevel });
      readLogs();
    };
    again.onclick = readLogs;
    this.detail.replaceChildren(
      el(
        "div",
        { className: "card-top" },
        el("span", { className: "avatar tunnel-kind" }, icon("server", 18)),
        el("div", { className: "card-heading" }, el("h2", { className: "card-name" }, d.name), el("span", { className: "card-address" }, d.description)),
        el("span", { className: `srv-state big state-${st.kind}` }, loading ? "…" : st.label),
      ),
      ...(problem ? [el("p", { className: "srv-problem" }, problem)] : []),
      ...(result ? [resultBox(result)] : []),
      el("div", { className: "srv-buttons" }, ...buttons),
      factList,
      el("div", { className: "srv-logs-head" }, el("h3", {}, "Logs"), el("div", { className: "srv-logs-tools" }, lines, level, again)),
      logs,
      ran,
    );
    if (!loading) readLogs();
  }

  private async readLogs(unit: string, out: HTMLElement, ran: HTMLElement) {
    const host = this.host;
    out.textContent = "Reading logs…";
    out.classList.remove("srv-logs-problem");
    try {
      const r = await serverServiceLogs(host, unit, this.prefs.lines, this.prefs.level, this.sudoBox.checked);
      if (this.selected !== unit || this.host !== host) return;
      out.textContent = r.text.trim() ? r.text : "No log lines.";
      ran.textContent = `${r.truncated ? "Older lines left out. " : ""}Ran: ${r.command}`;
      out.scrollTop = out.scrollHeight;
    } catch (err) {
      if (this.selected !== unit) return;
      out.textContent = errorText(err);
      out.classList.add("srv-logs-problem");
    }
  }

  /** Shows the exact command, asks, then runs it and shows what happened. */
  private async change(d: ServiceDetails, action: ServiceAction) {
    const host = this.host;
    const sudo = this.sudoBox.checked;
    let command: string;
    try {
      command = await serverServiceCommand(action, d.name, sudo);
    } catch (err) {
      this.renderDetail(d, false, errorText(err));
      return;
    }
    const q = confirmText(action, d.name, host);
    const body = confirmBody(q.what, host, command, sudo, q.warning);
    const danger = action === "stop" || action === "disable" || q.warning !== null;
    if (!(await confirmDialog(q.title, body, q.button, danger, danger))) return;
    this.renderDetail(d, true);
    try {
      const r = await serverServiceAction(host, action, d.name, sudo);
      if (this.host !== host || this.selected !== d.name) return;
      let fresh = d;
      try {
        fresh = await serverServiceDetails(host, d.name, sudo);
        this.updateListed(fresh);
      } catch {
        // Show the result with the status we had.
      }
      this.renderDetail(fresh, false, undefined, r);
    } catch (err) {
      this.renderDetail(d, false, errorText(err));
    }
  }
}
