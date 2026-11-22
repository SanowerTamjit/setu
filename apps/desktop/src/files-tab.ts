// A Files tab: this computer on the left, the server on the right (SFTP on
// an SSH tab's connection), and a transfer queue below.

import { el, errorText, field, modal } from "./dom";
import {
  filesClose,
  filesList,
  filesMkdir,
  filesOpen,
  filesRemove,
  filesRename,
  localList,
  transfer,
  transferCancel,
  transferPause,
  type FileEntry,
  type Listing,
  type TransferProgress,
} from "./ipc";
import { icon } from "./icons";
import type { Tab } from "./tab";

const FOLDER_ICON =
  '<svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true"><path fill="currentColor" d="M1.5 3A1.5 1.5 0 0 1 3 1.5h3.2l1.6 1.8H13A1.5 1.5 0 0 1 14.5 4.8v7.7A1.5 1.5 0 0 1 13 14H3a1.5 1.5 0 0 1-1.5-1.5z"/></svg>';
const FILE_ICON =
  '<svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true"><path fill="currentColor" d="M3.5 1.5h6l3 3v10h-9zM9 2v3h3" opacity=".85"/></svg>';
const LINK_ICON =
  '<svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="1.6" d="M6.5 9.5l3-3M7 4.5l1.2-1.2a2.5 2.5 0 0 1 3.5 3.5L10.5 8M9 11.5l-1.2 1.2a2.5 2.5 0 0 1-3.5-3.5L5.5 8"/></svg>';

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let n = bytes / 1024;
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  return `${n >= 100 ? n.toFixed(0) : n.toFixed(1)} ${units[i]}`;
}

function formatDate(seconds: number | null) {
  if (seconds === null) return "";
  const d = new Date(seconds * 1000);
  const sameYear = d.getFullYear() === new Date().getFullYear();
  return d.toLocaleString(undefined, {
    day: "numeric",
    month: "short",
    ...(sameYear ? { hour: "2-digit", minute: "2-digit" } : { year: "numeric" }),
  });
}

/** Joins a folder and a name in the side's own path style. */
function joinPath(dir: string, name: string, remote: boolean) {
  const sep = remote ? "/" : dir.includes("\\") && !dir.includes("/") ? "\\" : "/";
  return dir.endsWith(sep) ? dir + name : dir + sep + name;
}

/** Asks for a name in a small dialog. Resolves null on cancel. */
function askName(title: string, label: string, initial = "", action = "OK"): Promise<string | null> {
  return new Promise((resolve) => {
    let value: string | null = null;
    const input = el("input", { value: initial, spellcheck: false, autocomplete: "off" });
    const ok = el("button", { type: "submit", className: "primary" }, action);
    const cancel = el("button", { type: "button" }, "Cancel");
    const form = el("form", { className: "dialog small" }, el("h2", {}, title), field(label, input), el("div", { className: "actions" }, cancel, ok));
    const { close } = modal(form, () => resolve(value));
    cancel.onclick = close;
    input.focus();
    // Select the name without its extension, like file managers do.
    const dot = initial.lastIndexOf(".");
    input.setSelectionRange(0, dot > 0 ? dot : initial.length);
    form.onsubmit = (e) => {
      e.preventDefault();
      if (!input.value.trim()) return;
      value = input.value.trim();
      close();
    };
  });
}

function confirmDialog(title: string, text: string, action: string, danger = false): Promise<boolean> {
  return new Promise((resolve) => {
    let yes = false;
    const ok = el("button", { type: "submit", className: danger ? "danger-solid" : "primary" }, action);
    const cancel = el("button", { type: "button" }, "Cancel");
    const form = el("form", { className: "dialog small" }, el("h2", {}, title), el("p", {}, text), el("div", { className: "actions" }, cancel, ok));
    const { close } = modal(form, () => resolve(yes));
    cancel.onclick = close;
    ok.focus();
    form.onsubmit = (e) => {
      e.preventDefault();
      yes = true;
      close();
    };
  });
}

type Side = "local" | "remote";

/** One side of the Files tab. */
class Pane {
  readonly element: HTMLElement;
  listing: Listing | null = null;
  private selected = new Set<string>();
  private anchor: string | null = null;
  private showHidden = false;
  private readonly list: HTMLDivElement;
  private readonly pathInput: HTMLInputElement;
  private readonly footer: HTMLDivElement;
  private readonly message: HTMLDivElement;

  constructor(
    readonly side: Side,
    label: string,
    private readonly load: (path: string) => Promise<Listing>,
    private readonly hooks: {
      onSend: (pane: Pane, names: string[]) => void;
      onDrop: (pane: Pane, from: Side, names: string[]) => void;
      actions: (pane: Pane) => HTMLElement[];
      onKey?: (pane: Pane, e: KeyboardEvent) => boolean;
    },
  ) {
    const up = el("button", { className: "tool", title: "Up one folder (Backspace)", ariaLabel: "Up one folder" }, "↑");
    up.onclick = () => this.up();
    const refresh = el("button", { className: "tool", title: "Refresh", ariaLabel: "Refresh" }, "⟳");
    refresh.onclick = () => void this.refresh();
    const hidden = el("button", { className: "tool", title: "Show hidden files", ariaLabel: "Show hidden files" }, ".*");
    hidden.setAttribute("aria-pressed", "false");
    hidden.onclick = () => {
      this.showHidden = !this.showHidden;
      hidden.setAttribute("aria-pressed", String(this.showHidden));
      this.render();
    };
    this.pathInput = el("input", { className: "path-input", spellcheck: false, autocomplete: "off" });
    this.pathInput.onkeydown = (e) => {
      if (e.key === "Enter") void this.open(this.pathInput.value);
      if (e.key === "Escape") this.pathInput.value = this.listing?.path ?? "";
    };
    const send = el("button", { className: "send", title: side === "local" ? "Upload the selection to the server" : "Download the selection to this computer" },
      side === "local" ? "Upload →" : "← Download");
    send.onclick = () => this.hooks.onSend(this, [...this.selected]);
    this.list = el("div", { className: "file-list", tabIndex: 0, role: "listbox" });
    this.list.setAttribute("aria-multiselectable", "true");
    this.list.onkeydown = (e) => this.onKey(e);
    this.list.ondragover = (e) => {
      if (e.dataTransfer?.types.includes("application/x-setu-files")) {
        e.preventDefault();
        this.list.classList.add("drop-target");
      }
    };
    this.list.ondragleave = () => this.list.classList.remove("drop-target");
    this.list.ondrop = (e) => {
      this.list.classList.remove("drop-target");
      const raw = e.dataTransfer?.getData("application/x-setu-files");
      if (!raw) return;
      e.preventDefault();
      const data = JSON.parse(raw) as { side: Side; names: string[] };
      if (data.side !== this.side) this.hooks.onDrop(this, data.side, data.names);
    };
    this.message = el("div", { className: "pane-message", hidden: true });
    this.footer = el("div", { className: "pane-footer muted" });
    this.element = el("section", { className: `pane ${side}` },
      el("div", { className: "pane-head" },
        el("span", { className: "pane-label" }, label),
        el("span", { className: "spacer" }),
        ...this.hooks.actions(this),
        send,
      ),
      el("div", { className: "pane-path" }, up, refresh, this.pathInput, hidden),
      this.message,
      el("div", { className: "file-header" },
        el("span", {}, "Name"), el("span", { className: "num" }, "Size"), el("span", {}, "Modified")),
      this.list,
      this.footer,
    );
  }

  get path() {
    return this.listing?.path ?? "";
  }

  /** Entries as shown: dotfiles only when "Show hidden files" is on. */
  get entries(): FileEntry[] {
    const all = this.listing?.entries ?? [];
    return this.showHidden ? all : all.filter((e) => !e.name.startsWith("."));
  }

  get selection(): FileEntry[] {
    return this.entries.filter((e) => this.selected.has(e.name));
  }

  focus() {
    this.list.focus();
  }

  showError(text: string | null) {
    this.message.hidden = text === null;
    this.message.textContent = text ?? "";
  }

  async open(path: string, select?: string) {
    try {
      const listing = await this.load(path);
      const changed = listing.path !== this.listing?.path;
      this.listing = listing;
      if (changed) this.selected.clear();
      if (select) this.selected = new Set([select]);
      // Forget selections of entries that are gone.
      const names = new Set(listing.entries.map((e) => e.name));
      for (const n of [...this.selected]) if (!names.has(n)) this.selected.delete(n);
      this.showError(null);
      this.render();
    } catch (err) {
      this.showError(errorText(err));
      this.pathInput.value = this.listing?.path ?? path;
    }
  }

  refresh() {
    return this.open(this.path);
  }

  up() {
    if (this.listing?.parent) void this.open(this.listing.parent, this.listing.path.split(/[\\/]/).filter(Boolean).pop());
  }

  private enter(entry: FileEntry) {
    if (entry.kind === "dir" || entry.kind === "symlink") {
      void this.open(joinPath(this.path, entry.name, this.side === "remote"));
    } else {
      this.hooks.onSend(this, [entry.name]);
    }
  }

  private click(entry: FileEntry, e: MouseEvent) {
    const names = this.entries.map((x) => x.name);
    if (e.shiftKey && this.anchor) {
      const [a, b] = [names.indexOf(this.anchor), names.indexOf(entry.name)].sort((x, y) => x - y);
      this.selected = new Set(names.slice(a, b + 1));
    } else if (e.ctrlKey || e.metaKey) {
      if (this.selected.has(entry.name)) this.selected.delete(entry.name);
      else this.selected.add(entry.name);
      this.anchor = entry.name;
    } else {
      this.selected = new Set([entry.name]);
      this.anchor = entry.name;
    }
    this.render();
  }

  private onKey(e: KeyboardEvent) {
    if (this.hooks.onKey?.(this, e)) return;
    const entries = this.entries;
    const current = entries.findIndex((x) => x.name === this.anchor);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = entries[Math.max(0, Math.min(entries.length - 1, current + (e.key === "ArrowDown" ? 1 : -1)))];
      if (next) {
        this.selected = new Set([next.name]);
        this.anchor = next.name;
        this.render();
      }
    } else if (e.key === "Enter" && current >= 0) {
      e.preventDefault();
      this.enter(entries[current]);
    } else if (e.key === "Backspace") {
      e.preventDefault();
      this.up();
    } else if ((e.ctrlKey || e.metaKey) && e.key === "a") {
      e.preventDefault();
      this.selected = new Set(entries.map((x) => x.name));
      this.render();
    }
  }

  render() {
    const listing = this.listing;
    if (!listing) return;
    this.pathInput.value = listing.path;
    const shown = this.entries;
    const rows = shown.map((entry) => {
      const icon = el("span", { className: `icon ${entry.kind}` });
      icon.innerHTML = entry.kind === "dir" ? FOLDER_ICON : entry.kind === "symlink" ? LINK_ICON : FILE_ICON;
      const row = el("div", { className: "file-row" + (this.selected.has(entry.name) ? " selected" : ""), role: "option", draggable: true },
        el("span", { className: "name" }, icon, el("span", {}, entry.name)),
        el("span", { className: "num muted" }, entry.kind === "file" ? formatSize(entry.size) : ""),
        el("span", { className: "muted" }, formatDate(entry.modified)),
      );
      row.setAttribute("aria-selected", String(this.selected.has(entry.name)));
      // Shift-click extends the selection; keep it from selecting text too.
      row.onmousedown = (e) => {
        if (e.shiftKey) e.preventDefault();
      };
      row.onclick = (e) => this.click(entry, e);
      row.ondblclick = () => this.enter(entry);
      row.ondragstart = (e) => {
        if (!this.selected.has(entry.name)) {
          this.selected = new Set([entry.name]);
          this.render();
        }
        e.dataTransfer?.setData("application/x-setu-files", JSON.stringify({ side: this.side, names: [...this.selected] }));
        if (e.dataTransfer) e.dataTransfer.effectAllowed = "copy";
      };
      return row;
    });
    this.list.replaceChildren(
      ...(rows.length ? rows : [el("div", { className: "empty-folder muted" }, "This folder is empty")]),
    );
    const sel = this.selection;
    const bytes = sel.reduce((n, e) => n + (e.kind === "file" ? e.size : 0), 0);
    this.footer.textContent = sel.length
      ? `${sel.length} selected${bytes ? ` · ${formatSize(bytes)}` : ""}`
      : `${shown.length} items${shown.length < listing.entries.length ? ` · ${listing.entries.length - shown.length} hidden` : ""}`;
  }
}

type Status = "queued" | "running" | "paused" | "done" | "failed" | "cancelled";

interface Job {
  id: number;
  direction: "upload" | "download";
  name: string;
  source: string;
  targetDir: string;
  status: Status;
  progress: TransferProgress | null;
  /** Continue partial files and skip finished ones (set by Resume and Retry). */
  resume: boolean;
  error?: string;
  started?: number;
  /** Bytes done when this run started, so speed counts only this run. */
  startBytes?: number;
  row?: HTMLElement;
}

let nextTransferId = 1;

export class FilesTab implements Tab {
  readonly element: HTMLDivElement;
  title: string;
  detail: string;
  private sftpId: number | null = null;
  private readonly local: Pane;
  private readonly remote: Pane;
  private readonly queue: Job[] = [];
  private readonly queueList: HTMLDivElement;
  private readonly queueTitle: HTMLSpanElement;
  private readonly pauseAll: HTMLButtonElement;
  private readonly resumeAll: HTMLButtonElement;
  private running = false;
  private disposed = false;

  constructor(
    container: HTMLElement,
    private readonly terminalId: number,
    private readonly hostLabel: string,
    private readonly onChange: () => void,
  ) {
    this.title = `Files · ${hostLabel}`;
    this.detail = `sftp ${hostLabel}`;
    this.local = new Pane("local", "This computer", (p) => localList(p), {
      onSend: (_, names) => this.enqueue("upload", names),
      onDrop: (_, from, names) => from === "remote" && this.enqueue("download", names),
      actions: () => [],
    });
    this.remote = new Pane("remote", hostLabel, (p) => filesList(this.id(), p), {
      onSend: (_, names) => this.enqueue("download", names),
      onDrop: (_, from, names) => from === "local" && this.enqueue("upload", names),
      actions: (pane) => this.remoteActions(pane),
      onKey: (pane, e) => {
        if (e.key === "Delete") {
          void this.remove(pane);
          return true;
        }
        if (e.key === "F2") {
          void this.rename(pane);
          return true;
        }
        return false;
      },
    });
    this.queueTitle = el("span", {}, "Transfers");
    const clear = el("button", { className: "text-button" }, "Clear finished");
    clear.onclick = () => {
      for (let i = this.queue.length - 1; i >= 0; i--) {
        if (["done", "cancelled"].includes(this.queue[i].status)) this.queue.splice(i, 1);
      }
      this.renderQueue();
    };
    this.pauseAll = el("button", { className: "text-button" }, "Pause all");
    this.pauseAll.onclick = () => {
      for (const job of this.queue) if (job.status === "queued" || job.status === "running") this.pause(job);
    };
    this.resumeAll = el("button", { className: "text-button" }, "Resume all");
    this.resumeAll.onclick = () => {
      for (const job of this.queue) if (job.status === "paused" || job.status === "failed") this.resume(job);
    };
    this.queueList = el("div", { className: "queue-list" });
    this.element = el("div", { className: "files-tab" },
      el("div", { className: "panes" }, this.local.element, this.remote.element),
      el("div", { className: "queue" }, el("div", { className: "queue-head" }, this.queueTitle, el("span", { className: "spacer" }), this.pauseAll, this.resumeAll, clear), this.queueList),
    );
    container.appendChild(this.element);
    this.renderQueue();
  }

  private id(): number {
    if (this.sftpId === null) throw new Error("not connected");
    return this.sftpId;
  }

  async start() {
    const { id, home } = await filesOpen(this.terminalId);
    this.sftpId = id;
    await Promise.all([this.local.open(""), this.remote.open(home)]);
    this.onChange();
  }

  private remoteActions(pane: Pane) {
    const mk = el("button", { className: "tool", title: "New folder" }, "New folder");
    mk.onclick = () => void this.mkdir(pane);
    const rn = el("button", { className: "tool", title: "Rename (F2)" }, "Rename");
    rn.onclick = () => void this.rename(pane);
    const rm = el("button", { className: "tool danger", title: "Delete (Delete)" }, "Delete");
    rm.onclick = () => void this.remove(pane);
    return [mk, rn, rm];
  }

  private async mkdir(pane: Pane) {
    const name = await askName("New folder", "Folder name", "", "Create");
    pane.focus();
    if (!name) return;
    try {
      await filesMkdir(this.id(), pane.path, name);
      await pane.open(pane.path, name);
    } catch (err) {
      pane.showError(errorText(err));
    }
  }

  private async rename(pane: Pane) {
    const [entry] = pane.selection;
    if (!entry) return;
    const name = await askName(`Rename ${entry.name}`, "New name", entry.name, "Rename");
    pane.focus();
    if (!name || name === entry.name) return;
    try {
      await filesRename(this.id(), pane.path, entry.name, name);
      await pane.open(pane.path, name);
    } catch (err) {
      pane.showError(errorText(err));
    }
  }

  private async remove(pane: Pane) {
    const sel = pane.selection;
    if (!sel.length) return;
    const folders = sel.filter((e) => e.kind === "dir").length;
    const what = sel.length === 1 ? `“${sel[0].name}”` : `${sel.length} items`;
    const ok = await confirmDialog(
      `Delete ${what}?`,
      `This deletes ${what} on ${this.hostLabel}${folders ? (sel.length === 1 ? " and everything inside it" : ", including everything inside the folders") : ""}. It cannot be undone.`,
      "Delete",
      true,
    );
    pane.focus();
    if (!ok) return;
    try {
      for (const entry of sel) await filesRemove(this.id(), pane.path, entry.name);
    } catch (err) {
      pane.showError(errorText(err));
    }
    await pane.refresh();
  }

  /** Queues copies of the named entries from one side to the other. */
  private async enqueue(direction: "upload" | "download", names: string[]) {
    const [from, to] = direction === "upload" ? [this.local, this.remote] : [this.remote, this.local];
    if (!names.length || !from.listing || !to.listing) return;
    const existing = new Set(to.listing.entries.map((e) => e.name));
    const clashes = names.filter((n) => existing.has(n));
    if (clashes.length) {
      const what = clashes.length === 1 ? `“${clashes[0]}” already exists` : `${clashes.length} items already exist`;
      const ok = await confirmDialog("Replace existing items?", `${what} in ${to.path}. Copying replaces files with the same name.`, "Replace");
      if (!ok) return;
    }
    for (const name of names) {
      this.queue.push({
        id: nextTransferId++,
        direction,
        name,
        source: joinPath(from.path, name, from.side === "remote"),
        targetDir: to.path,
        status: "queued",
        progress: null,
        resume: false,
      });
    }
    this.renderQueue();
    void this.pump();
  }

  /** Runs queued copies one at a time. */
  private async pump() {
    if (this.running) return;
    this.running = true;
    try {
      let job: Job | undefined;
      while (!this.disposed && (job = this.queue.find((j) => j.status === "queued"))) {
        job.status = "running";
        job.started = performance.now();
        job.startBytes = undefined;
        job.error = undefined;
        this.renderQueue();
        const current = job;
        try {
          current.progress = await transfer(this.id(), current.id, current.direction, current.source, current.targetDir, current.resume, (p) => {
            current.progress = p;
            current.startBytes ??= p.doneBytes;
            this.updateRow(current);
          });
          current.status = "done";
        } catch (err) {
          const text = errorText(err);
          current.status = text === "paused" ? "paused" : /cancelled/.test(text) ? "cancelled" : "failed";
          if (current.status === "failed") current.error = text;
          // A paused or failed copy keeps its partial files; going on continues them.
          if (current.status !== "cancelled") current.resume = true;
        }
        this.renderQueue();
        const target = current.direction === "upload" ? this.remote : this.local;
        if (target.path === current.targetDir) void target.refresh();
      }
    } finally {
      this.running = false;
    }
  }

  private cancel(job: Job) {
    if (job.status === "queued" || job.status === "paused" || job.status === "failed") {
      job.status = "cancelled";
      this.renderQueue();
    } else if (job.status === "running") {
      void transferCancel(job.id);
    }
  }

  private pause(job: Job) {
    if (job.status === "queued") {
      job.status = "paused";
      this.renderQueue();
    } else if (job.status === "running") {
      void transferPause(job.id);
    }
  }

  /** Queues a paused, failed or cancelled copy again, continuing partial files. */
  private resume(job: Job) {
    if (job.status === "queued" || job.status === "running" || job.status === "done") return;
    job.resume = job.status !== "cancelled" || job.resume;
    job.status = "queued";
    // Copies keep their order: a resumed one goes to the back of the line.
    this.queue.splice(this.queue.indexOf(job), 1);
    this.queue.push(job);
    this.renderQueue();
    void this.pump();
  }

  private renderQueue() {
    const active = this.queue.filter((j) => j.status === "queued" || j.status === "running").length;
    const stopped = this.queue.filter((j) => j.status === "paused" || j.status === "failed").length;
    this.queueTitle.textContent = [
      "Transfers",
      ...(active ? [`${active} in progress`] : []),
      ...(stopped ? [`${stopped} paused or failed`] : []),
    ].join(" · ");
    this.pauseAll.hidden = active === 0;
    this.resumeAll.hidden = stopped === 0;
    if (!this.queue.length) {
      this.queueList.replaceChildren(el("div", { className: "queue-empty muted" }, "Drag files between the two sides, or select them and use Upload or Download."));
      return;
    }
    // Running and waiting copies first, then finished ones, newest first.
    const order = (j: Job) => (j.status === "running" ? 0 : j.status === "queued" ? 1 : j.status === "paused" || j.status === "failed" ? 2 : 3);
    const shown = [...this.queue].sort((a, b) => order(a) - order(b) || (order(a) === 3 ? b.id - a.id : 0));
    this.queueList.replaceChildren(
      ...shown.map((job) => {
        job.row = el("div", { className: `job ${job.status}` });
        this.updateRow(job);
        return job.row;
      }),
    );
  }

  private updateRow(job: Job) {
    const row = job.row;
    if (!row) return;
    const p = job.progress;
    const pct = p && p.totalBytes ? Math.min(100, (p.doneBytes / p.totalBytes) * 100) : job.status === "done" ? 100 : 0;
    const seconds = job.started ? (performance.now() - job.started) / 1000 : 0;
    const moved = p ? p.doneBytes - (job.startBytes ?? p.doneBytes) : 0;
    const speed = p && seconds > 0.3 && moved > 0 ? `${formatSize(moved / seconds)}/s` : "";
    const of = p ? `${formatSize(p.doneBytes)} of ${formatSize(p.totalBytes)}` : "";
    let status: string;
    switch (job.status) {
      case "queued":
        status = job.resume && p ? `Waiting to resume · ${of}` : "Waiting";
        break;
      case "running":
        status = p ? `${of}${speed ? ` · ${speed}` : ""}${p.resumedBytes ? " · resumed" : ""}` : "Starting…";
        break;
      case "paused":
        status = p ? `Paused · ${of}` : "Paused";
        break;
      case "done":
        status = p
          ? `Done · ${p.filesTotal} ${p.filesTotal === 1 ? "file" : "files"}, ${formatSize(p.totalBytes)}${p.resumedBytes ? " · resumed" : ""}`
          : "Done";
        break;
      case "cancelled":
        status = "Cancelled";
        break;
      default:
        status = `${job.error ?? "Failed"}${p ? ` · ${of} kept` : ""}`;
    }
    const button = (name: "pause" | "play" | "retry" | "close", label: string, run: () => void) => {
      const b = el("button", { className: "row-button", title: label, ariaLabel: `${label} ${job.name}` }, icon(name, 13));
      b.onclick = run;
      return b;
    };
    const buttons: HTMLButtonElement[] = [];
    if (job.status === "running" || job.status === "queued") buttons.push(button("pause", "Pause", () => this.pause(job)));
    if (job.status === "paused") buttons.push(button("play", "Resume", () => this.resume(job)));
    if (job.status === "failed") buttons.push(button("retry", "Retry from where it stopped", () => this.resume(job)));
    if (job.status === "cancelled") buttons.push(button("retry", "Start again", () => this.resume(job)));
    if (job.status === "running" || job.status === "queued") buttons.push(button("close", "Cancel (deletes the partial file)", () => this.cancel(job)));
    if (job.status === "paused" || job.status === "failed") {
      buttons.push(
        button("close", "Remove from the list (the .setu-part file stays until you resume or delete it)", () => {
          this.queue.splice(this.queue.indexOf(job), 1);
          this.renderQueue();
        }),
      );
    }
    const sub = job.status === "running" && p && p.filesTotal > 1 ? `${p.filesDone + 1}/${p.filesTotal} · ${p.current}` : `to ${job.targetDir}`;
    row.className = `job ${job.status}`;
    row.replaceChildren(
      el("span", { className: "dir" }, job.direction === "upload" ? "↑" : "↓"),
      el("span", { className: "job-text" }, el("span", { className: "job-name" }, job.name), el("span", { className: "muted job-sub" }, sub)),
      el("span", { className: "bar" }, Object.assign(el("span", {}), { style: `width:${pct}%` })),
      el(
        "span",
        {
          className: "job-status muted",
          title: p?.resumedBytes ? `${status}\nContinued from ${formatSize(p.resumedBytes)} already copied before it stopped.` : status,
        },
        status,
      ),
      el("span", { className: "job-buttons" }, ...buttons),
    );
  }

  // ---- Tab ----

  get tooltip() {
    return `Files on ${this.hostLabel} (SFTP)`;
  }

  get sizeLabel() {
    return "";
  }

  get hasExited() {
    return false;
  }

  show() {
    this.element.classList.add("active");
  }

  hide() {
    this.element.classList.remove("active");
  }

  refit() {}

  focus() {
    this.remote.focus();
  }

  async dispose() {
    this.disposed = true;
    for (const job of this.queue) if (job.status === "running") void transferCancel(job.id);
    if (this.sftpId !== null) await filesClose(this.sftpId).catch(() => {});
    this.element.remove();
  }
}
