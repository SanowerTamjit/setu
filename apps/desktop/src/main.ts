import "@xterm/xterm/css/xterm.css";
import "./styles.css";
import { appInfo, listShells, terminalHold, windowDetach, windowTake, type ShellProfile } from "./ipc";
import type { MovedPane, TerminalTab } from "./terminal-tab";
import { PaneTab, type Direction } from "./pane-tab";
import { FilesTab } from "./files-tab";
import type { Tab } from "./tab";
import { openSshDialog, setDefaultUser, type DialogOptions } from "./ssh-dialog";
import { HostsPanel } from "./hosts-panel";
import { vaultStatus, type Host, type VaultStatus } from "./ipc";
import { onVaultChange, openVaultDialog } from "./vault-dialog";
import { listen } from "@tauri-apps/api/event";
import { command, registerCommand, registerProvider, runCommand, type Command } from "./commands";
import { bindingsFor, KeyDispatcher, KEYMAPS, prefixBytes } from "./keymap";
import { openPalette } from "./palette";
import { openSettings } from "./settings-dialog";
import "./settings-shells";
import "./settings-features";
import { pasteAllowed } from "./paste-guard";
import { openKeyManager } from "./key-manager";
import { copyText } from "./dom";
import "./known-hosts";
import { openJumpMap } from "./jump-map";
import { HostsHome } from "./hosts-home";
import { WorkflowsView } from "./workflows-view";
import { enabled } from "./features";
import { icon, type IconName } from "./icons";
import { loadPacks, openRolePicker, registerSnippets } from "./packs";
import { loadSettings, onSettingsChange, settings, updateSettings, type LayoutName } from "./settings";
import { applyTheme, THEMES } from "./themes";
import { LAYOUTS } from "./layouts";
import { Studio, type SessionInfo } from "./studio";
import { dialogValues, describe, flushSave, loadSession, scheduleSave, SESSION_VERSION, type SavedPane, type SavedTab, type Session } from "./session";

const tabsEl = document.getElementById("tabs")!;
const terminalsEl = document.getElementById("terminals")!;
const newTabButton = document.getElementById("new-tab") as HTMLButtonElement;
const menuButton = document.getElementById("shell-menu-button") as HTMLButtonElement;
const menuEl = document.getElementById("shell-menu")!;
const statusShell = document.getElementById("status-shell")!;
const statusSize = document.getElementById("status-size")!;
const statusVersion = document.getElementById("status-version")!;
const sidebarEl = document.getElementById("sidebar")!;
const sidebarButton = document.getElementById("sidebar-button") as HTMLButtonElement;
sidebarButton.replaceChildren(icon("sidebar", 16));
const statusVault = document.getElementById("status-vault") as HTMLButtonElement;
const statusFiles = document.getElementById("status-files") as HTMLButtonElement;
statusFiles.onclick = () => void openFiles();

const tabs: Tab[] = [];
let active: Tab | null = null;
let shells: ShellProfile[] = [];
let localUser = "";

/** The terminal that keyboard actions apply to: the focused pane of a terminal tab. */
function terminalOf(tab: Tab | null): TerminalTab | null {
  return tab instanceof PaneTab ? tab.focused : null;
}

function createPaneTab(saved?: SavedTab, setUp?: (pane: TerminalTab, saved: SavedPane) => void) {
  return new PaneTab(
    terminalsEl,
    {
      onChange: () => {
        renderTabs();
        renderStatus();
      },
      onResize: (t) => {
        if (t === active) renderStatus();
      },
    },
    saved,
    setUp,
  );
}

/** The open terminal tabs as session.json stores them (Files tabs are not kept). */
function snapshot(): Session {
  const kept = tabs.filter((t): t is PaneTab => t instanceof PaneTab);
  const activeIndex = active instanceof PaneTab ? kept.indexOf(active) : -1;
  return { version: SESSION_VERSION, tabs: kept.map((t) => t.save()), active: Math.max(0, activeIndex) };
}

/** Set once the saved session has been put back, so it is not overwritten while loading. */
let restoring = true;

/** A window opened for a moved tab; only the main window saves the session. */
const detachedToken = new URLSearchParams(location.search).get("detached");
const isMainWindow = detachedToken === null;

interface MovedTab {
  layout: SavedTab;
  panes: MovedPane[];
}

/** Moves a tab, with its running shells, into a new window of its own. */
async function detachTab(tab: Tab | null = active) {
  if (!(tab instanceof PaneTab)) return;
  const panes = tab.panes;
  const ids = panes.flatMap((p) => (p.sessionId !== null && !p.hasExited ? [p.sessionId] : []));
  const payload: MovedTab = { layout: tab.save(), panes: panes.map((p) => p.handOver()) };
  await terminalHold(ids);
  try {
    await windowDetach(JSON.stringify(payload), tab.title);
  } catch (err) {
    // The window did not open: keep the tab here, showing output again.
    for (const p of panes) {
      const moved = payload.panes[panes.indexOf(p)];
      if (moved.sessionId !== null) void p.takeOver({ ...moved, scrollback: "" });
    }
    reportMoveError(tab, err);
    return;
  }
  const i = tabs.indexOf(tab);
  tabs.splice(i, 1);
  await tab.dispose(true);
  if (active === tab) {
    active = null;
    const next = tabs[Math.min(i, tabs.length - 1)];
    if (next) activate(next);
  }
  if (tabs.length === 0) await newTab();
  renderTabs();
  renderStatus();
}

function reportMoveError(tab: PaneTab, err: unknown) {
  tab.focused.term.write(`\r\n\x1b[31mCould not move this tab to a new window: ${String(err)}\x1b[0m\r\n`);
}

/** In a window opened for a moved tab: takes the tab over. */
async function receiveTab(token: string): Promise<boolean> {
  const text = await windowTake(token).catch(() => null);
  if (!text) return false;
  const moved = JSON.parse(text) as MovedTab;
  let i = 0;
  const groups = new Map<number, object>();
  const tab: PaneTab = createPaneTab(moved.layout, (pane, s) => {
    const m = moved.panes[i++];
    if (s.kind === "ssh") {
      if (!groups.has(s.group)) groups.set(s.group, {});
      if (!m || m.sessionId === null) {
        pane.waitForReconnect(s.connection, s.label, groups.get(s.group)!, () => reconnectPane(tab, pane));
        return;
      }
      pane.connection = s.connection;
      pane.label = s.label;
      pane.connectionGroup = groups.get(s.group)!;
    }
    if (m) void pane.takeOver(m);
  });
  tabs.push(tab);
  activate(tab);
  return true;
}

function renderTabs() {
  if (!restoring && isMainWindow) scheduleSave(snapshot);
  if (view === "hosts") hostsHome.render();
  studio.render();
  tabsEl.replaceChildren(
    ...tabs.map((tab, i) => {
      const el = document.createElement("div");
      el.className = "tab" + (tab === active ? " active" : "") + (tab.hasExited ? " exited" : "");
      el.setAttribute("role", "tab");
      el.setAttribute("aria-selected", String(tab === active));
      el.title = tab.tooltip;
      if (tab instanceof FilesTab) el.classList.add("files");
      el.onclick = () => activate(tab);
      // Drag a tab out of the window to give it a window of its own.
      if (tab instanceof PaneTab) {
        el.draggable = true;
        el.ondragend = (e) => {
          const outside =
            e.screenX < window.screenX ||
            e.screenY < window.screenY ||
            e.screenX > window.screenX + window.outerWidth ||
            e.screenY > window.screenY + window.outerHeight;
          if (outside && (e.screenX !== 0 || e.screenY !== 0)) void detachTab(tab);
        };
      }
      el.onauxclick = (e) => {
        if (e.button === 1) void closeTab(tab);
      };

      const label = document.createElement("span");
      label.className = "tab-label";
      label.textContent = tab.title;

      const index = document.createElement("span");
      index.className = "tab-index";
      index.textContent = String(i + 1);

      const close = document.createElement("button");
      close.className = "tab-close";
      close.textContent = "×";
      close.title = "Close tab (Ctrl+Shift+W)";
      close.setAttribute("aria-label", `Close ${tab.title}`);
      close.onclick = (e) => {
        e.stopPropagation();
        void closeTab(tab);
      };

      if (tab instanceof PaneTab && tab.paneCount > 1) {
        const panes = document.createElement("span");
        panes.className = "tab-panes";
        panes.textContent = `⊞ ${tab.paneCount}`;
        panes.title = `${tab.paneCount} panes`;
        label.after(panes);
        el.append(index, label, panes, close);
      } else {
        el.append(index, label, close);
      }
      return el;
    }),
  );
}

function renderStatus() {
  studio.render();
  if (!active) {
    statusShell.textContent = "";
    statusSize.textContent = "";
    return;
  }
  statusShell.textContent = active.detail;
  statusSize.textContent = active.sizeLabel;
  const term = terminalOf(active);
  statusFiles.hidden = !(term && term.isRemote && !term.hasExited && term.sessionId !== null);
}

function activate(tab: Tab) {
  if (view !== "terminals") showView("terminals");
  if (active && active !== tab) active.hide();
  active = tab;
  tab.show();
  renderTabs();
  renderStatus();
}

async function newTab(shellId: string | null = null) {
  const tab = createPaneTab();
  tabs.push(tab);
  activate(tab);
  try {
    await tab.focused.start(shellId ?? settings().defaultShell);
  } catch (err) {
    tab.focused.term.write(`\x1b[31mCould not start the shell: ${String(err)}\x1b[0m\r\n`);
  }
  renderTabs();
  renderStatus();
}

/**
 * Asks for connection details (or uses a saved host's), then connects in a
 * new tab. `label` names the tab, for saved hosts.
 */
async function newSshTab(options: DialogOptions = {}, label?: string, then?: (tab: TerminalTab) => void) {
  await openSshDialog(async (request) => {
    const tab = createPaneTab();
    const previous = active;
    tabs.push(tab);
    activate(tab);
    try {
      await tab.focused.startSsh(request);
      if (label) {
        tab.title = label;
        tab.focused.label = label;
      }
      renderTabs();
      renderStatus();
      then?.(tab.focused);
    } catch (err) {
      // Remove the half-open tab; the dialog shows what went wrong.
      tabs.splice(tabs.indexOf(tab), 1);
      await tab.dispose();
      active = null;
      if (previous) activate(previous);
      renderTabs();
      throw err;
    }
  }, options);
  // The dialog took focus (even when hidden); give it back to the terminal.
  active?.focus();
}

/** Connects to a saved host, asking only for what the host file can't hold. */
function connectHost(host: Host, then?: (tab: TerminalTab) => void) {
  const authKind = ({ auto: "default", agent: "agent", key: "keyFile", password: "password" } as const)[host.auth ?? "auto"];
  void newSshTab(
    {
      title: `Connect to ${host.name}`,
      autoConnect: true,
      prefill: {
        host: host.host,
        port: String(host.port ?? 22),
        user: host.user || localUser,
        authKind,
        keyPath: host.key_file ?? "~/.ssh/id_ed25519",
        jump: host.jump ?? "",
      },
    },
    host.name,
    then,
  );
}

/** Opens a Files tab (SFTP) on an SSH tab's connection. */
async function openFiles(from: TerminalTab | null = terminalOf(active)) {
  if (!from || !from.isRemote || from.sessionId === null || from.hasExited) return;
  const tab = new FilesTab(terminalsEl, from.sessionId, from.title, () => {
    renderTabs();
    renderStatus();
  });
  tabs.push(tab);
  activate(tab);
  try {
    await tab.start();
    tab.focus();
  } catch (err) {
    const message = document.createElement("div");
    message.className = "files-error";
    message.textContent = `Could not open files on ${from.title}: ${String(err)}`;
    tab.element.replaceChildren(message);
  }
  renderStatus();
}

const hostsPanel = new HostsPanel(sidebarEl, {
  connect: (host) => connectHost(host),
  files: (host) => connectHost(host, (tab) => void openFiles(tab)),
  defaultUser: () => localUser,
});

/** Open panes connected to a saved host, for the cards' status. */
function openCount(host: Host) {
  return tabs
    .filter((t): t is PaneTab => t instanceof PaneTab)
    .flatMap((t) => t.panes)
    .filter((p) => p.isRemote && p.label === host.name && !p.hasExited && p.sessionId !== null).length;
}

const hostsHomeEl = document.getElementById("hosts-home")!;
const hostsHome = new HostsHome(hostsHomeEl, hostsPanel, {
  connect: (host) => connectHost(host),
  files: (host) => connectHost(host, (tab) => void openFiles(tab)),
  map: () => void openJumpMap(hostsPanel.hosts, (h) => connectHost(h)),
  defaultUser: () => localUser,
  openCount,
});

const workflowsEl = document.getElementById("workflows")!;
const workflows = new WorkflowsView(workflowsEl, { hosts: () => hostsPanel.hosts });

// The hosts view, workflows and the terminals share the space right of the rail.
type View = "terminals" | "hosts" | "workflows";
let view: View = "terminals";
let sidebarWanted = true;
const railButtons = new Map<string, HTMLButtonElement>();

function showView(next: View) {
  if (next === view) {
    if (next === "hosts") hostsHome.focusSearch();
    return;
  }
  view = next;
  const hosts = next === "hosts";
  hostsHomeEl.hidden = !hosts;
  workflowsEl.hidden = next !== "workflows";
  terminalsEl.hidden = next !== "terminals";
  syncSidebar();
  sidebarButton.hidden = next !== "terminals";
  for (const [id, b] of railButtons) {
    if (id === "hosts" || id === "terminals" || id === "workflows") b.classList.toggle("current", id === next);
  }
  studio.render();
  if (hosts) {
    hostsHome.render();
    hostsHome.focusSearch();
  } else if (next === "workflows") {
    void workflows.refresh().then(() => workflows.focus());
  } else {
    active?.refit();
    active?.focus();
  }
}

function buildRail() {
  const rail = document.getElementById("rail")!;
  const item = (id: string, name: IconName, label: string, run: () => void) => {
    const b = document.createElement("button");
    b.className = "rail-button";
    b.title = label;
    b.setAttribute("aria-label", label);
    b.append(icon(name, 20));
    b.onclick = run;
    railButtons.set(id, b);
    return b;
  };
  const logo = document.createElement("div");
  logo.className = "rail-logo";
  logo.title = "Setu";
  logo.append(icon("logo", 22));
  const spacer = document.createElement("div");
  spacer.className = "rail-spacer";
  rail.replaceChildren(
    logo,
    item("hosts", "hosts", "Hosts", () => showView("hosts")),
    item("terminals", "terminal", "Terminals", () => showView("terminals")),
    ...(enabled("workflows") ? [item("workflows", "workflow", "Workflows", () => showView("workflows"))] : []),
    item("keys", "key", "SSH keys and known hosts", () => void runCommand("keys.open")),
    item("snippets", "snippet", "Snippets", () => void showPalette("Snippet ")),
    item("vault", "vault", "Vault", () => void openVaultDialog().then(() => active?.focus())),
    spacer,
    item("layout", "layout", "Switch to the Studio layout", () => void runCommand("layout.studio")),
    item("palette", "command", "Commands (Ctrl+Shift+P)", () => void showPalette()),
    item("settings", "settings", "Settings (Ctrl+,)", () => void runCommand("settings.open")),
  );
  railButtons.get("terminals")!.classList.add("current");
  // A tab moved into its own window is just terminals.
  rail.hidden = !isMainWindow;
}
buildRail();
hostsPanel.onChange(() => hostsHome.render());

// ---- Layouts: Classic (rail + tab bar) and Studio (title bar + navigator) ----

function sessionInfo(tab: Tab): SessionInfo {
  if (tab instanceof FilesTab) return { kind: "files", panes: 1, live: !tab.hasExited };
  const pane = terminalOf(tab);
  const panes = tab instanceof PaneTab ? tab.paneCount : 1;
  if (pane?.isRemote) {
    const live = pane.sessionId !== null && !pane.hasExited && !pane.awaitingReconnect;
    return { kind: "ssh", panes, live, label: pane.label ?? undefined };
  }
  return { kind: "local", panes, live: !tab.hasExited };
}

const studio: Studio = new Studio({
  tabs: () => tabs,
  active: () => active,
  view: () => view,
  info: sessionInfo,
  activate: (tab) => activate(tab),
  close: (tab) => void closeTab(tab),
  newTab: () => void newTab(),
  shellMenu: (e) => {
    e.stopPropagation();
    toggleMenu();
  },
  showView: (v) => showView(v),
  workflows: () => enabled("workflows"),
  palette: () => void showPalette(),
  run: (id) => void runCommand(id),
  can: (id) => {
    if (id === "files.open") return isRemote();
    if (id === "tab.detach") return active instanceof PaneTab && isMainWindow;
    return isTerminal();
  },
  shortcut: (id) => shortcut(id),
});
{
  const app = document.getElementById("app")!;
  app.append(studio.titlebar, studio.navigator);
  document.querySelector(".workspace")!.prepend(studio.header);
}

/** Pieces both layouts show, and where Classic keeps them. */
const shared = [sidebarEl, menuEl, statusVault, document.getElementById("status-prefix")!].map((node) => ({
  node,
  parent: node.parentElement!,
  next: node.nextSibling,
}));
const studioSlots = [studio.hostsSlot, studio.menuSlot, studio.statusSlot, studio.prefixSlot];
let layout: LayoutName = "classic";

function applyLayout(next: LayoutName) {
  layout = next;
  document.documentElement.dataset.layout = next;
  // Moved back in reverse so each one's saved neighbour is in place again.
  if (next === "studio") shared.forEach((s, i) => studioSlots[i].append(s.node));
  else for (const s of [...shared].reverse()) s.parent.insertBefore(s.node, s.next);
  toggleMenu(false);
  syncSidebar();
  studio.render();
  if (!terminalsEl.hidden) active?.refit();
}

/**
 * Classic hides the hosts sidebar when it is turned off; Studio hides the
 * whole navigator instead and keeps the hosts tree inside it.
 */
function syncSidebar() {
  if (layout === "studio") {
    // The tree is the navigator's list of hosts, so it stays beside the cards too.
    sidebarEl.hidden = false;
    studio.navigator.hidden = !sidebarWanted;
    studio.render();
  } else {
    sidebarEl.hidden = view !== "terminals" || !sidebarWanted;
    studio.navigator.hidden = false;
  }
}

function renderVault(status: VaultStatus) {
  const state = status.unlocked ? "unlocked" : status.exists ? "locked" : "none";
  const label = { unlocked: "Vault unlocked", locked: "Vault locked", none: "Set up vault" }[state];
  statusVault.replaceChildren(
    Object.assign(document.createElement("span"), { className: `dot ${state}` }),
    label,
  );
}
onVaultChange(renderVault);
statusVault.onclick = async () => {
  await openVaultDialog();
  active?.focus();
};
void listen<VaultStatus>("vault-status", (e) => renderVault(e.payload));

const SIDEBAR_KEY = "setu.sidebar";
function setSidebar(open: boolean) {
  sidebarWanted = open;
  syncSidebar();
  sidebarButton.setAttribute("aria-pressed", String(open));
  try {
    localStorage.setItem(SIDEBAR_KEY, open ? "open" : "closed");
  } catch {
    // Remembering the sidebar is a convenience only.
  }
  active?.refit();
}
sidebarButton.onclick = () => setSidebar(!sidebarWanted);

async function closeTab(tab: Tab) {
  const i = tabs.indexOf(tab);
  if (i < 0) return;
  tabs.splice(i, 1);
  await tab.dispose();
  if (active === tab) {
    active = null;
    const next = tabs[Math.min(i, tabs.length - 1)];
    if (next) activate(next);
  }
  if (tabs.length === 0) await newTab();
  renderTabs();
  renderStatus();
}

/** Splits the active tab's focused pane. */
async function splitActive(direction: Direction) {
  if (!(active instanceof PaneTab)) return;
  await active.split(direction);
}

/** Ctrl+Shift+W: closes the focused pane of a split, otherwise the tab. */
async function closeActive() {
  if (!active) return;
  if (active instanceof PaneTab && (await active.closePane())) return;
  await closeTab(active);
}

function cycle(delta: number) {
  if (!active || tabs.length < 2) return;
  const i = tabs.indexOf(active);
  activate(tabs[(i + delta + tabs.length) % tabs.length]);
}

/** Everything a new tab can start: your profiles first, then detected shells. */
function launchable(): { id: string; name: string; detail: string }[] {
  const names = new Map(shells.map((s) => [s.id, s.name]));
  return [
    ...settings().shellProfiles.map((p) => ({
      id: p.id,
      name: p.name,
      detail: `${names.get(p.base) ?? p.base} profile`,
    })),
    ...shells.map((s) => ({ id: s.id, name: s.name, detail: s.program })),
  ];
}

// Shell picker menu
function renderMenu() {
  const ssh = document.createElement("button");
  ssh.className = "menu-item";
  ssh.setAttribute("role", "menuitem");
  ssh.innerHTML = `<span>New SSH connection…</span><span class="muted">Ctrl+Shift+O</span>`;
  ssh.onclick = () => {
    toggleMenu(false);
    void newSshTab();
  };
  const separator = document.createElement("div");
  separator.className = "menu-separator";
  const splitItem = (label: string, keys: string, direction: Direction) => {
    const item = document.createElement("button");
    item.className = "menu-item";
    item.setAttribute("role", "menuitem");
    item.innerHTML = `<span></span><span class="muted"></span>`;
    item.children[0].textContent = label;
    item.children[1].textContent = keys;
    item.onclick = () => {
      toggleMenu(false);
      void splitActive(direction);
    };
    return item;
  };
  const separator2 = document.createElement("div");
  separator2.className = "menu-separator";
  menuEl.replaceChildren(
    ssh,
    separator2,
    splitItem("Split right", shortcut("pane.splitRight") ?? "", "right"),
    splitItem("Split down", shortcut("pane.splitDown") ?? "", "down"),
    separator,
    ...launchable().map((shell) => {
      const item = document.createElement("button");
      item.className = "menu-item";
      item.setAttribute("role", "menuitem");
      item.innerHTML = `<span></span><span class="muted"></span>`;
      item.children[0].textContent = shell.name;
      item.children[1].textContent = shell.detail;
      item.onclick = () => {
        toggleMenu(false);
        void newTab(shell.id);
      };
      return item;
    }),
  );
}

function toggleMenu(open = menuEl.hidden) {
  menuEl.hidden = !open;
  menuButton.setAttribute("aria-expanded", String(open));
}

newTabButton.onclick = () => void newTab();
menuButton.onclick = (e) => {
  e.stopPropagation();
  toggleMenu();
};
document.addEventListener("click", () => toggleMenu(false));

// ---- Commands, keymaps and the palette ----

function focusPane(dx: number, dy: number) {
  if (active instanceof PaneTab && active.paneCount > 1) active.focusNeighbour(dx, dy);
}

const isTerminal = () => terminalOf(active) !== null;
// Alt+arrow keys move cards on the task board, so pane focus only takes them in the terminals view.
const inTerminals = () => view === "terminals";
const isRemote = () => {
  const t = terminalOf(active);
  return !!t && t.isRemote && t.sessionId !== null && !t.hasExited;
};

for (const c of [
  { id: "palette.open", title: "Show all commands", run: () => showPalette() },
  { id: "tab.new", title: "New tab", category: "Tab", run: () => newTab() },
  { id: "ssh.new", title: "New SSH connection…", category: "Connect", run: () => newSshTab() },
  { id: "pane.splitRight", title: "Split right", category: "Pane", when: isTerminal, run: () => splitActive("right") },
  { id: "pane.splitDown", title: "Split down", category: "Pane", when: isTerminal, run: () => splitActive("down") },
  { id: "pane.close", title: "Close pane", category: "Pane", run: () => closeActive() },
  { id: "pane.focusLeft", when: inTerminals, title: "Focus pane on the left", category: "Pane", run: () => focusPane(-1, 0) },
  { id: "pane.focusRight", when: inTerminals, title: "Focus pane on the right", category: "Pane", run: () => focusPane(1, 0) },
  { id: "pane.focusUp", when: inTerminals, title: "Focus pane above", category: "Pane", run: () => focusPane(0, -1) },
  { id: "pane.focusDown", when: inTerminals, title: "Focus pane below", category: "Pane", run: () => focusPane(0, 1) },
  { id: "tab.detach", title: "Move tab to a new window", category: "Tab", keywords: "detach window", when: () => active instanceof PaneTab, run: () => detachTab() },
  { id: "tab.close", title: "Close tab", category: "Tab", run: () => (active ? closeTab(active) : undefined) },
  { id: "tab.next", title: "Next tab", category: "Tab", run: () => cycle(1) },
  { id: "tab.previous", title: "Previous tab", category: "Tab", run: () => cycle(-1) },
  {
    id: "hosts.search",
    title: "Search saved hosts",
    category: "Hosts",
    run: () => {
      if (view === "hosts") return hostsHome.focusSearch();
      if (!sidebarWanted) setSidebar(true);
      hostsPanel.focusSearch();
    },
  },
  { id: "view.hosts", title: "Go to hosts", category: "View", keywords: "cards home servers", run: () => showView("hosts") },
  { id: "view.terminals", title: "Go to terminals", category: "View", run: () => showView("terminals") },
  { id: "view.workflows", title: "Go to workflows", category: "View", keywords: "automation runbook pipeline flow", when: () => enabled("workflows"), run: () => showView("workflows") },
  {
    id: "view.board",
    title: "Open the task board",
    category: "View",
    keywords: "kanban todo cards tasks jobs",
    when: () => enabled("workflows"),
    run: () => {
      showView("workflows");
      workflows.setMode("board");
      workflows.focus();
    },
  },
  {
    id: "workflow.new",
    title: "New workflow",
    category: "Workflows",
    keywords: "automation runbook pipeline flow",
    when: () => enabled("workflows"),
    run: () => {
      showView("workflows");
      void workflows.create();
    },
  },
  { id: "sidebar.toggle", title: "Show or hide the hosts sidebar", category: "View", run: () => setSidebar(!sidebarWanted) },
  { id: "find.open", title: "Find in scrollback", category: "Terminal", when: isTerminal, run: () => terminalOf(active)?.findBar.open() },
  { id: "files.open", title: "Open Files (SFTP) on this server", category: "Files", when: isRemote, run: () => openFiles() },
  { id: "settings.open", title: "Open settings", category: "Settings", keywords: "preferences options", run: () => openSettings().then(() => active?.focus()) },
  { id: "settings.shells", title: "Shell profiles", category: "Settings", keywords: "wsl environment variables folder default shell", run: () => openSettings("shells").then(() => active?.focus()) },
  {
    id: "terminal.copy",
    title: "Copy selection",
    category: "Terminal",
    run: async () => {
      const text = terminalOf(active)?.term.getSelection();
      if (text) await copyText(text);
    },
  },
  {
    id: "terminal.paste",
    title: "Paste",
    category: "Terminal",
    run: async () => {
      const pane = terminalOf(active);
      const text = await navigator.clipboard.readText().catch(() => "");
      if (pane && text && (await pasteAllowed(pane.term, text))) pane.term.paste(text);
      pane?.focus();
    },
  },
  { id: "secrets.toggle", title: "Hide secrets in output: turn on or off", category: "Terminal", keywords: "mask redact password token privacy", run: () => updateSettings({ maskSecrets: !settings().maskSecrets }) },
  { id: "hosts.jumpMap", title: "Jump-host map", category: "Hosts", keywords: "bastion proxyjump topology diagram", run: () => openJumpMap(hostsPanel.hosts, (h) => connectHost(h)).then(() => active?.focus()) },
  { id: "hosts.import", title: "Import hosts", category: "Hosts", keywords: "putty mobaxterm termius csv ansible inventory ssh config", run: () => hostsPanel.openImport() },
  { id: "packs.open", title: "Packs and snippets", category: "Settings", keywords: "role pack install snippet", run: () => openSettings("packs").then(() => active?.focus()) },
  { id: "role.choose", title: "Choose your role", category: "Settings", keywords: "sysadmin devops secops netops pack", run: () => openRolePicker().then(() => active?.focus()) },
  { id: "keys.open", title: "Manage SSH keys", category: "SSH", keywords: "keygen ed25519 rsa agent fingerprint public key", run: () => openKeyManager().then(() => active?.focus()) },
  { id: "knownHosts.open", title: "Known hosts", category: "SSH", keywords: "known_hosts host key fingerprint forget remove", run: () => openKeyManager("known-hosts").then(() => active?.focus()) },
  { id: "snippets.open", title: "Insert a snippet", category: "Snippets", keywords: "snippet command template", run: () => showPalette("Snippet ") },
  { id: "vault.open", title: "Open the vault", category: "Vault", run: () => openVaultDialog().then(() => active?.focus()) },
] satisfies Command[]) {
  registerCommand(c);
}

for (const t of THEMES) {
  registerCommand({
    id: `theme.${t.id}`,
    title: t.name,
    category: "Theme",
    keywords: `color colour appearance ${t.description}`,
    run: () => updateSettings({ theme: t.id }),
  });
}
for (const l of LAYOUTS) {
  registerCommand({
    id: `layout.${l.id}`,
    title: `${l.name} layout`,
    category: "Layout",
    keywords: `layout view window ui switch ${l.description}`,
    run: () => updateSettings({ layout: l.id }),
  });
}
for (const k of KEYMAPS) {
  registerCommand({
    id: `keymap.${k.id}`,
    title: k.name,
    category: "Keybindings",
    keywords: `keymap shortcuts ${k.description}`,
    run: () => updateSettings({ keymap: k.id }),
  });
}

registerProvider(() =>
  hostsPanel.hosts.map((host) => ({
    id: `connect:${host.name}`,
    title: host.name,
    category: "Connect",
    keywords: `ssh ${host.user ?? ""}@${host.host} ${host.folder ?? ""} ${(host.tags ?? []).join(" ")}`,
    run: () => connectHost(host),
  })),
);
registerSnippets((command) => {
  const pane = terminalOf(active);
  if (!pane) return;
  // Typed, not run: the person reads it, edits placeholders, then presses Enter.
  // One line only, so nothing runs before Enter even without bracketed paste.
  pane.term.paste(command.replace(/[\r\n]+/g, " ").trim());
  pane.focus();
});
registerProvider(() =>
  launchable().map((shell) => ({
    id: `shell:${shell.id}`,
    title: shell.name,
    category: "New tab",
    keywords: shell.detail,
    run: () => newTab(shell.id),
  })),
);
registerProvider(() =>
  tabs.map((tab, i) => ({
    id: `goto:${i}`,
    title: `${i + 1}. ${tab.title}`,
    category: "Go to tab",
    run: () => activate(tab),
  })),
);

const statusPrefix = document.getElementById("status-prefix")!;
const keys = new KeyDispatcher((prefix) => {
  statusPrefix.hidden = !prefix;
  statusPrefix.textContent = prefix ? `${prefix} …` : "";
});
let bindings = bindingsFor("default");

function applyKeymap() {
  bindings = bindingsFor(settings().keymap);
  keys.setBindings(bindings);
  renderMenu();
}

/** The first shortcut for a command in the active keymap, for menus and the palette. */
function shortcut(id: string): string | undefined {
  return bindings[id]?.[0];
}

const RECENT_KEY = "setu.recentCommands";
function recentCommands(): string[] {
  try {
    return JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]") as string[];
  } catch {
    return [];
  }
}
function noteRecent(id: string) {
  try {
    const next = [id, ...recentCommands().filter((x) => x !== id)].slice(0, 12);
    localStorage.setItem(RECENT_KEY, JSON.stringify(next));
  } catch {
    // Recent commands are a convenience.
  }
}

let paletteOpen = false;
async function showPalette(initial?: string) {
  if (paletteOpen) return;
  paletteOpen = true;
  await openPalette({ shortcut, recent: recentCommands, onRun: noteRecent, initial });
  paletteOpen = false;
  if (!document.querySelector(".overlay")) active?.focus();
}
document.getElementById("status-palette")!.onclick = () => void showPalette();
document.getElementById("status-settings")!.onclick = () => void runCommand("settings.open");

// Keyboard shortcuts come from the active keymap. Ctrl+Shift (Cmd on macOS)
// and the Vim/Emacs prefixes keep plain keys for the shell.
document.addEventListener(
  "keydown",
  (e) => {
    if (e.key === "Escape" && !menuEl.hidden) {
      toggleMenu(false);
      return;
    }
    // Dialogs and the palette handle their own keys.
    if (document.querySelector(".overlay")) return;
    // In a text field (hosts search, find), Ctrl+X and friends keep their usual meaning.
    const t = e.target as HTMLElement | null;
    const inField = !!t && (t.tagName === "INPUT" || (t.tagName === "TEXTAREA" && !t.classList.contains("xterm-helper-textarea")));
    const result = keys.handle(e, !inField);
    if (!result) return;
    if ("passPrefix" in result) {
      const term = terminalOf(active);
      const bytes = prefixBytes(result.passPrefix);
      if (term && bytes) {
        e.preventDefault();
        term.term.input(bytes);
      }
      return;
    }
    // A shortcut whose command does not apply here leaves the key to the page.
    if ("command" in result && command(result.command)?.when?.() === false) return;
    e.preventDefault();
    e.stopPropagation();
    if ("command" in result) {
      noteRecent(result.command);
      void runCommand(result.command);
    }
  },
  { capture: true },
);

// A hidden terminal area measures 0 wide; fitting then would shrink every shell.
new ResizeObserver(() => {
  if (!terminalsEl.hidden && terminalsEl.clientWidth > 0) active?.refit();
}).observe(terminalsEl);

/**
 * Reconnects a restored SSH pane: the connect dialog stays hidden unless it
 * needs something (a password, a passphrase, a new host key). The other
 * panes of the tab that shared this connection then open shells on it.
 */
function reconnectPane(tab: PaneTab, pane: TerminalTab) {
  const connection = pane.connection;
  const group = pane.connectionGroup;
  if (!connection) return;
  const name = pane.label ?? describe(connection);
  void openSshDialog(
    async (request) => {
      await pane.startSsh(request);
      pane.connectionGroup = group;
      if (pane.label) pane.title = pane.label;
      renderTabs();
      renderStatus();
      for (const other of tab.panes) {
        if (other !== pane && other.awaitingReconnect && other.connectionGroup === group) {
          void other.startSshFrom(pane).catch((err) => {
            other.term.write(`\x1b[31mCould not open the pane: ${String(err)}\x1b[0m\r\n`);
          });
        }
      }
    },
    { title: `Reconnect to ${name}`, autoConnect: true, prefill: dialogValues(connection) },
  ).then(() => pane.focus());
}

/** Reopens last time's tabs: local shells start, SSH panes wait for Enter. */
function restoreSession(session: Session) {
  for (const saved of session.tabs) {
    // One object per saved connection group, shared by its panes.
    const groups = new Map<number, object>();
    const tab: PaneTab = createPaneTab(saved, (pane, s) => {
      if (s.kind === "local") {
        void pane.start(s.shellId).catch((err) => {
          pane.term.write(`\x1b[31mCould not start the shell: ${String(err)}\x1b[0m\r\n`);
        });
      } else {
        if (!groups.has(s.group)) groups.set(s.group, {});
        pane.waitForReconnect(s.connection, s.label, groups.get(s.group)!, () => reconnectPane(tab, pane));
      }
    });
    tabs.push(tab);
  }
  activate(tabs[session.active] ?? tabs[0]);
}

window.addEventListener("beforeunload", () => flushSave());

onSettingsChange((s, changed) => {
  if (changed.theme) applyTheme(s.theme);
  if (changed.keymap) applyKeymap();
  if (changed.layout) applyLayout(s.layout);
  if (changed.shellProfiles) renderMenu();
});

async function boot() {
  const [info, found] = await Promise.all([appInfo(), listShells(), loadSettings(), loadPacks()]);
  applyTheme(settings().theme);
  applyLayout(settings().layout);
  shells = found;
  localUser = info.user;
  setDefaultUser(info.user);
  let sidebar = "open";
  try {
    sidebar = localStorage.getItem(SIDEBAR_KEY) ?? "open";
  } catch {
    // Default to open.
  }
  setSidebar(sidebar !== "closed");
  void hostsPanel.reload();
  void vaultStatus().then(renderVault);
  statusVersion.textContent = `Setu ${info.version} · ${info.os}`;
  applyKeymap();
  if (detachedToken !== null && (await receiveTab(detachedToken))) {
    restoring = false;
    renderTabs();
    return;
  }
  const session = isMainWindow ? await loadSession() : null;
  if (session) {
    restoreSession(session);
  } else {
    // Plug and play: a working terminal on first launch, no wizard.
    await newTab();
  }
  restoring = false;
  renderTabs();
  // Asked once, over a working terminal, and easy to skip.
  if (settings().role === null) {
    await openRolePicker();
    active?.focus();
  }
}

void boot();
