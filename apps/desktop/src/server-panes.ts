// The Processes and Disk tabs of the Server view. Like Services, they only
// show what the server reports (as text), and stopping a process shows the
// exact command and asks first. src-tauri/src/server.rs runs the commands.

import { confirmDialog, el, errorText } from "./dom";
import { icon } from "./icons";
import { serverDisks, serverFolderSizes, serverProcessCommand, serverProcessSignal, serverProcesses, type ActionResult, type Filesystem, type FolderSizes, type Process, type ProcessSignal } from "./ipc";
import { childPath, crumbs, elapsedWords, filterProcesses, fullness, kbText, parentPath, signalText, usedPercent, type ProcessSort } from "./server-model";

export interface PaneContext {
  host: () => string;
  sudo: () => boolean;
}

/** The result of a change, as shown above a pane. */
export function resultBox(r: ActionResult): HTMLElement {
  return el(
    "div",
    { className: `net-result ${r.ok ? "good" : "bad"} srv-result`, role: "status" },
    el("p", { className: "net-headline" }, r.ok ? "Done." : `It did not work${r.code !== null ? ` (exit code ${r.code})` : ""}.`),
    el("code", { className: "srv-command small" }, r.command),
    ...(r.hint ? [el("p", { className: "net-small" }, r.hint)] : []),
    ...(r.output ? [el("pre", { className: "net-lines" }, r.output)] : []),
  );
}

/** The body of a "run this?" dialog: what happens, the exact command, notes. */
export function confirmBody(what: string, host: string, command: string, sudo: boolean, warning: string | null): HTMLElement {
  return el(
    "div",
    { className: "srv-confirm" },
    el("p", {}, what),
    el("p", { className: "muted" }, `Setu will run this on ${host}:`),
    el("code", { className: "srv-command" }, command),
    ...(sudo ? [el("p", { className: "muted net-small" }, "sudo -n fails instead of asking for a password: Setu never types one into sudo.")] : []),
    ...(warning ? [el("p", { className: "srv-warning" }, warning)] : []),
  );
}

export class ProcessesPane {
  readonly root: HTMLElement;
  private readonly search: HTMLInputElement;
  private readonly sort: HTMLSelectElement;
  private readonly note: HTMLElement;
  private readonly bar: HTMLElement;
  private readonly table: HTMLElement;
  private readonly message: HTMLElement;
  private list: Process[] = [];
  private selected: number | null = null;
  private loadedFor = "";
  private gen = 0;

  constructor(private readonly ctx: PaneContext) {
    this.search = el("input", { type: "search", className: "srv-search", placeholder: "Filter by name, user or process id", spellcheck: false, autocomplete: "off" });
    this.search.oninput = () => this.render();
    this.sort = el("select", { className: "srv-select" }, el("option", { value: "cpu" }, "Busiest first"), el("option", { value: "mem" }, "Most memory first"));
    this.sort.setAttribute("aria-label", "Sort");
    this.sort.onchange = () => this.render();
    const again = el("button", { type: "button", className: "card-icon", title: "Read the processes again" }, icon("retry", 14));
    again.setAttribute("aria-label", "Read the processes again");
    again.onclick = () => void this.load(true);
    this.note = el("span", { className: "muted srv-count" });
    this.bar = el("div", { className: "srv-proc-bar" });
    this.message = el("div", { className: "srv-message" });
    this.table = el("div", { className: "srv-proc-table", role: "table" });
    this.table.setAttribute("aria-label", "Processes");
    this.root = el(
      "section",
      { className: "host-card srv-pane" },
      el("div", { className: "srv-toolbar" }, this.search, this.sort, again, this.note),
      this.message,
      this.bar,
      this.table,
      el("p", { className: "muted srv-ran" }, "Ran: ps -eo pid,ppid,user,pcpu,pmem,rss,etimes,stat,comm --sort=-pcpu. Only command names are read, never their arguments, which can hold passwords. CPU is averaged over each process's life, as ps counts it; Monitoring shows it live."),
    );
    this.renderBar();
  }

  /** Reads the list unless it is already there for this host. */
  async load(force = false) {
    const host = this.ctx.host();
    if (!host || (!force && this.loadedFor === host)) return;
    if (this.loadedFor !== host) {
      this.selected = null;
      this.message.replaceChildren();
    }
    this.loadedFor = host;
    const gen = ++this.gen;
    if (!this.list.length) this.table.replaceChildren(el("p", { className: "muted srv-loading" }, `Reading processes on ${host}…`));
    try {
      const r = await serverProcesses(host);
      if (gen !== this.gen) return;
      this.list = r.processes;
      this.note.textContent = `${r.processes.length}${r.truncated ? "+" : ""} processes`;
      this.render();
    } catch (err) {
      if (gen !== this.gen) return;
      this.list = [];
      this.table.replaceChildren(el("p", { className: "srv-problem" }, errorText(err)));
    }
  }

  /** Forgets the list (the host changed). */
  reset() {
    this.loadedFor = "";
    this.list = [];
    this.selected = null;
    this.message.replaceChildren();
    this.renderBar();
  }

  private render() {
    const shown = filterProcesses(this.list, this.search.value, this.sort.value as ProcessSort);
    if (this.selected !== null && !this.list.some((p) => p.pid === this.selected)) this.selected = null;
    const head = el(
      "div",
      { className: "srv-proc-row head", role: "row" },
      ...["PID", "User", "CPU", "Mem", "Memory", "Running", "Command"].map((h) => el("span", { role: "columnheader" }, h)),
    );
    const rows = shown.slice(0, 300).map((p) => {
      const row = el(
        "button",
        { type: "button", className: `srv-proc-row${p.pid === this.selected ? " current" : ""}`, role: "row" },
        el("span", { className: "num" }, String(p.pid)),
        el("span", { className: "srv-proc-user" }, p.user),
        el("span", { className: `num${p.cpu >= 50 ? " srv-hot" : ""}` }, `${p.cpu.toFixed(1)}%`),
        el("span", { className: "num" }, `${p.mem.toFixed(1)}%`),
        el("span", { className: "num" }, kbText(p.rssKb)),
        el("span", { className: "num" }, elapsedWords(p.elapsedSecs)),
        el("span", { className: "srv-proc-cmd" }, p.command, ...(p.state.startsWith("Z") ? [el("small", { className: "muted" }, " (zombie)")] : [])),
      );
      row.setAttribute("aria-selected", String(p.pid === this.selected));
      row.onclick = () => {
        this.selected = p.pid;
        this.render();
      };
      return row;
    });
    this.table.replaceChildren(head, ...(rows.length ? rows : [el("p", { className: "muted srv-loading" }, "No process matches.")]));
    this.renderBar();
  }

  private renderBar() {
    const p = this.list.find((x) => x.pid === this.selected);
    if (!p) {
      this.bar.replaceChildren(el("span", { className: "muted" }, "Pick a process to stop it."));
      return;
    }
    if (p.pid <= 1) {
      this.bar.replaceChildren(el("span", {}, el("strong", {}, p.command), el("span", { className: "muted" }, ` ${p.pid} · the system itself; Setu does not stop it`)));
      return;
    }
    const stop = el("button", { type: "button", className: "srv-action risky" }, "Stop");
    stop.title = "Ask it to stop (SIGTERM)";
    stop.onclick = () => void this.signal(p, "term");
    const kill = el("button", { type: "button", className: "srv-action risky" }, "Kill");
    kill.title = "Stop it at once (SIGKILL)";
    kill.onclick = () => void this.signal(p, "kill");
    this.bar.replaceChildren(el("span", {}, el("strong", {}, p.command), el("span", { className: "muted" }, ` ${p.pid} · ${p.user}`)), stop, kill);
  }

  private async signal(p: Process, signal: ProcessSignal) {
    const host = this.ctx.host();
    const sudo = this.ctx.sudo();
    let command: string;
    try {
      command = await serverProcessCommand(signal, p.pid, sudo);
    } catch (err) {
      this.message.replaceChildren(el("p", { className: "srv-problem" }, errorText(err)));
      return;
    }
    const q = signalText(signal, p, host);
    if (!(await confirmDialog(q.title, confirmBody(q.what, host, command, sudo, q.warning), q.button, true, true))) return;
    try {
      const r = await serverProcessSignal(host, signal, p.pid, sudo);
      if (this.ctx.host() !== host) return;
      this.message.replaceChildren(resultBox(r));
      if (r.ok) this.selected = null;
    } catch (err) {
      this.message.replaceChildren(el("p", { className: "srv-problem" }, errorText(err)));
    }
    // Give it a moment to go, then read the list again.
    await new Promise((r) => setTimeout(r, 700));
    await this.load(true);
  }
}

export class DiskPane {
  readonly root: HTMLElement;
  private readonly disks: HTMLElement;
  private readonly crumbsEl: HTMLElement;
  private readonly path: HTMLInputElement;
  private readonly folders: HTMLElement;
  private readonly ran: HTMLElement;
  private loadedFor = "";
  private gen = 0;
  private scanGen = 0;

  constructor(private readonly ctx: PaneContext) {
    this.disks = el("div", { className: "srv-disks" });
    this.crumbsEl = el("nav", { className: "srv-crumbs" });
    this.crumbsEl.setAttribute("aria-label", "Folder");
    this.path = el("input", { className: "srv-search srv-path", value: "/", spellcheck: false, autocomplete: "off" });
    this.path.setAttribute("aria-label", "Folder to measure");
    const go = el("button", { type: "submit", className: "primary" }, "Measure");
    const form = el("form", { className: "srv-toolbar" }, this.path, go);
    form.onsubmit = (e) => {
      e.preventDefault();
      void this.scan(this.path.value.trim() || "/");
    };
    this.folders = el("div", { className: "srv-folders" });
    this.ran = el("p", { className: "muted srv-ran" });
    this.root = el(
      "section",
      { className: "host-card srv-pane" },
      el("h3", { className: "srv-h" }, "Disks"),
      this.disks,
      el("h3", { className: "srv-h" }, "What uses the space"),
      el("p", { className: "muted net-small" }, "Measures the folders in a folder, biggest first, staying on that disk. Big folders can take up to a minute."),
      form,
      this.crumbsEl,
      this.folders,
      this.ran,
    );
  }

  async load(force = false) {
    const host = this.ctx.host();
    if (!host || (!force && this.loadedFor === host)) return;
    const fresh = this.loadedFor !== host;
    this.loadedFor = host;
    const gen = ++this.gen;
    if (fresh) {
      this.folders.replaceChildren();
      this.crumbsEl.replaceChildren();
      this.ran.textContent = "";
    }
    this.disks.replaceChildren(el("p", { className: "muted srv-loading" }, `Reading disks on ${host}…`));
    try {
      const list = await serverDisks(host);
      if (gen !== this.gen) return;
      this.disks.replaceChildren(...list.map((f) => this.disk(f)));
    } catch (err) {
      if (gen !== this.gen) return;
      this.disks.replaceChildren(el("p", { className: "srv-problem" }, errorText(err)));
    }
  }

  reset() {
    this.loadedFor = "";
    this.folders.replaceChildren();
    this.crumbsEl.replaceChildren();
    this.ran.textContent = "";
    this.path.value = "/";
  }

  private disk(f: Filesystem) {
    const pct = usedPercent(f);
    const look = el("button", { type: "button", className: "srv-action" }, "What uses it");
    look.onclick = () => void this.scan(f.mount);
    const fill = el("span", { className: "srv-bar-fill" });
    fill.style.width = `${pct.toFixed(1)}%`;
    return el(
      "div",
      { className: `srv-disk level-${fullness(pct)}` },
      el("div", { className: "srv-disk-head" }, el("span", { className: "srv-disk-mount" }, f.mount), el("span", { className: "muted" }, `${f.kind} · ${f.device}`), look),
      el("span", { className: "srv-bar" }, fill),
      el("span", { className: "srv-disk-use" }, `${kbText(f.usedKb)} of ${kbText(f.totalKb)} used (${Math.round(pct)}%) · ${kbText(f.availableKb)} free`),
    );
  }

  private async scan(path: string) {
    const host = this.ctx.host();
    const gen = ++this.scanGen;
    this.path.value = path;
    this.renderCrumbs(path);
    this.folders.replaceChildren(el("p", { className: "muted srv-loading" }, `Measuring ${path}…`));
    this.ran.textContent = "";
    try {
      const r = await serverFolderSizes(host, path, this.ctx.sudo());
      if (gen !== this.scanGen || this.ctx.host() !== host) return;
      this.path.value = r.path;
      this.renderCrumbs(r.path);
      this.renderFolders(r);
    } catch (err) {
      if (gen !== this.scanGen) return;
      this.folders.replaceChildren(el("p", { className: "srv-problem" }, errorText(err)));
    }
  }

  private renderCrumbs(path: string) {
    const up = el("button", { type: "button", className: "card-icon", title: "Up one folder", disabled: parentPath(path) === null }, icon("chevronRight", 14));
    up.classList.add("srv-up");
    up.setAttribute("aria-label", "Up one folder");
    up.onclick = () => {
      const p = parentPath(path);
      if (p) void this.scan(p);
    };
    const parts = crumbs(path).map(([label, full], i, all) => {
      if (i === all.length - 1) return el("span", { className: "srv-crumb current" }, label);
      const b = el("button", { type: "button", className: "srv-crumb" }, label);
      b.onclick = () => void this.scan(full);
      return b;
    });
    this.crumbsEl.replaceChildren(up, ...parts);
  }

  private renderFolders(r: FolderSizes) {
    this.ran.textContent = `Ran: ${r.command}`;
    if (!r.folders.length) {
      this.folders.replaceChildren(
        el("p", { className: "muted srv-loading" }, r.totalKb === null ? "No sizes came back: the folder may not exist, you may not be allowed to read it (try Use sudo), or it took longer than a minute." : `No folders inside (${kbText(r.totalKb)} in all).`),
      );
      return;
    }
    const top = Math.max(1, ...r.folders.map((f) => f.kb));
    const rows = r.folders.map((f) => {
      const fill = el("span", { className: "srv-bar-fill" });
      fill.style.width = `${((f.kb / top) * 100).toFixed(1)}%`;
      const row = el(
        "button",
        { type: "button", className: "srv-folder", title: `Open ${childPath(r.path, f.name)}` },
        el("span", { className: "srv-folder-name" }, icon("folder", 14), f.name),
        el("span", { className: "srv-bar" }, fill),
        el("span", { className: "num" }, kbText(f.kb)),
      );
      row.onclick = () => void this.scan(childPath(r.path, f.name));
      return row;
    });
    this.folders.replaceChildren(...(r.totalKb !== null ? [el("p", { className: "muted net-small" }, `${kbText(r.totalKb)} in ${r.path} in all (files directly in it included).`)] : []), ...rows);
  }
}
