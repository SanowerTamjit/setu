import "@xterm/xterm/css/xterm.css";
import "./styles.css";
import { appInfo, listShells, type ShellProfile } from "./ipc";
import { TerminalTab } from "./terminal-tab";

const tabsEl = document.getElementById("tabs")!;
const terminalsEl = document.getElementById("terminals")!;
const newTabButton = document.getElementById("new-tab") as HTMLButtonElement;
const menuButton = document.getElementById("shell-menu-button") as HTMLButtonElement;
const menuEl = document.getElementById("shell-menu")!;
const statusShell = document.getElementById("status-shell")!;
const statusSize = document.getElementById("status-size")!;
const statusVersion = document.getElementById("status-version")!;

const tabs: TerminalTab[] = [];
let active: TerminalTab | null = null;
let shells: ShellProfile[] = [];

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
  statusShell.textContent = active.shell ? `${active.shell.name} · ${active.shell.program}` : "";
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
  menuEl.replaceChildren(
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
  statusVersion.textContent = `Setu ${info.version} · ${info.os}`;
  renderMenu();
  // Plug and play: a working terminal on first launch, no wizard.
  await newTab();
}

void boot();
