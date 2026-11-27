// The Problems view: what needs attention across saved hosts in one list,
// like a code editor's problems list. Failed services, full disks, a
// restart needed and a drifting clock come from a read-only health check
// over SSH; alerts from Monitoring and failed workflow runs join them.
// src-tauri/src/problems.rs does the work; the logic for filtering and
// grouping is in problems-model.ts.

import { el, errorText } from "./dom";
import { icon, type IconName } from "./icons";
import { problemsCheck, problemsList, problemsSnooze, problemsUnsnooze, type Finding, type Host, type ProblemAction, type ProblemsList, type Severity, type SnoozedFinding } from "./ipc";
import { SNOOZES, actionLabel, agoWords, badgeText, checkedWords, filterFindings, groupByHost, progressWords, severityLabel, sourceLabel, untilWords, type SeverityFilter } from "./problems-model";

export interface ProblemsActions {
  hosts: () => Host[];
  /** Opens the hosts view, for an empty inventory. */
  showHosts: () => void;
  /** Opens what a problem is about (a service, Monitoring, a workflow…). */
  open: (action: ProblemAction) => void;
  /** The number of errors and warnings changed (for the rail badge). */
  badge: (text: string) => void;
}

const SEVERITY_ICON: Record<Severity, IconName> = { error: "error", warning: "problems", info: "info" };

export class ProblemsView {
  private data: ProblemsList | null = null;
  private severity: SeverityFilter = "all";
  private hostFilter = "";
  private progress = "";
  /** Hosts in the check that is running. */
  private total = 0;
  private snoozedOpen = false;
  private problem = "";
  private readonly chips = new Map<SeverityFilter, HTMLButtonElement>();
  private readonly hostSelect: HTMLSelectElement;
  private readonly search: HTMLInputElement;
  private readonly checkButton: HTMLButtonElement;
  private readonly status: HTMLElement;
  private readonly body: HTMLElement;

  constructor(
    private readonly root: HTMLElement,
    private readonly actions: ProblemsActions,
  ) {
    const chips = el("div", { className: "srv-filters prob-filters", role: "group" });
    chips.setAttribute("aria-label", "Show");
    for (const f of ["all", "error", "warning", "info"] as const) {
      const b = el("button", { type: "button", className: "srv-chip" });
      b.onclick = () => {
        this.severity = f;
        this.render();
      };
      this.chips.set(f, b);
      chips.append(b);
    }
    this.hostSelect = el("select", { className: "net-from-select" });
    this.hostSelect.setAttribute("aria-label", "Host");
    this.hostSelect.onchange = () => {
      this.hostFilter = this.hostSelect.value;
      this.render();
    };
    this.search = el("input", { type: "search", className: "srv-search prob-search", placeholder: "Filter problems", spellcheck: false, autocomplete: "off" });
    this.search.oninput = () => this.render();
    this.checkButton = el("button", { type: "button", className: "primary with-icon" }, icon("retry", 14), el("span", {}, "Check all hosts"));
    this.checkButton.onclick = () => void this.check([]);
    this.status = el("p", { className: "muted prob-status", role: "status" });
    this.body = el("div", { className: "prob-body" });
    this.root.replaceChildren(
      el(
        "div",
        { className: "home-inner prob-inner" },
        el(
          "header",
          { className: "home-header" },
          el("div", { className: "home-title" }, el("h1", {}, "Problems")),
          el("div", { className: "home-actions" }, el("label", { className: "net-from" }, el("span", { className: "muted" }, "Host"), this.hostSelect), this.checkButton),
        ),
        el(
          "p",
          { className: "muted tunnel-intro" },
          "What needs attention across your saved hosts: failed services, disks filling up, a restart needed after updates and clocks out of sync, with alerts from Monitoring and workflows that failed. The check only reads; it logs in the way monitoring does and changes nothing.",
        ),
        el("div", { className: "prob-toolbar" }, chips, this.search),
        this.status,
        this.body,
      ),
    );
    this.render();
  }

  focus() {
    (this.data?.findings.length ? this.search : this.checkButton).focus();
  }

  /** Reads the list again (cheap: nothing is checked). */
  async refresh() {
    try {
      this.data = await problemsList();
      this.problem = "";
    } catch (err) {
      this.problem = errorText(err);
    }
    this.actions.badge(this.data ? badgeText(this.data.counts) : "");
    // Do not pull an open snooze menu out from under the pointer.
    const focused = document.activeElement;
    if (focused instanceof HTMLSelectElement && this.body.contains(focused)) return;
    this.render();
  }

  /** Checks the named hosts (all saved hosts when empty). */
  async check(names: string[]) {
    if (this.data?.checking || this.progress) return;
    this.progress = "Checking…";
    this.render();
    try {
      await problemsCheck(names, (e) => {
        if (e.kind === "started") {
          this.total = e.total;
          this.progress = progressWords(0, e.total);
        } else if (e.kind === "host") {
          this.progress = progressWords(e.done, this.total);
          void this.refresh();
        }
      });
    } catch (err) {
      this.problem = errorText(err);
    }
    this.progress = "";
    this.total = 0;
    await this.refresh();
  }


  private render() {
    const now = Date.now();
    const hosts = this.actions.hosts().map((h) => h.name);
    const d = this.data;
    const all = d?.findings ?? [];
    // Host picker: every saved host, keeping the choice while it exists.
    if (this.hostFilter && !hosts.includes(this.hostFilter)) this.hostFilter = "";
    this.hostSelect.replaceChildren(el("option", { value: "" }, "All hosts"), ...hosts.map((n) => el("option", { value: n }, n)));
    this.hostSelect.value = this.hostFilter;
    const byHost = all.filter((f) => !this.hostFilter || f.host === this.hostFilter);
    const n = (s: Severity) => byHost.filter((f) => f.severity === s).length;
    const words: Record<SeverityFilter, string> = { all: `All ${byHost.length}`, error: `Errors ${n("error")}`, warning: `Warnings ${n("warning")}`, info: `Info ${n("info")}` };
    for (const [f, b] of this.chips) {
      b.textContent = words[f];
      b.classList.toggle("current", f === this.severity);
      b.classList.toggle("srv-chip-failed", f === "error" && n("error") > 0);
      b.setAttribute("aria-pressed", String(f === this.severity));
    }
    const busy = Boolean(this.progress) || Boolean(d?.checking);
    this.checkButton.disabled = busy || !hosts.length;
    (this.checkButton.lastElementChild as HTMLElement).textContent = this.hostFilter ? `Check ${this.hostFilter}` : "Check all hosts";
    this.checkButton.onclick = () => void this.check(this.hostFilter ? [this.hostFilter] : []);
    this.status.textContent = this.progress || (d ? checkedWords(d, hosts.length, now) : "");
    this.status.classList.toggle("prob-busy", busy);

    if (!hosts.length && !all.length) {
      const add = el("button", { type: "button", className: "primary" }, "Add a host");
      add.onclick = () => this.actions.showHosts();
      this.body.replaceChildren(el("div", { className: "home-empty" }, el("p", {}, "Save a host first: Problems checks your saved hosts."), add));
      return;
    }
    const shown = filterFindings(all, { severity: this.severity, host: this.hostFilter, text: this.search.value });
    const parts: HTMLElement[] = [];
    if (this.problem) parts.push(el("p", { className: "srv-problem" }, this.problem));
    if (!shown.length) parts.push(this.emptyState(all.length > 0, now));
    for (const g of groupByHost(shown)) {
      const head = el(
        "div",
        { className: "prob-group-head" },
        el("span", { className: `prob-dot sev-${g.worst}` }),
        el("h2", {}, g.host ?? "Not about one host"),
        el("span", { className: "muted prob-group-count" }, g.findings.length === 1 ? "1 problem" : `${g.findings.length} problems`),
      );
      if (g.host) {
        const again = el("button", { type: "button", className: "link-button" }, "Check again");
        again.disabled = busy;
        const host = g.host;
        again.onclick = () => void this.check([host]);
        head.append(again);
      }
      parts.push(el("section", { className: "host-card prob-group" }, head, ...g.findings.map((f) => this.row(f, now))));
    }
    const snoozed = (d?.snoozed ?? []).filter((f) => !this.hostFilter || f.host === this.hostFilter);
    if (snoozed.length) parts.push(this.snoozedList(snoozed, now));
    this.body.replaceChildren(...parts);
  }

  private emptyState(filtered: boolean, now: number): HTMLElement {
    const d = this.data;
    if (filtered) return el("p", { className: "muted prob-empty" }, "No problem matches the filter.");
    if (!d || !d.checked) {
      return el(
        "div",
        { className: "home-empty prob-empty" },
        el("p", {}, "Nothing to show yet. Check your hosts to look for failed services, full disks, pending restarts and clock drift."),
      );
    }
    return el(
      "div",
      { className: "home-empty prob-empty prob-clear" },
      icon("info", 22),
      el("p", {}, `No problems found on ${d.checked === 1 ? "the host" : `${d.checked} hosts`} checked${d.checkedAtMs ? ` (${agoWords(d.checkedAtMs, now)}${agoWords(d.checkedAtMs, now) === "just now" ? "" : " ago"})` : ""}.`),
    );
  }

  private row(f: Finding, now: number): HTMLElement {
    const sev = el("span", { className: `prob-sev sev-${f.severity}`, title: severityLabel(f.severity) }, icon(SEVERITY_ICON[f.severity], 16, severityLabel(f.severity)));
    const meta = [sourceLabel(f.source)];
    if (f.sinceMs) meta.push(agoWords(f.sinceMs, now) === "just now" ? "just now" : `for ${agoWords(f.sinceMs, now)}`);
    const tools = el("div", { className: "prob-tools" });
    if (f.action) {
      const a = f.action;
      const open = el("button", { type: "button", className: "prob-open" }, actionLabel(a));
      open.onclick = () => this.actions.open(a);
      tools.append(open);
    }
    const snooze = el("select", { className: "prob-snooze", title: "Hide this problem for a while" });
    snooze.setAttribute("aria-label", `Snooze ${f.title}`);
    snooze.append(el("option", { value: "" }, "Snooze…"), ...SNOOZES.map(([m, label]) => el("option", { value: String(m) }, label)));
    snooze.onchange = async () => {
      const minutes = Number(snooze.value);
      if (!minutes) return;
      // The list is drawn again below; a focused menu would hold it back.
      snooze.blur();
      try {
        await problemsSnooze(f.id, minutes);
      } catch (err) {
        this.problem = errorText(err);
      }
      await this.refresh();
    };
    tools.append(snooze);
    return el(
      "div",
      { className: `prob-row sev-${f.severity}` },
      sev,
      el("div", { className: "prob-text" }, el("div", { className: "prob-title" }, f.title), el("div", { className: "prob-detail muted" }, f.detail), el("div", { className: "prob-meta muted" }, meta.join(" · "))),
      tools,
    );
  }

  private snoozedList(list: SnoozedFinding[], now: number): HTMLElement {
    const box = el("details", { className: "host-card prob-snoozed", open: this.snoozedOpen }, el("summary", {}, `Snoozed (${list.length})`));
    box.ontoggle = () => (this.snoozedOpen = box.open);
    for (const f of list) {
      const wake = el("button", { type: "button", className: "link-button" }, "Show again");
      wake.onclick = async () => {
        try {
          await problemsUnsnooze(f.id);
        } catch (err) {
          this.problem = errorText(err);
        }
        await this.refresh();
      };
      box.append(
        el(
          "div",
          { className: "prob-row prob-row-snoozed" },
          el("span", { className: `prob-sev sev-${f.severity}` }, icon(SEVERITY_ICON[f.severity], 16, severityLabel(f.severity))),
          el("div", { className: "prob-text" }, el("div", { className: "prob-title" }, f.host ? `${f.host}: ${f.title}` : f.title), el("div", { className: "prob-meta muted" }, `Shows again ${untilWords(f.untilMs, now)}`)),
          el("div", { className: "prob-tools" }, wake),
        ),
      );
    }
    return box;
  }
}
