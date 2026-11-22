// The hosts view: every saved host as a card, grouped by folder, with
// search, tag filters, and one-click connect and Files.

import { el } from "./dom";
import type { HostsPanel } from "./hosts-panel";
import { icon } from "./icons";
import type { Host } from "./ipc";

export interface HostsHomeActions {
  connect: (host: Host) => void;
  files: (host: Host) => void;
  map: () => void;
  defaultUser: () => string;
  /** How many open panes are connected to this host. */
  openCount: (host: Host) => number;
}

/** A stable hue for a folder, so a folder's cards share a colour. */
function hue(text: string) {
  let h = 0;
  for (const c of text) h = (h * 31 + c.charCodeAt(0)) % 360;
  return h;
}

function initials(name: string) {
  // web-01 → W1, api-prod-01 → AP, bastion → BA.
  const parts = name.split(/[^A-Za-z0-9]+/).filter(Boolean);
  if (parts.length < 2) return (parts[0] ?? name).slice(0, 2).toUpperCase();
  const second = parts[1].replace(/^0+(?=\d)/, "");
  return (parts[0][0] + second[0]).toUpperCase();
}

function matches(host: Host, words: string[]) {
  const haystack = [host.name, host.host, host.user ?? "", host.folder ?? "", host.jump ?? "", ...(host.tags ?? [])]
    .join(" ")
    .toLowerCase();
  return words.every((w) => haystack.includes(w));
}

export class HostsHome {
  private query = "";
  private tag: string | null = null;
  private readonly search: HTMLInputElement;
  private readonly chips: HTMLDivElement;
  private readonly grid: HTMLDivElement;
  private readonly notice: HTMLDivElement;
  private readonly count: HTMLSpanElement;
  private readonly footer: HTMLDivElement;
  private noticeTimer: number | undefined;

  constructor(
    private readonly root: HTMLElement,
    private readonly panel: HostsPanel,
    private readonly actions: HostsHomeActions,
  ) {
    this.search = el("input", {
      type: "search",
      className: "home-search-input",
      placeholder: "Search by name, address, tag or folder",
      spellcheck: false,
      autocomplete: "off",
    });
    this.search.setAttribute("aria-label", "Search hosts");
    this.search.oninput = () => {
      this.query = this.search.value.trim().toLowerCase();
      this.render();
    };
    this.search.onkeydown = (e) => {
      if (e.key === "Enter") {
        const first = this.visible()[0];
        if (first) this.actions.connect(first);
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        this.grid.querySelector<HTMLElement>(".host-card")?.focus();
      }
    };
    const button = (name: Parameters<typeof icon>[0], label: string, onClick: () => void, primary = false) => {
      const b = el("button", { type: "button", className: primary ? "primary with-icon" : "with-icon" }, icon(name, 15), label);
      b.onclick = onClick;
      return b;
    };
    this.count = el("span", { className: "home-count" });
    this.chips = el("div", { className: "tag-chips" });
    this.grid = el("div", { className: "home-groups" });
    this.notice = el("div", { className: "home-notice", hidden: true });
    this.footer = el("div", { className: "home-footer muted" });
    this.root.replaceChildren(
      el(
        "div",
        { className: "home-inner" },
        el(
          "header",
          { className: "home-header" },
          el("div", { className: "home-title" }, el("h1", {}, "Hosts"), this.count),
          el(
            "div",
            { className: "home-actions" },
            button("map", "Jump-host map", () => this.actions.map()),
            button("import", "Import", () => void this.panel.openImport()),
            button("plus", "New host", () => void this.panel.editHost(null), true),
          ),
        ),
        el("label", { className: "home-search" }, icon("search", 16), this.search),
        this.chips,
        this.notice,
        this.grid,
        this.footer,
      ),
    );
    panel.onChange(() => this.render());
    panel.onNotice((text, error) => this.flash(text, error));
    this.render();
  }

  focusSearch() {
    this.search.focus();
    this.search.select();
  }

  private flash(text: string, error: boolean) {
    this.notice.textContent = text;
    this.notice.classList.toggle("error", error);
    this.notice.hidden = false;
    window.clearTimeout(this.noticeTimer);
    this.noticeTimer = window.setTimeout(() => (this.notice.hidden = true), 5000);
  }

  private visible() {
    const words = this.query.split(/\s+/).filter(Boolean);
    return this.panel.hosts.filter(
      (h) => matches(h, words) && (this.tag === null || (h.tags ?? []).includes(this.tag)),
    );
  }

  /** Redraws the cards, for example when a connection opens or closes. */
  render() {
    const all = this.panel.hosts;
    const hosts = this.visible();
    this.count.textContent = all.length ? `${all.length} saved` : "";
    this.footer.textContent = this.panel.path ? `Saved in ${this.panel.path}` : "";

    const tags = [...new Set(all.flatMap((h) => h.tags ?? []))].sort((a, b) => a.localeCompare(b));
    if (this.tag && !tags.includes(this.tag)) this.tag = null;
    const chip = (label: string, value: string | null) => {
      const b = el("button", { type: "button", className: "chip" + (this.tag === value ? " on" : "") }, label);
      b.setAttribute("aria-pressed", String(this.tag === value));
      b.onclick = () => {
        this.tag = this.tag === value ? null : value;
        this.render();
      };
      return b;
    };
    this.chips.hidden = tags.length === 0;
    this.chips.replaceChildren(chip("All", null), ...tags.map((t) => chip(t, t)));

    if (all.length === 0) {
      const add = el("button", { type: "button", className: "primary" }, "Add a host");
      add.onclick = () => void this.panel.editHost(null);
      const imp = el("button", { type: "button" }, "Import hosts");
      imp.onclick = () => void this.panel.openImport();
      this.grid.replaceChildren(
        el(
          "div",
          { className: "home-empty" },
          icon("hosts", 40),
          el("h2", {}, "No saved hosts yet"),
          el("p", { className: "muted" }, "Save the servers you use, or bring them in from your SSH config, PuTTY, MobaXterm, Termius or Ansible."),
          el("div", { className: "home-empty-actions" }, imp, add),
        ),
      );
      return;
    }
    if (hosts.length === 0) {
      this.grid.replaceChildren(el("p", { className: "home-empty muted" }, "No hosts match."));
      return;
    }

    // Group by the top-level folder; the rest of the path shows on the card.
    const groups = new Map<string, Host[]>();
    for (const h of hosts) {
      const top = (h.folder ?? "").split("/")[0] || "";
      const list = groups.get(top) ?? [];
      list.push(h);
      groups.set(top, list);
    }
    const names = [...groups.keys()].sort((a, b) => (a === "" ? 1 : b === "" ? -1 : a.localeCompare(b)));
    this.grid.replaceChildren(
      ...names.map((name) =>
        el(
          "section",
          { className: "home-group" },
          el(
            "h2",
            { className: "group-title" },
            icon("folder", 14),
            name || "No folder",
            el("span", { className: "group-count" }, String(groups.get(name)!.length)),
          ),
          el(
            "div",
            { className: "card-grid" },
            ...groups
              .get(name)!
              .sort((a, b) => a.name.localeCompare(b.name))
              .map((h) => this.card(h)),
          ),
        ),
      ),
    );
  }

  private card(host: Host) {
    const user = host.user || this.actions.defaultUser();
    const port = host.port && host.port !== 22 ? `:${host.port}` : "";
    const sub = (host.folder ?? "").split("/").slice(1).join(" / ");
    const open = this.actions.openCount(host);
    const avatar = el("span", { className: "avatar" }, initials(host.name));
    avatar.style.setProperty("--hue", String(hue(host.folder || host.name)));

    const connect = el("button", { type: "button", className: "card-connect" }, icon("connect", 15), "Connect");
    connect.onclick = (e) => {
      e.stopPropagation();
      this.actions.connect(host);
    };
    const files = el("button", { type: "button", className: "card-icon", title: `Files on ${host.name} (SFTP)` }, icon("files", 15));
    files.setAttribute("aria-label", `Files on ${host.name}`);
    files.onclick = (e) => {
      e.stopPropagation();
      this.actions.files(host);
    };
    const edit = el("button", { type: "button", className: "card-icon", title: `Edit ${host.name}` }, icon("edit", 15));
    edit.setAttribute("aria-label", `Edit ${host.name}`);
    edit.onclick = (e) => {
      e.stopPropagation();
      void this.panel.editHost(host);
    };

    const card = el(
      "article",
      { className: "host-card" + (open ? " live" : ""), tabIndex: 0 },
      el(
        "div",
        { className: "card-top" },
        avatar,
        el(
          "div",
          { className: "card-heading" },
          el("span", { className: "card-name" }, host.name),
          el("span", { className: "card-address" }, `${user}@${host.host}${port}`),
        ),
        el(
          "span",
          { className: "card-status" + (open ? " on" : ""), title: open ? `${open} open` : "Not connected" },
          open ? `${open} open` : "",
        ),
      ),
      el(
        "div",
        { className: "card-meta" },
        ...(host.jump ? [el("span", { className: "card-via", title: `Through ${host.jump}` }, icon("jump", 13), host.jump)] : []),
        ...(sub ? [el("span", { className: "card-folder" }, icon("folder", 12), sub)] : []),
        ...(host.auth === "key" ? [el("span", { className: "card-auth" }, icon("key", 12), "key")] : []),
        ...(host.auth === "password" ? [el("span", { className: "card-auth" }, "password")] : []),
      ),
      ...(host.tags?.length ? [el("div", { className: "tags" }, ...host.tags.map((t) => el("span", { className: "tag" }, t)))] : []),
      el("div", { className: "card-actions" }, connect, el("span", { className: "spacer" }), files, edit),
    );
    if (host.notes) card.title = `${host.name}\n${host.notes}`;
    card.ondblclick = () => this.actions.connect(host);
    card.onkeydown = (e) => {
      if (e.target !== card) return;
      if (e.key === "Enter") this.actions.connect(host);
      else if (e.key === "F2" || e.key === "e") {
        e.preventDefault();
        void this.panel.editHost(host);
      } else if (e.key.startsWith("Arrow")) {
        e.preventDefault();
        const cards = [...this.grid.querySelectorAll<HTMLElement>(".host-card")];
        const i = cards.indexOf(card);
        const step = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : -1;
        const next = cards[i + step];
        if (next) next.focus();
        else if (step < 0) this.search.focus();
      }
    };
    return card;
  }
}
