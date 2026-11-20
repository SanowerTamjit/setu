// The hosts sidebar: saved hosts in folders, search, connect, edit, import.

import { el, errorText } from "./dom";
import { openHostDialog } from "./host-dialog";
import { openImportDialog } from "./import-dialog";
import { hostsList, type Host, type HostsView } from "./ipc";

interface Folder {
  name: string;
  path: string;
  folders: Map<string, Folder>;
  hosts: Host[];
}

const COLLAPSED_KEY = "setu.hosts.collapsed";

function loadCollapsed(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(COLLAPSED_KEY) ?? "[]") as string[]);
  } catch {
    return new Set();
  }
}

function saveCollapsed(set: Set<string>) {
  try {
    localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...set]));
  } catch {
    // Not persisting folder state is fine.
  }
}

function matches(host: Host, query: string) {
  if (!query) return true;
  const haystack = [host.name, host.host, host.user ?? "", host.folder ?? "", ...(host.tags ?? [])]
    .join(" ")
    .toLowerCase();
  return query
    .toLowerCase()
    .split(/\s+/)
    .every((word) => haystack.includes(word));
}

function buildTree(hosts: Host[]): Folder {
  const root: Folder = { name: "", path: "", folders: new Map(), hosts: [] };
  for (const host of hosts) {
    let node = root;
    for (const part of (host.folder ?? "").split("/").filter(Boolean)) {
      let next = node.folders.get(part);
      if (!next) {
        next = { name: part, path: node.path ? `${node.path}/${part}` : part, folders: new Map(), hosts: [] };
        node.folders.set(part, next);
      }
      node = next;
    }
    node.hosts.push(host);
  }
  return root;
}

function countHosts(folder: Folder): number {
  let n = folder.hosts.length;
  for (const f of folder.folders.values()) n += countHosts(f);
  return n;
}

export class HostsPanel {
  private view: HostsView = { hosts: [], folders: [], path: "" };
  private query = "";
  private collapsed = loadCollapsed();
  private selected: string | null = null;
  private readonly search: HTMLInputElement;
  private readonly list: HTMLDivElement;
  private readonly notice: HTMLDivElement;
  private readonly footer: HTMLDivElement;
  private noticeTimer: number | undefined;

  constructor(
    root: HTMLElement,
    private readonly actions: { connect: (host: Host) => void; files: (host: Host) => void; defaultUser: () => string },
  ) {
    const add = el("button", { className: "icon-button", title: "Add host", ariaLabel: "Add host" }, "+");
    add.onclick = () => void this.edit(null);
    const importButton = el("button", { className: "text-button", title: "Import hosts from ~/.ssh/config" }, "Import");
    importButton.onclick = () => void this.import();
    this.search = el("input", { type: "search", placeholder: "Search hosts, tags, folders", spellcheck: false, autocomplete: "off" });
    this.search.oninput = () => {
      this.query = this.search.value.trim();
      this.render();
    };
    this.search.onkeydown = (e) => {
      if (e.key === "Enter") {
        const first = this.visibleHosts()[0];
        if (first) this.actions.connect(first);
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        this.list.querySelector<HTMLElement>(".host-row")?.focus();
      }
    };
    this.list = el("div", { className: "host-list", role: "tree" });
    this.notice = el("div", { className: "panel-notice", hidden: true });
    this.footer = el("div", { className: "panel-footer muted" });
    root.replaceChildren(
      el("div", { className: "panel-header" }, el("span", { className: "panel-title" }, "Hosts"), importButton, add),
      el("div", { className: "panel-search" }, this.search),
      this.notice,
      this.list,
      this.footer,
    );
  }

  focusSearch() {
    this.search.focus();
    this.search.select();
  }

  async reload() {
    try {
      this.setView(await hostsList());
    } catch (err) {
      this.flash(`Could not load hosts: ${errorText(err)}`, true);
    }
  }

  private setView(view: HostsView) {
    this.view = view;
    this.footer.replaceChildren(el("span", {}, view.path));
    this.footer.title = `Saved hosts are stored in ${view.path}`;
    this.render();
  }

  private flash(text: string, error = false) {
    this.notice.textContent = text;
    this.notice.classList.toggle("error", error);
    this.notice.hidden = false;
    window.clearTimeout(this.noticeTimer);
    this.noticeTimer = window.setTimeout(() => (this.notice.hidden = true), 5000);
  }

  private visibleHosts() {
    return this.view.hosts.filter((h) => matches(h, this.query));
  }

  private async edit(host: Host | null, folder: string | null = null) {
    const view = await openHostDialog(host, this.view.folders, this.actions.defaultUser(), folder);
    if (!view) return;
    const existed = host !== null;
    this.setView(view);
    if (existed && !view.hosts.some((h) => h.name === host.name)) {
      this.flash(`Deleted ${host.name}.`);
    }
  }

  private async import() {
    const result = await openImportDialog();
    if (!result) return;
    this.setView(result.view);
    const { added, updated } = result.summary;
    const parts = [];
    if (added) parts.push(`added ${added} ${added === 1 ? "host" : "hosts"}`);
    if (updated) parts.push(`updated ${updated}`);
    this.flash(parts.length ? `Imported: ${parts.join(", ")}.` : "Nothing to import.");
  }

  private render() {
    const hosts = this.visibleHosts();
    if (this.view.hosts.length === 0) {
      const add = el("button", { className: "primary" }, "Add a host");
      add.onclick = () => void this.edit(null);
      const imp = el("button", {}, "Import from ~/.ssh/config");
      imp.onclick = () => void this.import();
      this.list.replaceChildren(
        el("div", { className: "empty" },
          el("p", {}, "No saved hosts yet."),
          el("p", { className: "muted" }, "Save the servers you use, or bring them in from your SSH config."),
          add,
          imp,
        ),
      );
      return;
    }
    if (hosts.length === 0) {
      this.list.replaceChildren(el("div", { className: "empty" }, el("p", { className: "muted" }, `No hosts match “${this.query}”.`)));
      return;
    }
    const tree = buildTree(hosts);
    // While searching, show every match: folders open regardless of state.
    this.list.replaceChildren(...this.renderFolder(tree, 0));
  }

  private renderFolder(folder: Folder, depth: number): HTMLElement[] {
    const out: HTMLElement[] = [];
    const folders = [...folder.folders.values()].sort((a, b) => a.name.localeCompare(b.name));
    for (const sub of folders) {
      const open = this.query !== "" || !this.collapsed.has(sub.path);
      const row = el("div", { className: "folder-row", tabIndex: 0, role: "treeitem" },
        el("span", { className: "chevron" }, open ? "▾" : "▸"),
        el("span", { className: "folder-name" }, sub.name),
        el("span", { className: "count muted" }, String(countHosts(sub))),
      );
      row.setAttribute("aria-expanded", String(open));
      row.style.paddingLeft = `${8 + depth * 14}px`;
      const toggle = () => {
        if (this.collapsed.has(sub.path)) this.collapsed.delete(sub.path);
        else this.collapsed.add(sub.path);
        saveCollapsed(this.collapsed);
        this.render();
      };
      row.onclick = toggle;
      row.onkeydown = (e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          toggle();
        } else this.arrowKeys(e);
      };
      out.push(row);
      if (open) out.push(...this.renderFolder(sub, depth + 1));
    }
    for (const host of folder.hosts) out.push(this.renderHost(host, depth));
    return out;
  }

  private renderHost(host: Host, depth: number): HTMLElement {
    const user = host.user || this.actions.defaultUser();
    const port = host.port && host.port !== 22 ? `:${host.port}` : "";
    const detail = `${user}@${host.host}${port}`;
    const edit = el("button", { className: "row-button", title: `Edit ${host.name}`, ariaLabel: `Edit ${host.name}` }, "✎");
    edit.onclick = (e) => {
      e.stopPropagation();
      void this.edit(host);
    };
    const connect = el("button", { className: "row-button", title: `Connect to ${host.name}`, ariaLabel: `Connect to ${host.name}` }, "▶");
    connect.onclick = (e) => {
      e.stopPropagation();
      this.actions.connect(host);
    };
    const files = el("button", { className: "row-button", title: `Files on ${host.name} (SFTP)`, ariaLabel: `Files on ${host.name}` }, "⇅");
    files.onclick = (e) => {
      e.stopPropagation();
      this.actions.files(host);
    };
    const row = el("div", { className: "host-row" + (this.selected === host.name ? " selected" : ""), tabIndex: 0, role: "treeitem" },
      el("span", { className: "host-icon" }, host.jump ? "⇢" : "●"),
      el("span", { className: "host-text" },
        el("span", { className: "host-name" }, host.name),
        el("span", { className: "host-detail muted" }, host.jump ? `${detail} via ${host.jump}` : detail),
        ...(host.tags?.length ? [el("span", { className: "tags" }, ...host.tags.map((t) => el("span", { className: "tag" }, t)))] : []),
      ),
      el("span", { className: "row-actions" }, edit, files, connect),
    );
    row.style.paddingLeft = `${8 + depth * 14}px`;
    row.title = host.notes ? `${host.name}\n${host.notes}` : `Double-click to connect to ${host.name}`;
    row.onclick = () => {
      this.selected = host.name;
      for (const r of this.list.querySelectorAll(".host-row.selected")) r.classList.remove("selected");
      row.classList.add("selected");
    };
    row.ondblclick = () => this.actions.connect(host);
    row.onkeydown = (e) => {
      if (e.key === "Enter") this.actions.connect(host);
      else if (e.key === "F2" || (e.key === "e" && !e.ctrlKey && !e.metaKey)) {
        // Keep the key out of the dialog's first field.
        e.preventDefault();
        void this.edit(host);
      }
      else this.arrowKeys(e);
    };
    return row;
  }

  private arrowKeys(e: KeyboardEvent) {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const rows = [...this.list.querySelectorAll<HTMLElement>(".folder-row, .host-row")];
    const i = rows.indexOf(e.currentTarget as HTMLElement);
    const next = rows[i + (e.key === "ArrowDown" ? 1 : -1)];
    if (next) next.focus();
    else if (e.key === "ArrowUp") this.search.focus();
  }
}
