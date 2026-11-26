// The Studio layout: a second way to lay out the window, next to the Classic
// rail and tab bar. A title bar with a search box for everything, a
// navigator that lists open sessions above the saved hosts, and a header over
// the terminal with the actions for that session.
//
// Studio does not own any tabs or panels. It shows the same tabs as the tab
// bar, and while it is on, main.ts moves a few shared pieces (the hosts tree,
// the shell menu, the vault button) into the slots it offers here.

import { el } from "./dom";
import { icon, type IconName } from "./icons";
import type { Tab } from "./tab";

export type StudioView = "terminals" | "hosts" | "workflows" | "tunnels" | "network" | "monitor" | "server";

export type SessionKind = "local" | "ssh" | "files";

export interface SessionInfo {
  kind: SessionKind;
  /** Panes in a split terminal tab (1 when not split). */
  panes: number;
  /** An SSH pane that is connected right now. */
  live: boolean;
  /** The saved host's name, for a tab opened from a saved host. */
  label?: string;
}

export interface StudioActions {
  tabs: () => readonly Tab[];
  active: () => Tab | null;
  view: () => StudioView;
  info: (tab: Tab) => SessionInfo;
  activate: (tab: Tab) => void;
  close: (tab: Tab) => void;
  newTab: () => void;
  /** Opens or closes the shell menu (moved into `menuSlot`). */
  shellMenu: (e: MouseEvent) => void;
  showView: (view: StudioView) => void;
  /** Whether the Workflows view is turned on (feature flag). */
  workflows: () => boolean;
  /** Whether the Tunnels view is turned on (feature flag). */
  tunnels: () => boolean;
  network: () => boolean;
  monitor: () => boolean;
  server: () => boolean;
  palette: () => void;
  /** Runs a command by id; `can` says whether it applies right now. */
  run: (id: string) => void;
  can: (id: string) => boolean;
  shortcut: (id: string) => string | undefined;
}

const KIND_LABEL: Record<SessionKind, string> = { local: "Local", ssh: "SSH", files: "Files" };
const KIND_ICON: Record<SessionKind, IconName> = { local: "terminal", ssh: "connect", files: "files" };

export class Studio {
  /** The title bar across the top. */
  readonly titlebar: HTMLElement;
  /** The column on the left: sessions, then the hosts tree. */
  readonly navigator: HTMLElement;
  /** The strip above the terminal. */
  readonly header: HTMLElement;
  /** Where the hosts tree goes while Studio is on. */
  readonly hostsSlot: HTMLElement;
  /** Where the shell menu goes, next to the New button. */
  readonly menuSlot: HTMLElement;
  /** Right side of the title bar, for the vault button. */
  readonly statusSlot: HTMLElement;
  /** Shows a keymap prefix (Vim/Emacs) while one is pending. */
  readonly prefixSlot: HTMLElement;

  private readonly navToggle: HTMLButtonElement;
  private readonly sessions: HTMLElement;
  private readonly sessionCount: HTMLElement;
  private readonly viewButtons = new Map<StudioView, HTMLButtonElement>();
  private readonly searchKeys: HTMLElement;
  private readonly headerMain: HTMLElement;
  private readonly headerActions: HTMLElement;

  constructor(private readonly actions: StudioActions) {
    // ---- Title bar ----
    const brand = el("div", { className: "st-brand" }, icon("logo", 20), el("span", {}, "Setu"));
    const views = el("div", { className: "st-views", role: "tablist" });
    views.setAttribute("aria-label", "View");
    for (const [id, label, name] of [
      ["terminals", "Terminals", "terminal"],
      ["hosts", "Hosts", "hosts"],
      ...(actions.workflows() ? ([["workflows", "Workflows", "workflow"]] as const) : []),
      ...(actions.tunnels() ? ([["tunnels", "Tunnels", "tunnel"]] as const) : []),
      ...(actions.network() ? ([["network", "Network", "network"]] as const) : []),
      ...(actions.monitor() ? ([["monitor", "Monitoring", "pulse"]] as const) : []),
      ...(actions.server() ? ([["server", "Server", "server"]] as const) : []),
    ] as const) {
      const b = el("button", { type: "button", className: "st-view" }, icon(name, 15), label);
      b.setAttribute("role", "tab");
      b.onclick = () => actions.showView(id);
      this.viewButtons.set(id, b);
      views.append(b);
    }
    this.searchKeys = el("kbd", { className: "st-search-keys" });
    const search = el(
      "button",
      { type: "button", className: "st-search", title: "Search hosts, commands and snippets" },
      icon("search", 15),
      el("span", { className: "st-search-text" }, "Search hosts, commands, snippets…"),
      this.searchKeys,
    );
    search.onclick = () => actions.palette();
    this.statusSlot = el("div", { className: "st-status" });
    this.navToggle = this.tool("sidebar", "Show or hide the navigator", () => actions.run("sidebar.toggle"));
    this.titlebar = el(
      "header",
      { className: "st-titlebar studio-only" },
      this.navToggle,
      brand,
      views,
      search,
      el(
        "div",
        { className: "st-tools" },
        this.statusSlot,
        this.tool("key", "SSH keys and known hosts", () => actions.run("keys.open")),
        this.tool("snippet", "Snippets", () => actions.run("snippets.open")),
        this.tool("layout", "Switch to the Classic layout", () => actions.run("layout.classic")),
        this.tool("settings", "Settings (Ctrl+,)", () => actions.run("settings.open")),
      ),
    );

    // ---- Navigator ----
    this.menuSlot = el("div", { className: "st-new" });
    const newButton = el("button", { type: "button", className: "st-new-button" }, icon("plus", 15), "New terminal");
    newButton.title = "New tab (Ctrl+Shift+T)";
    newButton.onclick = () => actions.newTab();
    const ssh = el("button", { type: "button", className: "st-new-ssh", title: "New SSH connection (Ctrl+Shift+O)" }, icon("connect", 15), "SSH");
    ssh.onclick = () => actions.run("ssh.new");
    const more = el("button", { type: "button", className: "st-new-more", title: "Choose a shell" }, icon("chevronDown", 15));
    more.setAttribute("aria-label", "Choose a shell");
    more.setAttribute("aria-haspopup", "menu");
    more.onclick = (e) => actions.shellMenu(e);
    this.menuSlot.append(newButton, more, ssh);
    this.sessionCount = el("span", { className: "count" });
    this.sessions = el("div", { className: "st-sessions", role: "tablist" });
    this.sessions.setAttribute("aria-label", "Open sessions");
    this.sessions.setAttribute("aria-orientation", "vertical");
    this.hostsSlot = el("div", { className: "st-hosts" });
    this.navigator = el(
      "aside",
      { className: "st-navigator studio-only" },
      this.menuSlot,
      el("div", { className: "st-section-title" }, el("span", {}, "Open sessions"), this.sessionCount),
      this.sessions,
      this.hostsSlot,
    );
    this.navigator.setAttribute("aria-label", "Sessions and hosts");

    // ---- Session header ----
    this.headerMain = el("div", { className: "st-header-main" });
    this.prefixSlot = el("div", { className: "st-prefix" });
    this.headerActions = el("div", { className: "st-header-actions" });
    this.header = el("div", { className: "st-header studio-only" }, this.headerMain, this.prefixSlot, this.headerActions);
  }

  private tool(name: IconName, label: string, run: () => void) {
    const b = el("button", { type: "button", className: "st-tool", title: label }, icon(name, 17));
    b.setAttribute("aria-label", label);
    b.onclick = run;
    return b;
  }

  /** Redraws the session list and the header; cheap, called on every change. */
  render() {
    const tabs = this.actions.tabs();
    const active = this.actions.active();
    const view = this.actions.view();
    for (const [id, b] of this.viewButtons) {
      b.classList.toggle("current", id === view);
      b.setAttribute("aria-selected", String(id === view));
    }
    this.searchKeys.textContent = this.actions.shortcut("palette.open") ?? "";
    this.searchKeys.hidden = !this.searchKeys.textContent;
    this.navToggle.setAttribute("aria-pressed", String(!this.navigator.hidden));
    this.sessionCount.textContent = String(tabs.length);
    this.sessions.replaceChildren(...tabs.map((tab, i) => this.row(tab, i, tab === active && view === "terminals")));
    this.renderHeader(active);
  }

  private row(tab: Tab, index: number, current: boolean) {
    const info = this.actions.info(tab);
    const state = tab.hasExited ? "exited" : info.kind === "ssh" ? (info.live ? "live" : "waiting") : "local";
    const row = el("div", { className: `st-session ${state}` + (current ? " current" : "") });
    row.setAttribute("role", "tab");
    row.setAttribute("aria-selected", String(current));
    row.tabIndex = current ? 0 : -1;
    row.title = tab.tooltip;
    row.onclick = () => this.actions.activate(tab);
    row.onkeydown = (e) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        this.actions.activate(tab);
      } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const rows = [...this.sessions.querySelectorAll<HTMLElement>(".st-session")];
        rows[(index + (e.key === "ArrowDown" ? 1 : -1) + rows.length) % rows.length]?.focus();
      }
    };
    row.onauxclick = (e) => {
      if (e.button === 1) this.actions.close(tab);
    };
    const close = el("button", { type: "button", className: "st-session-close", title: "Close (Ctrl+Shift+W)" }, icon("close", 13));
    close.setAttribute("aria-label", `Close ${tab.title}`);
    close.onclick = (e) => {
      e.stopPropagation();
      this.actions.close(tab);
    };
    const badges = el("span", { className: "st-session-badges" });
    if (info.panes > 1) badges.append(el("span", { className: "st-badge", title: `${info.panes} panes` }, icon("splitRight", 12), String(info.panes)));
    row.append(
      el("span", { className: `st-session-icon ${info.kind}` }, icon(KIND_ICON[info.kind], 15), el("span", { className: "st-dot" })),
      el(
        "span",
        { className: "st-session-text" },
        el("span", { className: "st-session-title" }, info.label ?? tab.title),
        el("span", { className: "st-session-detail" }, (info.label && info.label !== tab.title ? tab.title : tab.detail) || KIND_LABEL[info.kind]),
      ),
      badges,
      close,
    );
    return row;
  }

  private renderHeader(active: Tab | null) {
    const showing = !!active && this.actions.view() === "terminals";
    this.header.hidden = !showing;
    if (!active || !showing) return;
    const info = this.actions.info(active);
    const state = active.hasExited ? "Exited" : info.kind === "ssh" ? (info.live ? "Connected" : "Not connected") : KIND_LABEL[info.kind];
    this.headerMain.replaceChildren(
      el("span", { className: `st-header-icon ${info.kind}` }, icon(KIND_ICON[info.kind], 16)),
      el("strong", { className: "st-header-title" }, info.label ?? active.title),
      el("span", { className: `st-pill ${active.hasExited ? "exited" : info.kind === "ssh" && !info.live ? "waiting" : info.kind}` }, state),
      el("span", { className: "st-header-detail" }, [active.detail, active.sizeLabel].filter(Boolean).join("  ·  ")),
    );
    const action = (id: string, name: IconName, label: string) => {
      const keys = this.actions.shortcut(id);
      const b = el("button", { type: "button", className: "st-action", title: keys ? `${label} (${keys})` : label }, icon(name, 15));
      b.setAttribute("aria-label", label);
      b.onclick = () => this.actions.run(id);
      return b;
    };
    const buttons: HTMLElement[] = [];
    if (this.actions.can("files.open")) {
      const files = action("files.open", "files", "Files");
      files.classList.add("labelled");
      files.append("Files");
      buttons.push(files);
    }
    if (this.actions.can("pane.splitRight")) {
      buttons.push(action("pane.splitRight", "splitRight", "Split right"), action("pane.splitDown", "splitDown", "Split down"));
    }
    if (this.actions.can("find.open")) buttons.push(action("find.open", "search", "Find in scrollback"));
    if (this.actions.can("tab.detach")) buttons.push(action("tab.detach", "window", "Move to a new window"));
    this.headerActions.replaceChildren(...buttons);
  }
}
