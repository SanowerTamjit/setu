// The Monitoring view: live CPU, memory, network and disk use for saved
// hosts, read over SSH with nothing installed on the server. Each watched
// host is a panel fed by a channel from src-tauri/src/monitor.rs; it keeps
// running while other views are open, until its panel is closed.

import { el, errorText } from "./dom";
import { icon } from "./icons";
import { monitorStart, monitorStop, type ActiveAlert, type Host, type MonitorEvent, type MonitorLimits, type MonitorOptions, type Snapshot } from "./ipc";
import { DEFAULT_HOLD, DETAILS, HOLDS, History, HourHistory, activeText, chartMinutes, durationWords, holdLabel, kb, level, parseLimit, pct, procCpu, rate, seriesPath, sparkline, uptime, type Detail } from "./monitor-model";

export interface MonitorActions {
  hosts: () => Host[];
  /** Opens the hosts view, for an empty inventory. */
  showHosts: () => void;
}

const SVG = "http://www.w3.org/2000/svg";
const PREFS = "setu.monitor";
const W = 220;
const H = 44;

interface Prefs extends MonitorOptions {
  interval: number;
}

function loadPrefs(): Prefs {
  const d: Prefs = { cpu: 90, mem: 90, disk: 90, interval: 2, hold: DEFAULT_HOLD, notify: true };
  try {
    const p = JSON.parse(localStorage.getItem(PREFS) ?? "{}") as Partial<Record<keyof Prefs, unknown>>;
    const n = (v: unknown, f: number) => parseLimit(String(v ?? ""), f);
    return {
      cpu: n(p.cpu, d.cpu),
      mem: n(p.mem, d.mem),
      disk: n(p.disk, d.disk),
      interval: [2, 5, 10, 30].includes(Number(p.interval)) ? Number(p.interval) : 2,
      hold: HOLDS.includes(Number(p.hold)) ? Number(p.hold) : d.hold,
      notify: typeof p.notify === "boolean" ? p.notify : d.notify,
    };
  } catch {
    return d;
  }
}

function savePrefs(p: Prefs) {
  try {
    localStorage.setItem(PREFS, JSON.stringify(p));
  } catch {
    // Private mode or storage off: the choice lasts until Setu closes.
  }
}

function svgEl(tag: string, attrs: Record<string, string>) {
  const node = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, v);
  return node;
}

class Panel {
  readonly root: HTMLElement;
  private id: string | null = null;
  private readonly history = new History();
  private readonly hour = new HourHistory();
  private active: ActiveAlert[] = [];
  private readonly status: HTMLElement;
  private readonly sub: HTMLElement;
  private readonly alerts: HTMLElement;
  private readonly body: HTMLElement;
  private readonly message: HTMLElement;
  private last: Snapshot | null = null;
  private detail: Detail | null = null;

  constructor(
    readonly host: Host,
    private readonly prefs: () => Prefs,
    private readonly onClose: (p: Panel) => void,
    private readonly onChange: () => void,
  ) {
    this.status = el("span", { className: "tunnel-phase phase-connecting" }, "Connecting…");
    this.sub = el("span", { className: "card-address" }, `${host.user ? `${host.user}@` : ""}${host.host}`);
    const close = el("button", { type: "button", className: "card-icon", title: `Stop watching ${host.name}` }, icon("close", 15));
    close.setAttribute("aria-label", `Stop watching ${host.name}`);
    close.onclick = () => this.close();
    this.alerts = el("div", { className: "mon-alerts", hidden: true });
    this.alerts.setAttribute("role", "status");
    this.message = el("p", { className: "tunnel-message", hidden: true });
    this.body = el("div", { className: "mon-body" });
    this.root = el(
      "section",
      { className: "host-card mon-panel" },
      el(
        "div",
        { className: "card-top" },
        el("span", { className: "avatar tunnel-kind" }, icon("pulse", 18)),
        el("div", { className: "card-heading" }, el("h2", { className: "card-name" }, host.name), this.sub),
        this.status,
        close,
      ),
      this.alerts,
      this.message,
      this.body,
    );
    this.root.setAttribute("aria-label", `Monitoring ${host.name}`);
    this.renderBody(null);
  }

  async start() {
    const p = this.prefs();
    try {
      this.id = await monitorStart(this.host.name, p.interval, p, (e) => this.event(e));
    } catch (err) {
      this.event({ kind: "problem", message: errorText(err), stopped: true });
    }
  }

  /** Starts again with the current interval and limits. */
  restart() {
    if (this.id) void monitorStop(this.id);
    this.id = null;
    void this.start();
  }

  /** Alerts that have lasted the hold time. */
  get alertCount() {
    return this.active.filter((a) => a.firing).length;
  }

  close() {
    if (this.id) void monitorStop(this.id);
    this.id = null;
    this.onClose(this);
  }

  private setStatus(phase: string, text: string) {
    this.status.className = `tunnel-phase phase-${phase}`;
    this.status.textContent = text;
  }

  private event(e: MonitorEvent) {
    switch (e.kind) {
      case "connecting":
        this.setStatus("connecting", "Connecting…");
        break;
      case "sample":
        this.message.hidden = true;
        this.setStatus("running", "Live");
        this.last = e.snapshot;
        this.active = e.active;
        this.history.add(e.snapshot);
        this.hour.add(Date.now(), e.snapshot);
        this.renderBody(e.snapshot);
        break;
      case "problem":
        {
          const said = e.message.replace(/[.\s]+$/, "");
          this.message.textContent = e.stopped ? `${said}. Close this panel and watch the host again to retry.` : `${said}. Trying again…`;
        }
        if (e.stopped && !this.last) this.body.hidden = true;
        this.message.className = `tunnel-message phase-${e.stopped ? "failed" : "reconnecting"}`;
        this.message.hidden = false;
        this.setStatus(e.stopped ? "failed" : "reconnecting", e.stopped ? "Stopped" : "Reconnecting…");
        if (e.stopped) this.id = null;
        break;
      case "stopped":
        this.setStatus("stopped", "Stopped");
        this.id = null;
        break;
    }
    this.onChange();
  }

  private tile(title: string, value: string, lvl: string, foot: string, lines: { values: number[]; max?: number; cls: string }[]) {
    const svg = svgEl("svg", { viewBox: `0 0 ${W} ${H}`, preserveAspectRatio: "none", class: "mon-spark", "aria-hidden": "true" });
    for (const l of lines) {
      const d = sparkline(l.values, W, H, l.max);
      if (d) svg.append(svgEl("path", { d, class: l.cls, "vector-effect": "non-scaling-stroke" }));
    }
    return el(
      "div",
      { className: `mon-tile level-${lvl}` },
      el("div", { className: "mon-tile-head" }, el("span", { className: "muted" }, title), el("strong", { className: "mon-value" }, value)),
      svg as unknown as HTMLElement,
      el("small", { className: "muted mon-foot" }, foot),
    );
  }

  private renderBody(s: Snapshot | null) {
    const p = this.prefs();
    if (s) {
      this.sub.textContent = [`${this.host.user ? `${this.host.user}@` : ""}${this.host.host}`, uptime(s.uptimeSecs)].filter(Boolean).join(" · ");
      this.alerts.hidden = this.active.length === 0;
      this.alerts.replaceChildren(
        ...this.active.map((a) =>
          el(
            "span",
            {
              className: `mon-alert${a.firing ? "" : " pending"}`,
              title: a.firing ? `Over the limit for ${durationWords(a.secs)}` : `Over the limit; alerts once it has lasted ${durationWords(p.hold)}`,
            },
            icon("spark", 12),
            activeText(a, p.hold),
          ),
        ),
      );
    }
    const load = s?.load ? `load ${s.load.map((l) => l.toFixed(2)).join(" ")}` : "";
    const cpu = this.tile("CPU", pct(s?.cpuPercent), level(s?.cpuPercent, p.cpu), [s ? `${s.cpus} CPU${s.cpus === 1 ? "" : "s"}` : "", load].filter(Boolean).join(" · ") || "Waiting for figures…", [
      { values: this.history.cpu, max: 100, cls: "mon-line" },
    ]);
    const mem = this.tile(
      "Memory",
      pct(s?.memPercent),
      level(s?.memPercent, p.mem),
      s && s.memTotalKb ? `${kb(s.memUsedKb)} of ${kb(s.memTotalKb)}${s.swapPercent != null ? ` · swap ${pct(s.swapPercent)}` : ""}` : "…",
      [{ values: this.history.mem, max: 100, cls: "mon-line" }],
    );
    const top = Math.max(1, ...this.history.rx, ...this.history.tx);
    const net = this.tile("Network", `↓ ${rate(s?.netRxPerSec)}`, "ok", `↑ ${rate(s?.netTxPerSec)} sent · all interfaces but loopback`, [
      { values: this.history.rx, max: top, cls: "mon-line" },
      { values: this.history.tx, max: top, cls: "mon-line second" },
    ]);
    const disks = s?.disks.length
      ? el(
          "div",
          { className: "mon-disks" },
          el("span", { className: "muted mon-disks-title" }, "Disks"),
          ...s.disks.map((d) => {
            const bar = el("span", { className: "mon-bar-fill" });
            bar.style.width = `${Math.min(100, Math.max(0, d.percent)).toFixed(1)}%`;
            const row = el(
              "div",
              { className: `mon-disk level-${level(d.percent, p.disk)}`, title: d.mount },
              el("code", { className: "mon-mount" }, d.mount),
              el("span", { className: "mon-bar" }, bar),
              el("span", { className: "mon-disk-use" }, `${kb(d.usedKb)} of ${kb(d.totalKb)} · ${pct(d.percent)}`),
            );
            return row;
          }),
        )
      : el("span", {});
    this.body.replaceChildren(el("div", { className: "mon-tiles" }, cpu, mem, net), disks, ...(s ? [this.details(s)] : []));
  }

  /** Tabs for a closer look: processes, each core, each interface, disk I/O. */
  /** CPU and memory over the last hour, with the alert limits. */
  private hourChart(p: Prefs) {
    const CW = 600;
    const CH = 140;
    const now = Date.now();
    const kept = this.hour.span(now);
    const minutes = chartMinutes(kept);
    const { cpu, mem } = this.hour.buckets(now, 120, minutes * 60_000);
    const svg = svgEl("svg", { viewBox: `0 0 ${CW} ${CH}`, preserveAspectRatio: "none", class: "mon-hour-chart", role: "img" });
    svg.setAttribute("aria-label", `CPU and memory over the last ${minutes} minutes for ${this.host.name}`);
    for (const y of [0.25, 0.5, 0.75]) svg.append(svgEl("path", { d: `M0 ${CH * y} H${CW}`, class: "mon-grid", "vector-effect": "non-scaling-stroke" }));
    const limitAt = (v: number) => CH - (v / 100) * CH;
    svg.append(svgEl("path", { d: `M0 ${limitAt(p.cpu)} H${CW}`, class: "mon-limit-line", "vector-effect": "non-scaling-stroke" }));
    if (p.mem !== p.cpu) svg.append(svgEl("path", { d: `M0 ${limitAt(p.mem)} H${CW}`, class: "mon-limit-line second", "vector-effect": "non-scaling-stroke" }));
    for (const [values, cls] of [
      [mem, "mon-line second"],
      [cpu, "mon-line"],
    ] as const) {
      const d = seriesPath(values, CW, CH, 100);
      if (d) svg.append(svgEl("path", { d, class: cls, "vector-effect": "non-scaling-stroke" }));
    }
    const note =
      minutes < 60
        ? `Kept while this panel is open: ${durationWords(kept / 1000)} so far, up to an hour. Setu does not save it.`
        : "Each point is the highest reading in 30 seconds, so short spikes still show. Setu does not save it.";
    return el(
      "div",
      { className: "mon-hour" },
      el(
        "div",
        { className: "mon-hour-legend" },
        el("span", { className: "mon-key" }, el("i", { className: "mon-key-line" }), "CPU"),
        el("span", { className: "mon-key" }, el("i", { className: "mon-key-line second" }), "Memory"),
        el("span", { className: "mon-key" }, el("i", { className: "mon-key-limit" }), `Alert limits (CPU ${p.cpu}%${p.mem !== p.cpu ? `, memory ${p.mem}%` : ""})`),
      ),
      el(
        "div",
        { className: "mon-hour-plot" },
        el("div", { className: "mon-hour-y muted" }, el("span", {}, "100%"), el("span", {}, "50%"), el("span", {}, "0%")),
        svg as unknown as HTMLElement,
      ),
      el("div", { className: "mon-hour-x muted" }, el("span", {}, `${minutes} min ago`), el("span", {}, `${minutes / 2} min ago`), el("span", {}, "now")),
      el("small", { className: "muted" }, note),
    );
  }

  private details(s: Snapshot) {
    const p = this.prefs();
    const tabs = el("div", { className: "mon-detail-tabs", role: "tablist" });
    tabs.setAttribute("aria-label", `Details for ${this.host.name}`);
    for (const d of DETAILS) {
      const b = el("button", { type: "button", className: `mon-detail-tab${this.detail === d.id ? " current" : ""}` }, d.label);
      b.setAttribute("role", "tab");
      b.setAttribute("aria-selected", String(this.detail === d.id));
      b.onclick = () => {
        this.detail = this.detail === d.id ? null : d.id;
        this.renderBody(this.last);
      };
      tabs.append(b);
    }
    // Text columns (names, commands) line up on the left, numbers on the right,
    // and each header follows its column.
    const table = (head: string[], rows: (string | HTMLElement)[][], cls = "") => {
      const text = head.map((_, i) => rows.length > 0 && typeof rows[0][i] !== "string");
      const align = (i: number) => (text[i] || i === 0 ? "" : "num");
      return el(
        "table",
        { className: `mon-table ${cls}` },
        el("thead", {}, el("tr", {}, ...head.map((h, i) => el("th", { className: align(i) }, h)))),
        el("tbody", {}, ...rows.map((r) => el("tr", {}, ...r.map((c, i) => el("td", { className: align(i) }, c))))),
      );
    };
    let content: HTMLElement | null = null;
    switch (this.detail) {
      case "hour":
        content = this.hourChart(p);
        break;
      case "processes":
        content = el(
          "div",
          {},
          table(
            ["PID", "Command", "CPU", "Memory", ""],
            s.processes.map((q) => [String(q.pid), el("code", {}, q.command), procCpu(q.cpuPercent), kb(q.rssKb), pct(q.memPercent)]),
            "mon-procs",
          ),
          el("small", { className: "muted" }, `The ${s.processes.length} busiest of ${s.processCount} processes. CPU is like top: 100% is one whole core.`),
        );
        break;
      case "cores":
        content = s.corePercents.length
          ? el(
              "div",
              { className: "mon-cores" },
              ...s.corePercents.map((v, i) => {
                const fill = el("span", { className: "mon-core-fill" });
                fill.style.height = `${Math.min(100, Math.max(0, v)).toFixed(0)}%`;
                const core = el("div", { className: `mon-core level-${level(v, p.cpu)}`, title: `CPU ${i}: ${pct(v)}` }, el("span", { className: "mon-core-bar" }, fill), el("small", {}, String(i)));
                core.setAttribute("aria-label", `CPU ${i} at ${pct(v)}`);
                return core;
              }),
            )
          : el("small", { className: "muted" }, "Waiting for a second reading…");
        break;
      case "network":
        content = table(
          ["Interface", "Received", "Sent"],
          s.interfaces.map((i) => [el("code", {}, i.name), rate(i.rxPerSec), rate(i.txPerSec)]),
        );
        break;
      case "disks":
        content = s.diskIo.length
          ? table(
              ["Device", "Read", "Written"],
              s.diskIo.map((d) => [el("code", {}, d.device), rate(d.readPerSec), rate(d.writePerSec)]),
            )
          : el("small", { className: "muted" }, "This server does not report disk activity (often the case in containers).");
        break;
    }
    return el("div", { className: "mon-details" }, tabs, ...(content ? [el("div", { className: "mon-detail" }, content)] : []));
  }
}

export class MonitorView {
  private readonly panels: Panel[] = [];
  private readonly list: HTMLDivElement;
  private readonly count: HTMLSpanElement;
  private readonly pick: HTMLSelectElement;
  private readonly watch: HTMLButtonElement;
  private prefs: Prefs = loadPrefs();

  constructor(
    private readonly root: HTMLElement,
    private readonly actions: MonitorActions,
  ) {
    this.count = el("span", { className: "home-count" });
    this.pick = el("select", { className: "net-from-select" });
    this.pick.setAttribute("aria-label", "Host to watch");
    this.watch = el("button", { type: "button", className: "primary with-icon" }, icon("plus", 15), "Watch");
    this.watch.onclick = () => this.add(this.pick.value);
    this.list = el("div", { className: "mon-list" });

    const every = el("select", {}, ...[2, 5, 10, 30].map((n) => el("option", { value: String(n), selected: n === this.prefs.interval }, `${n} s`)));
    every.onchange = () => this.setPrefs({ interval: Number(every.value) });
    const limit = (key: keyof MonitorLimits, label: string) => {
      const input = el("input", { value: String(this.prefs[key]), inputMode: "numeric", className: "mon-limit" });
      input.setAttribute("aria-label", `${label} alert above, percent`);
      input.onchange = () => {
        const v = parseLimit(input.value, this.prefs[key]);
        input.value = String(v);
        this.setPrefs({ [key]: v } as Partial<Prefs>);
      };
      return el("label", { className: "mon-setting" }, el("span", { className: "muted" }, label), input, el("span", { className: "muted" }, "%"));
    };
    const hold = el("select", {}, ...HOLDS.map((n) => el("option", { value: String(n), selected: n === this.prefs.hold }, holdLabel(n))));
    hold.setAttribute("aria-label", "Alert once a reading has stayed over its limit");
    hold.title = "A reading must stay over its limit this long before it alerts, so short spikes do not.";
    hold.onchange = () => this.setPrefs({ hold: Number(hold.value) });
    const notifyBox = el("input", { type: "checkbox", checked: this.prefs.notify });
    notifyBox.onchange = () => this.setPrefs({ notify: notifyBox.checked });
    const notify = el(
      "label",
      { className: "mon-setting mon-notify", title: "Show a desktop notification when an alert starts and when it clears." },
      notifyBox,
      el("span", {}, "Notify me"),
    );
    const settings = el(
      "div",
      { className: "mon-settings" },
      el("label", { className: "mon-setting" }, el("span", { className: "muted" }, "Every"), every),
      el("span", { className: "muted" }, "Alert above:"),
      limit("cpu", "CPU"),
      limit("mem", "Memory"),
      limit("disk", "Disk"),
      hold,
      notify,
    );

    this.root.replaceChildren(
      el(
        "div",
        { className: "home-inner" },
        el(
          "header",
          { className: "home-header" },
          el("div", { className: "home-title" }, el("h1", {}, "Monitoring"), this.count),
          el("div", { className: "home-actions net-from" }, this.pick, this.watch),
        ),
        el(
          "p",
          { className: "muted tunnel-intro" },
          "Live CPU, memory, network and disk use for your servers. Setu reads Linux's own figures over SSH, so nothing is installed on the server. Panels keep updating while you work in other views.",
        ),
        settings,
        this.list,
      ),
    );
  }

  focus() {
    (this.pick.options.length ? this.pick : this.watch).focus();
  }

  /** Fills the host picker and shows the empty state if nothing is watched. */
  refresh() {
    const keep = this.pick.value;
    const names = this.actions.hosts().map((h) => h.name);
    this.pick.replaceChildren(...names.map((n) => el("option", { value: n }, n)));
    if (names.includes(keep)) this.pick.value = keep;
    this.pick.disabled = names.length === 0;
    this.render();
  }

  /** Starts watching a saved host, or brings its panel into view. */
  add(name: string) {
    const host = this.actions.hosts().find((h) => h.name === name);
    if (!host) return;
    const open = this.panels.find((p) => p.host.name === name);
    if (open) {
      open.root.scrollIntoView({ block: "nearest" });
      return;
    }
    const panel = new Panel(
      host,
      () => this.prefs,
      (p) => {
        this.panels.splice(this.panels.indexOf(p), 1);
        this.render();
      },
      () => this.updateCount(),
    );
    this.panels.push(panel);
    this.render();
    void panel.start();
  }

  private setPrefs(change: Partial<Prefs>) {
    this.prefs = { ...this.prefs, ...change };
    savePrefs(this.prefs);
    for (const p of this.panels) p.restart();
  }

  private updateCount() {
    const alerts = this.panels.reduce((n, p) => n + p.alertCount, 0);
    this.count.textContent = this.panels.length
      ? `${this.panels.length} watched${alerts ? ` · ${alerts} alert${alerts === 1 ? "" : "s"}` : ""}`
      : "";
    this.count.classList.toggle("mon-count-alert", alerts > 0);
  }

  private render() {
    this.updateCount();
    this.watch.disabled = this.actions.hosts().length === 0;
    if (this.panels.length === 0) {
      const hosts = this.actions.hosts().length > 0;
      const go = el("button", { type: "button", className: "primary" }, hosts ? "Watch a host" : "Add a host first");
      go.onclick = () => (hosts ? this.add(this.pick.value) : this.actions.showHosts());
      this.list.replaceChildren(
        el(
          "div",
          { className: "home-empty" },
          icon("pulse", 40),
          el("h2", {}, "No hosts watched"),
          el("p", { className: "muted" }, "Pick a saved host and press Watch to see its CPU, memory, network and disks update live. Works with any Linux server you can log in to."),
          el("div", { className: "home-empty-actions" }, go),
        ),
      );
      return;
    }
    this.list.replaceChildren(...this.panels.map((p) => p.root));
  }
}
