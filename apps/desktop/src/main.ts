import "@xterm/xterm/css/xterm.css";
import "./styles.css";
import { appInfo, listShells, type ShellProfile } from "./ipc";
import type { TerminalTab } from "./terminal-tab";
import { PaneTab, type Direction } from "./pane-tab";
import { FilesTab } from "./files-tab";
import type { Tab } from "./tab";
import { openSshDialog, setDefaultUser, type DialogOptions } from "./ssh-dialog";
import { HostsPanel } from "./hosts-panel";
import { vaultStatus, type Host, type VaultStatus } from "./ipc";
import { onVaultChange, openVaultDialog } from "./vault-dialog";
import { listen } from "@tauri-apps/api/event";
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

function renderTabs() {
  if (!restoring) scheduleSave(snapshot);
  tabsEl.replaceChildren(
    ...tabs.map((tab, i) => {
      const el = document.createElement("div");
      el.className = "tab" + (tab === active ? " active" : "") + (tab.hasExited ? " exited" : "");
      el.setAttribute("role", "tab");
      el.setAttribute("aria-selected", String(tab === active));
      el.title = tab.tooltip;
      if (tab instanceof FilesTab) el.classList.add("files");
      el.onclick = () => activate(tab);
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
    await tab.focused.start(shellId);
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
  sidebarEl.hidden = !open;
  sidebarButton.setAttribute("aria-pressed", String(open));
  try {
    localStorage.setItem(SIDEBAR_KEY, open ? "open" : "closed");
  } catch {
    // Remembering the sidebar is a convenience only.
  }
  active?.refit();
}
sidebarButton.onclick = () => setSidebar(sidebarEl.hidden);

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
    splitItem("Split right", "Ctrl+Shift+D", "right"),
    splitItem("Split down", "Ctrl+Alt+Shift+D", "down"),
    separator,
    ...shells.map((shell) => {
      const item = document.createElement("button");
      item.className = "menu-item";
      item.setAttribute("role", "menuitem");
      item.innerHTML = `<span></span><span class="muted"></span>`;
      item.children[0].textContent = shell.name;
      item.children[1].textContent = shell.program;
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

// Keyboard shortcuts. Ctrl+Shift (Cmd on macOS) keeps plain Ctrl keys for the shell.
document.addEventListener(
  "keydown",
  (e) => {
    const mod = (e.ctrlKey || e.metaKey) && e.shiftKey;
    if (mod && e.code === "KeyT") {
      e.preventDefault();
      void newTab();
    } else if (mod && e.code === "KeyW") {
      e.preventDefault();
      void closeActive();
    } else if (mod && e.code === "KeyD") {
      e.preventDefault();
      void splitActive(e.altKey ? "down" : "right");
    } else if (e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey && e.code.startsWith("Arrow") && active instanceof PaneTab && active.paneCount > 1) {
      e.preventDefault();
      e.stopPropagation();
      const [dx, dy] = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }[e.code] ?? [0, 0];
      active.focusNeighbour(dx, dy);
    } else if (mod && e.code === "KeyH") {
      e.preventDefault();
      if (sidebarEl.hidden) setSidebar(true);
      hostsPanel.focusSearch();
    } else if (mod && e.code === "KeyF") {
      e.preventDefault();
      terminalOf(active)?.findBar.open();
    } else if (mod && e.code === "KeyE") {
      e.preventDefault();
      void openFiles();
    } else if (mod && e.code === "KeyO") {
      e.preventDefault();
      void newSshTab();
    } else if (e.ctrlKey && e.code === "Tab") {
      e.preventDefault();
      cycle(e.shiftKey ? -1 : 1);
    } else if (e.key === "Escape" && !menuEl.hidden) {
      toggleMenu(false);
    }
  },
  { capture: true },
);

new ResizeObserver(() => active?.refit()).observe(terminalsEl);

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

async function boot() {
  const [info, found] = await Promise.all([appInfo(), listShells()]);
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
  renderMenu();
  const session = await loadSession();
  if (session) {
    restoreSession(session);
  } else {
    // Plug and play: a working terminal on first launch, no wizard.
    await newTab();
  }
  restoring = false;
  renderTabs();
}

void boot();
