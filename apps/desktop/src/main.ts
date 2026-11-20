import "@xterm/xterm/css/xterm.css";
import "./styles.css";
import { appInfo, listShells, type ShellProfile } from "./ipc";
import { TerminalTab } from "./terminal-tab";
import { openSshDialog, setDefaultUser, type DialogOptions } from "./ssh-dialog";
import { HostsPanel } from "./hosts-panel";
import type { Host } from "./ipc";

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

const tabs: TerminalTab[] = [];
let active: TerminalTab | null = null;
let shells: ShellProfile[] = [];
let localUser = "";

function renderTabs() {
  tabsEl.replaceChildren(
    ...tabs.map((tab, i) => {
      const el = document.createElement("div");
      el.className = "tab" + (tab === active ? " active" : "") + (tab.hasExited ? " exited" : "");
      el.setAttribute("role", "tab");
      el.setAttribute("aria-selected", String(tab === active));
      el.title = tab.shell?.program ?? tab.title;
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

      el.append(index, label, close);
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
  const { cols, rows } = active.size;
  statusSize.textContent = `${cols}×${rows}`;
}

function activate(tab: TerminalTab) {
  if (active && active !== tab) active.hide();
  active = tab;
  tab.show();
  renderTabs();
  renderStatus();
}

async function newTab(shellId: string | null = null) {
  const tab = new TerminalTab(terminalsEl, {
    onTitle: () => renderTabs(),
    onExit: () => renderTabs(),
    onResize: (t) => {
      if (t === active) renderStatus();
    },
  });
  tabs.push(tab);
  activate(tab);
  try {
    await tab.start(shellId);
  } catch (err) {
    tab.term.write(`\x1b[31mCould not start the shell: ${String(err)}\x1b[0m\r\n`);
  }
  renderTabs();
  renderStatus();
}

/**
 * Asks for connection details (or uses a saved host's), then connects in a
 * new tab. `label` names the tab, for saved hosts.
 */
async function newSshTab(options: DialogOptions = {}, label?: string) {
  await openSshDialog(async (request) => {
    const tab = new TerminalTab(terminalsEl, {
      onTitle: () => renderTabs(),
      onExit: () => renderTabs(),
      onResize: (t) => {
        if (t === active) renderStatus();
      },
    });
    const previous = active;
    tabs.push(tab);
    activate(tab);
    try {
      await tab.startSsh(request);
      if (label) tab.title = label;
      renderTabs();
      renderStatus();
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
  active?.term.focus();
}

/** Connects to a saved host, asking only for what the host file can't hold. */
function connectHost(host: Host) {
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
  );
}

const hostsPanel = new HostsPanel(sidebarEl, { connect: connectHost, defaultUser: () => localUser });

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

async function closeTab(tab: TerminalTab) {
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
  menuEl.replaceChildren(
    ssh,
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
      if (active) void closeTab(active);
    } else if (mod && e.code === "KeyH") {
      e.preventDefault();
      if (sidebarEl.hidden) setSidebar(true);
      hostsPanel.focusSearch();
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
  statusVersion.textContent = `Setu ${info.version} · ${info.os}`;
  renderMenu();
  // Plug and play: a working terminal on first launch, no wizard.
  await newTab();
}

void boot();
