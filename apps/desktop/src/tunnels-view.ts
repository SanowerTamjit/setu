// The Tunnels view: saved port forwards over a saved host's SSH login, each
// a card with its state, live connection counts and Start/Stop. The Rust
// side (src-tauri/src/tunnels.rs) runs them and reconnects after a drop.

import { confirmDialog, el, errorText, field, modal } from "./dom";
import { icon } from "./icons";
import {
  tunnelDelete,
  tunnelSave,
  tunnelStart,
  tunnelStop,
  tunnelsList,
  type Host,
  type TunnelEntry,
  type TunnelKind,
  type TunnelsView as TunnelsData,
} from "./ipc";
import { KIND_HELP, KIND_LABEL, canStart, countsLabel, formFor, phaseLabel, readForm, usage } from "./tunnel-form";

export interface TunnelsActions {
  hosts: () => Host[];
  /** Opens the hosts view, for an empty inventory. */
  showHosts: () => void;
}

/** How often the open view asks for fresh status. */
const POLL_MS = 1000;

export class TunnelsView {
  private data: TunnelsData = { tunnels: [], path: "" };
  private readonly list: HTMLDivElement;
  private readonly count: HTMLSpanElement;
  private readonly notice: HTMLDivElement;
  private readonly footer: HTMLDivElement;
  private readonly addButton: HTMLButtonElement;
  private timer: number | undefined;
  private noticeTimer: number | undefined;

  constructor(
    private readonly root: HTMLElement,
    private readonly actions: TunnelsActions,
  ) {
    this.count = el("span", { className: "home-count" });
    this.notice = el("div", { className: "home-notice", hidden: true });
    this.list = el("div", { className: "tunnel-list" });
    this.footer = el("div", { className: "home-footer muted" });
    this.addButton = el("button", { type: "button", className: "primary with-icon" }, icon("plus", 15), "New tunnel");
    this.addButton.onclick = () => void this.edit(null);
    this.root.replaceChildren(
      el(
        "div",
        { className: "home-inner" },
        el(
          "header",
          { className: "home-header" },
          el("div", { className: "home-title" }, el("h1", {}, "Tunnels"), this.count),
          el("div", { className: "home-actions" }, this.addButton),
        ),
        el(
          "p",
          { className: "muted tunnel-intro" },
          "Reach a database or admin page inside a network, share a local port with a server, or browse through a host. Tunnels log in the way their saved host does and reconnect if the connection drops.",
        ),
        this.notice,
        this.list,
        this.footer,
      ),
    );
  }

  focus() {
    (this.list.querySelector<HTMLElement>(".tunnel-card") ?? this.addButton).focus();
  }

  /** Loads the list and keeps it fresh while the view is showing. */
  async refresh() {
    await this.load();
    window.clearInterval(this.timer);
    this.timer = window.setInterval(() => {
      if (this.root.hidden) {
        window.clearInterval(this.timer);
        return;
      }
      void this.load();
    }, POLL_MS);
  }

  private async load() {
    try {
      this.data = await tunnelsList();
    } catch (err) {
      this.flash(errorText(err), true);
      return;
    }
    this.render();
  }

  private flash(text: string, error = false) {
    this.notice.textContent = text;
    this.notice.classList.toggle("error", error);
    this.notice.hidden = false;
    window.clearTimeout(this.noticeTimer);
    this.noticeTimer = window.setTimeout(() => (this.notice.hidden = true), 6000);
  }

  private render() {
    const { tunnels, path, problem } = this.data;
    const running = tunnels.filter((t) => t.status.phase === "running").length;
    this.count.textContent = tunnels.length ? `${tunnels.length} saved${running ? ` · ${running} running` : ""}` : "";
    this.footer.textContent = path ? `Saved in ${path}` : "";
    this.addButton.disabled = Boolean(problem);
    if (problem) {
      this.list.replaceChildren(
        el("div", { className: "home-notice error" }, `${problem}. Fix or move the file; Setu will not write over it.`),
      );
      return;
    }
    if (tunnels.length === 0) {
      const hosts = this.actions.hosts().length > 0;
      const add = el("button", { type: "button", className: "primary" }, hosts ? "New tunnel" : "Add a host first");
      add.onclick = () => (hosts ? void this.edit(null) : this.actions.showHosts());
      this.list.replaceChildren(
        el(
          "div",
          { className: "home-empty" },
          icon("tunnel", 40),
          el("h2", {}, "No tunnels yet"),
          el(
            "p",
            { className: "muted" },
            "A tunnel carries connections through SSH: open a database that only listens on the server, or browse as if you were in its network.",
          ),
          el("div", { className: "home-empty-actions" }, add),
        ),
      );
      return;
    }
    // Keep focus on the same card across the once-a-second redraw.
    const focused = (document.activeElement as HTMLElement | null)?.closest<HTMLElement>(".tunnel-card")?.dataset.id;
    const focusedButton = (document.activeElement as HTMLElement | null)?.dataset.role;
    this.list.replaceChildren(...tunnels.map((t) => this.card(t)));
    if (focused) {
      const card = this.list.querySelector<HTMLElement>(`.tunnel-card[data-id="${CSS.escape(focused)}"]`);
      const target = focusedButton ? card?.querySelector<HTMLElement>(`[data-role="${focusedButton}"]`) : card;
      (target ?? card)?.focus();
    }
  }

  private card(t: TunnelEntry) {
    const phase = t.status.phase;
    const toggle = el(
      "button",
      { type: "button", className: canStart(t) ? "card-connect" : "card-connect stop" },
      icon(canStart(t) ? "play" : "stop", 15),
      canStart(t) ? "Start" : "Stop",
    );
    toggle.dataset.role = "toggle";
    toggle.onclick = () => void this.toggle(t);
    const edit = el("button", { type: "button", className: "card-icon", title: `Edit ${t.name}` }, icon("edit", 15));
    edit.setAttribute("aria-label", `Edit ${t.name}`);
    edit.dataset.role = "edit";
    edit.onclick = () => void this.edit(t);

    const counts = countsLabel(t.status);
    const card = el(
      "article",
      { className: `tunnel-card host-card phase-${phase}`, tabIndex: 0 },
      el(
        "div",
        { className: "card-top" },
        el("span", { className: `avatar tunnel-kind kind-${t.kind}`, title: KIND_LABEL[t.kind] }, icon(t.kind === "dynamic" ? "connect" : "tunnel", 18)),
        el(
          "div",
          { className: "card-heading" },
          el("span", { className: "card-name" }, t.name),
          el("span", { className: "card-address" }, `${KIND_LABEL[t.kind]} · ${t.host}`),
        ),
        el("span", { className: `tunnel-phase phase-${phase}` }, phaseLabel(t.status)),
      ),
      el("code", { className: "tunnel-summary", title: "The same tunnel as an ssh command-line option" }, t.summary),
      el("p", { className: "tunnel-usage" }, usage(t)),
      ...(t.status.message ? [el("p", { className: `tunnel-message phase-${phase}` }, t.status.message)] : []),
      el(
        "div",
        { className: "card-meta" },
        ...(t.shared ? [el("span", { className: "card-auth tunnel-shared", title: "Other computers on the network can connect to this port" }, "open to the network")] : []),
        ...(t.autoStart ? [el("span", { className: "card-auth" }, icon("clock", 12), "starts with Setu")] : []),
        ...(counts ? [el("span", { className: "tunnel-counts" }, counts)] : []),
      ),
      el("div", { className: "card-actions" }, toggle, el("span", { className: "spacer" }), edit),
    );
    card.dataset.id = t.id;
    card.onkeydown = (e) => {
      if (e.target !== card) return;
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        void this.toggle(t);
      } else if (e.key === "F2" || e.key === "e") {
        e.preventDefault();
        void this.edit(t);
      }
    };
    return card;
  }

  private async toggle(t: TunnelEntry) {
    try {
      if (canStart(t)) await tunnelStart(t.id);
      else await tunnelStop(t.id);
    } catch (err) {
      this.flash(errorText(err), true);
    }
    await this.load();
  }

  private async edit(existing: TunnelEntry | null) {
    const saved = await openTunnelDialog(existing, this.actions.hosts());
    if (saved) {
      this.flash(saved);
      await this.load();
    }
  }
}

/** Add or edit a tunnel. Resolves with a notice to show, or null when cancelled. */
function openTunnelDialog(existing: TunnelEntry | null, hosts: Host[]): Promise<string | null> {
  return new Promise((resolve) => {
    let result: string | null = null;
    const f = formFor(existing, hosts[0]?.name ?? "");

    const name = el("input", { value: f.name, placeholder: "Postgres on db-01", autocomplete: "off", spellcheck: false });
    const host = el("select", {}, ...hosts.map((h) => el("option", { value: h.name }, h.name)));
    if (existing && !hosts.some((h) => h.name === existing.host)) host.prepend(el("option", { value: existing.host }, `${existing.host} (not saved any more)`));
    host.value = f.host;
    const kind = el(
      "select",
      {},
      ...(["local", "dynamic", "remote"] as const).map((k) => el("option", { value: k }, `${KIND_LABEL[k]}  (ssh ${k === "local" ? "-L" : k === "remote" ? "-R" : "-D"})`)),
    );
    kind.value = f.kind;
    const kindHelp = el("small", { className: "muted" });
    const bindPort = el("input", { value: f.bindPort, placeholder: "5433", inputMode: "numeric", autocomplete: "off" });
    const bindField = field("Listen on port", bindPort);
    const targetHost = el("input", { value: f.targetHost, placeholder: "127.0.0.1", autocomplete: "off", spellcheck: false });
    const targetPort = el("input", { value: f.targetPort, placeholder: "5432", inputMode: "numeric", autocomplete: "off" });
    const targetLabel = el("span", {});
    const targetRow = el(
      "div",
      { className: "row" },
      el("label", { className: "field" }, targetLabel, targetHost),
      field("Port", targetPort),
    );
    const shared = el("input", { type: "checkbox", checked: f.shared });
    const sharedText = el("span", {});
    const sharedWarn = el("small", { className: "tunnel-warning" });
    const autoStart = el("input", { type: "checkbox", checked: f.autoStart });

    const sync = () => {
      const k = kind.value as TunnelKind;
      kindHelp.textContent = KIND_HELP[k];
      targetRow.hidden = k === "dynamic";
      bindField.querySelector("span")!.textContent = k === "remote" ? "Listen on server port" : "Listen on port (this computer)";
      targetLabel.textContent = k === "remote" ? "Destination, as this computer sees it" : "Destination, as the server sees it";
      sharedText.textContent =
        k === "remote" ? "Let other computers reach the server's port (needs GatewayPorts on the server)" : "Let other computers on my network use this tunnel";
      sharedWarn.hidden = !shared.checked;
      sharedWarn.textContent =
        k === "dynamic"
          ? "Anyone who can reach this computer could then browse through your login, with no password. Leave this off unless you know the network."
          : "Anyone who can reach the port could then use your login to reach the destination. Leave this off unless you know the network.";
    };
    kind.onchange = sync;
    shared.onchange = sync;
    sync();

    const message = el("div", { className: "dialog-message error", hidden: true });
    const save = el("button", { type: "submit", className: "primary" }, existing ? "Save" : "Add tunnel");
    const cancel = el("button", { type: "button" }, "Cancel");
    const remove = el("button", { type: "button", className: "danger-button", hidden: !existing }, "Delete");
    const form = el(
      "form",
      { className: "dialog tunnel-dialog" },
      el("h2", {}, existing ? `Edit ${existing.name}` : "New tunnel"),
      field("Name", name),
      el("div", { className: "row even" }, field("Through host", host), el("label", { className: "field" }, el("span", {}, "Kind"), kind)),
      kindHelp,
      bindField,
      targetRow,
      el("label", { className: "check-row" }, shared, sharedText),
      sharedWarn,
      el("label", { className: "check-row" }, autoStart, el("span", {}, "Start when Setu opens")),
      ...(existing && !canStart(existing) ? [el("small", { className: "muted" }, "Changes apply the next time the tunnel starts.")] : []),
      message,
      el("div", { className: "actions" }, remove, el("span", { className: "spacer" }), cancel, save),
    );
    const { close } = modal(form, () => resolve(result));
    cancel.onclick = close;
    name.focus();

    const fail = (text: string) => {
      message.hidden = false;
      message.textContent = text;
    };
    remove.onclick = async () => {
      if (!existing) return;
      const yes = await confirmDialog("Delete tunnel", `Delete ${existing.name}? A running tunnel stops first.`, "Delete", true, true);
      if (!yes) return;
      try {
        await tunnelDelete(existing.id);
        result = `Deleted ${existing.name}.`;
        close();
      } catch (err) {
        fail(errorText(err));
      }
    };
    form.onsubmit = async (e) => {
      e.preventDefault();
      const read = readForm({
        id: existing?.id ?? "",
        name: name.value,
        host: host.value,
        kind: kind.value as TunnelKind,
        bindPort: bindPort.value,
        shared: shared.checked,
        targetHost: targetHost.value,
        targetPort: targetPort.value,
        autoStart: autoStart.checked,
      });
      if ("error" in read) return fail(read.error);
      save.disabled = true;
      try {
        await tunnelSave(read.tunnel);
        result = existing ? `Saved ${read.tunnel.name}.` : `Added ${read.tunnel.name}. Press Start to open it.`;
        close();
      } catch (err) {
        fail(errorText(err));
      } finally {
        save.disabled = false;
      }
    };
  });
}
