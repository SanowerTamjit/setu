// The Keys tab of the Server view: the login user's ~/.ssh/authorized_keys.
// Keys are listed by fingerprint; adding checks the pasted key first and
// removing names the line and the key, so a file that changed meanwhile is
// never edited at the wrong place. Every change asks first.
// src-tauri/src/server.rs reads and writes the file over SFTP.

import { confirmDialog, el, errorText } from "./dom";
import { icon } from "./icons";
import { serverKeyAdd, serverKeyCheck, serverKeyRemove, serverKeys, type AuthorizedKey, type AuthorizedKeyList, type NewKey } from "./ipc";
import type { PaneContext } from "./server-panes";
import { addKeyText, filterKeys, optionWords, removeKeyText } from "./server-model";

export class KeysPane {
  readonly root: HTMLElement;
  private readonly search: HTMLInputElement;
  private readonly note: HTMLElement;
  private readonly message: HTMLElement;
  private readonly table: HTMLElement;
  private readonly paste: HTMLTextAreaElement;
  private readonly preview: HTMLElement;
  private readonly addButton: HTMLButtonElement;
  private readonly pathEl: HTMLElement;
  private list: AuthorizedKeyList | null = null;
  private checked: NewKey | null = null;
  private loadedFor = "";
  private gen = 0;
  private checkGen = 0;
  private checkTimer = 0;
  private busy = false;

  constructor(private readonly ctx: PaneContext) {
    this.search = el("input", { type: "search", className: "srv-search", placeholder: "Filter by comment, fingerprint or type", spellcheck: false, autocomplete: "off" });
    this.search.oninput = () => this.render();
    const again = el("button", { type: "button", className: "card-icon", title: "Read the keys again" }, icon("retry", 14));
    again.setAttribute("aria-label", "Read the keys again");
    again.onclick = () => void this.load(true);
    this.note = el("span", { className: "muted srv-count" });
    this.message = el("div", { className: "srv-message" });
    this.table = el("div", { className: "srv-key-list", role: "list" });
    this.table.setAttribute("aria-label", "Authorized keys");
    this.paste = el("textarea", {
      className: "srv-key-paste",
      rows: 2,
      spellcheck: false,
      placeholder: "Paste a public key: ssh-ed25519 AAAA… you@laptop",
    });
    this.paste.setAttribute("aria-label", "Public key to add");
    this.paste.setAttribute("autocomplete", "off");
    this.paste.oninput = () => this.queueCheck();
    this.preview = el("p", { className: "muted srv-key-preview" }, "The key is checked here before anything is sent. Only the public key (.pub) is ever needed.");
    this.addButton = el("button", { type: "button", className: "srv-action", disabled: true }, "Add key…");
    this.addButton.onclick = () => void this.add();
    this.pathEl = el("p", { className: "muted srv-ran" });
    this.root = el(
      "section",
      { className: "host-card srv-pane" },
      el("div", { className: "srv-toolbar" }, this.search, again, this.note),
      this.message,
      this.table,
      el("div", { className: "srv-key-add" }, el("h3", {}, "Add a key"), this.paste, el("div", { className: "srv-key-add-row" }, this.preview, this.addButton)),
      this.pathEl,
    );
  }

  /** Reads the file unless it is already there for this host. */
  async load(force = false) {
    const host = this.ctx.host();
    if (!host || (!force && this.loadedFor === host)) return;
    if (this.loadedFor !== host) this.message.replaceChildren();
    this.loadedFor = host;
    const gen = ++this.gen;
    if (!this.list) this.table.replaceChildren(el("p", { className: "muted srv-loading" }, `Reading authorized_keys on ${host}…`));
    try {
      const list = await serverKeys(host);
      if (gen !== this.gen) return;
      this.show(list);
    } catch (err) {
      if (gen !== this.gen) return;
      this.list = null;
      this.note.textContent = "";
      this.table.replaceChildren(el("p", { className: "srv-problem" }, errorText(err)));
    }
  }

  /** Forgets the list (the host changed). */
  reset() {
    this.loadedFor = "";
    this.list = null;
    this.message.replaceChildren();
    this.paste.value = "";
    this.queueCheck();
  }

  private show(list: AuthorizedKeyList) {
    this.list = list;
    const n = list.keys.length;
    this.note.textContent = `${n} ${n === 1 ? "key" : "keys"}`;
    this.pathEl.textContent = `File: ${list.path}${list.missing ? " (not there yet: adding a key creates it, with ~/.ssh at 0700 and the file at 0600)" : ""}. Read and written over SFTP as the login user; Use sudo does not apply here. The new file is written beside the old one and read back before it takes its place.`;
    this.render();
  }

  private render() {
    if (!this.list) return;
    const all = this.list.keys;
    const shown = filterKeys(all, this.search.value);
    const rows = shown.map((k) => this.row(k, all.length));
    const extra = this.list.unreadable
      ? [el("p", { className: "muted srv-loading" }, `${this.list.unreadable} other ${this.list.unreadable === 1 ? "line is" : "lines are"} not a key Setu can read. ${this.list.unreadable === 1 ? "It is" : "They are"} kept as written.`)]
      : [];
    const empty = all.length
      ? "No key matches."
      : this.list.missing
        ? "This user has no authorized_keys file yet, so nobody logs in with a key."
        : "authorized_keys has no keys.";
    this.table.replaceChildren(...(rows.length ? rows : [el("p", { className: "muted srv-loading" }, empty)]), ...extra);
  }

  private row(k: AuthorizedKey, total: number): HTMLElement {
    const remove = el("button", { type: "button", className: "srv-action risky" }, "Remove…");
    remove.setAttribute("aria-label", `Remove ${k.comment || k.fingerprint}`);
    remove.onclick = () => void this.remove(k, total);
    const tags = optionWords(k.options).map((w) => el("span", { className: "srv-key-tag" }, w));
    return el(
      "div",
      { className: "srv-key-row", role: "listitem" },
      el(
        "div",
        { className: "srv-key-main" },
        el("div", { className: "srv-key-name" }, el("strong", {}, k.comment || "No comment"), el("span", { className: "muted" }, ` ${k.kind}`), ...tags),
        el("code", { className: "srv-key-fp", title: "Fingerprint, as ssh-keygen -l shows it" }, k.fingerprint),
      ),
      el("span", { className: "muted srv-key-line" }, `line ${k.line}`),
      remove,
    );
  }

  private queueCheck() {
    window.clearTimeout(this.checkTimer);
    this.checked = null;
    this.addButton.disabled = true;
    const text = this.paste.value;
    if (!text.trim()) {
      this.preview.className = "muted srv-key-preview";
      this.preview.textContent = "The key is checked here before anything is sent. Only the public key (.pub) is ever needed.";
      return;
    }
    if (/PRIVATE KEY/.test(text)) {
      // Do not keep a private key on screen or in memory longer than needed.
      this.paste.value = "";
      this.preview.className = "srv-key-preview srv-problem";
      this.preview.textContent = "That was a PRIVATE key, so Setu cleared it. Never paste it anywhere: paste the public key (the .pub file) instead.";
      return;
    }
    this.checkTimer = window.setTimeout(() => void this.check(text), 200);
  }

  private async check(text: string) {
    const gen = ++this.checkGen;
    try {
      const k = await serverKeyCheck(text);
      if (gen !== this.checkGen) return;
      const blob = k.line.split(" ")[1];
      const there = this.list?.keys.find((x) => x.key === blob);
      if (there) {
        this.checked = null;
        this.preview.className = "srv-key-preview srv-problem";
        this.preview.textContent = `This key is already in the file, on line ${there.line}${there.comment ? ` (${there.comment})` : ""}.`;
        this.addButton.disabled = true;
        return;
      }
      this.checked = k;
      this.preview.className = "srv-key-preview good";
      this.preview.replaceChildren(el("strong", {}, k.comment || "No comment"), ` ${k.kind} · `, el("code", {}, k.fingerprint));
      this.addButton.disabled = this.busy;
    } catch (err) {
      if (gen !== this.checkGen) return;
      this.checked = null;
      this.preview.className = "srv-key-preview srv-problem";
      this.preview.textContent = errorText(err);
      this.addButton.disabled = true;
    }
  }

  private changeBody(what: string, host: string, path: string, change: string, warning: string | null): HTMLElement {
    return el(
      "div",
      { className: "srv-confirm" },
      el("p", {}, what),
      el("p", { className: "muted" }, `Setu will change this file on ${host}:`),
      el("code", { className: "srv-command" }, change),
      el("p", { className: "muted net-small" }, `${path} is rewritten with every other line kept as it is.`),
      ...(warning ? [el("p", { className: "srv-warning" }, warning)] : []),
    );
  }

  private async add() {
    const k = this.checked;
    const list = this.list;
    if (!k || !list || this.busy) return;
    const host = this.ctx.host();
    const q = addKeyText(k, host);
    const change = `add ${k.kind} ${k.fingerprint}${k.comment ? ` (${k.comment})` : ""}`;
    if (!(await confirmDialog(q.title, this.changeBody(q.what, host, list.path, change, null), q.button))) return;
    await this.run(host, () => serverKeyAdd(host, this.paste.value), `Added ${k.comment || k.fingerprint}.`, true);
  }

  private async remove(k: AuthorizedKey, total: number) {
    const list = this.list;
    if (!list || this.busy) return;
    const host = this.ctx.host();
    const q = removeKeyText(k, total, host);
    const change = `remove line ${k.line}: ${k.kind} ${k.fingerprint}${k.comment ? ` (${k.comment})` : ""}`;
    if (!(await confirmDialog(q.title, this.changeBody(q.what, host, list.path, change, q.warning), q.button, true, true))) return;
    await this.run(host, () => serverKeyRemove(host, k.line, k.key), `Removed ${k.comment || k.fingerprint}.`, false);
  }

  private async run(host: string, op: () => Promise<AuthorizedKeyList>, done: string, clear: boolean) {
    this.busy = true;
    this.addButton.disabled = true;
    try {
      const list = await op();
      if (this.ctx.host() !== host) return;
      this.message.replaceChildren(el("div", { className: "net-result good srv-result", role: "status" }, el("p", { className: "net-headline" }, done)));
      if (clear) {
        this.paste.value = "";
        this.queueCheck();
      }
      this.show(list);
    } catch (err) {
      if (this.ctx.host() !== host) return;
      this.message.replaceChildren(el("div", { className: "net-result bad srv-result", role: "status" }, el("p", { className: "net-headline" }, "It did not work."), el("p", { className: "net-small" }, errorText(err))));
      await this.load(true);
    } finally {
      this.busy = false;
      this.addButton.disabled = !this.checked;
    }
  }
}
