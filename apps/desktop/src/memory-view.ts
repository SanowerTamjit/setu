// The Memory tab in Workflows: what Setu remembers of the commands you run,
// and what it suggests automating. Suggestions open as workflow drafts in
// the builder (nothing runs until you save and press Run) or save a
// snippet for the palette (it types the command; you press Enter).
//
// Everything here stays on this device. Remembering is off until you turn
// it on; Pause, Forget and the Features switch stop it.

import { confirmDialog, copyText, el, errorText, field, modal } from "./dom";
import { icon, type IconName } from "./icons";
import {
  memoryClear,
  memoryDelete,
  memoryDismiss,
  memorySettings,
  memorySnippetDelete,
  memorySnippetSave,
  memorySummary,
  memoryView,
  memoryWorkflowDraft,
  type MemorySuggestion,
  type MemoryView as MemoryData,
  type SuggestionKind,
  type Workflow,
} from "./ipc";
import { refreshMemoryState } from "./work-memory";

export interface MemoryActions {
  /** Opens a workflow draft in the builder. */
  openDraft: (w: Workflow) => void;
  /** The Workflows | Board | Memory switch. */
  tabs: () => HTMLElement;
}

const KIND: Record<SuggestionKind, { label: string; icon: IconName; action: string }> = {
  workflow: { label: "Workflow", icon: "workflow", action: "Open as workflow" },
  fleet: { label: "Fleet workflow", icon: "hosts", action: "Open as workflow" },
  snippet: { label: "Snippet", icon: "snippet", action: "Save as snippet" },
};

function when(at: number) {
  const d = new Date(at * 1000);
  const today = new Date();
  const sameDay = d.toDateString() === today.toDateString();
  const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return sameDay ? time : `${d.toLocaleDateString([], { month: "short", day: "numeric" })} ${time}`;
}

export class MemoryView {
  readonly element: HTMLElement;
  private data: MemoryData | null = null;
  private error = "";
  private notice = "";
  private query = "";
  private readonly header: HTMLElement;
  private readonly body: HTMLElement;

  constructor(private readonly actions: MemoryActions) {
    this.header = el("div", { className: "bd-header mem-header" });
    this.body = el("div", { className: "mem-body" });
    this.element = el("section", { className: "mem-view", hidden: true }, this.header, this.body);
    this.element.setAttribute("aria-label", "Work memory");
  }

  async refresh() {
    try {
      this.data = await memoryView();
      this.error = "";
    } catch (err) {
      this.error = `Could not open work memory: ${errorText(err)}`;
    }
    this.render();
  }

  focus() {
    (this.body.querySelector("button.primary, button") as HTMLElement | null)?.focus({ preventScroll: true });
  }

  private flash(text: string) {
    this.notice = text;
    this.render();
    window.setTimeout(() => {
      if (this.notice === text) {
        this.notice = "";
        this.render();
      }
    }, 4000);
  }

  private async act(run: () => Promise<unknown>, done?: string) {
    try {
      await run();
      await refreshMemoryState();
      await this.refresh();
      if (done) this.flash(done);
    } catch (err) {
      this.error = errorText(err);
      this.render();
    }
  }

  private setRecording(on: boolean) {
    const s = this.data?.settings;
    if (!s) return;
    void this.act(() => memorySettings({ ...s, recording: on }), on ? "Remembering is on." : "Paused. Nothing new is kept.");
  }

  render() {
    const d = this.data;
    const on = !!d?.settings.recording;
    const state = el(
      "span",
      { className: "mem-state " + (on ? "on" : "off"), title: d?.path ?? "" },
      el("span", { className: "mem-state-dot" }),
      on ? "Remembering" : d && (d.total || d.snippets.length) ? "Paused" : "Off",
    );
    const tools = el("div", { className: "bd-tools" });
    if (d && (on || d.total || d.snippets.length)) {
      const toggle = el("button", { type: "button", className: "with-icon" }, icon(on ? "pause" : "play", 14), on ? "Pause" : "Resume");
      toggle.onclick = () => this.setRecording(!on);
      const copy = el("button", { type: "button", className: "with-icon", title: "A Markdown summary of your routine work, secrets masked. Paste it to a teammate or an AI assistant." }, icon("files", 14), "Copy summary");
      copy.onclick = () =>
        void memorySummary()
          .then(copyText)
          .then(() => this.flash("Summary copied. It stays on your device until you paste it somewhere."))
          .catch((err) => this.flash(errorText(err)));
      const prefs = el("button", { type: "button", className: "with-icon" }, icon("settings", 14), "Settings");
      prefs.onclick = () => this.openSettings();
      tools.append(toggle, copy, prefs);
    }
    this.header.replaceChildren(
      this.actions.tabs(),
      el(
        "div",
        { className: "bd-title" },
        el("h2", {}, "Work memory"),
        el("span", { className: "muted" }, d ? `${d.total} ${d.total === 1 ? "command" : "commands"} kept` : ""),
      ),
      state,
      tools,
    );
    const parts: HTMLElement[] = [];
    if (this.error) parts.push(el("div", { className: "wf-banner error" }, this.error));
    if (this.notice) parts.push(el("div", { className: "wf-banner" }, this.notice));
    if (!d) {
      this.body.replaceChildren(...parts);
      return;
    }
    if (!on && !d.total && !d.snippets.length) {
      parts.push(this.intro());
    } else {
      parts.push(this.suggestionsSection(d), this.snippetsSection(d), this.historySection(d));
    }
    this.body.replaceChildren(...parts);
  }

  private intro() {
    const start = el("button", { type: "button", className: "primary with-icon" }, icon("spark", 15), "Start remembering");
    start.onclick = () => this.setRecording(true);
    const point = (ic: IconName, title: string, text: string) =>
      el("li", {}, icon(ic, 16), el("span", {}, el("strong", {}, title), el("small", { className: "muted" }, text)));
    return el(
      "div",
      { className: "mem-intro" },
      el("div", { className: "mem-intro-icon" }, icon("spark", 28)),
      el("h3", {}, "Stop doing the same work twice"),
      el(
        "p",
        { className: "muted" },
        "Setu can remember the commands you run in its terminals and notice what you repeat: the same commands in the same order, one command on many servers, a long line you keep typing. It then offers to turn them into a workflow or a snippet in one click.",
      ),
      el(
        "ul",
        { className: "mem-points" },
        point("lock", "Only on this device", "Kept in one private file in Setu's settings folder. Nothing is sent anywhere."),
        point("vault", "Secrets kept out", "Nothing typed at a password prompt is kept, and passwords, tokens and keys in commands are masked."),
        point("pause", "You stay in charge", "Pause any time, delete single commands or forget everything. Lines starting with a space are skipped."),
        point("workflow", "Nothing runs by itself", "A suggestion opens as a draft for you to check; snippets type, you press Enter."),
      ),
      el("div", { className: "actions" }, start),
    );
  }

  private section(title: string, count: string, ...children: HTMLElement[]) {
    return el("section", { className: "mem-section" }, el("div", { className: "mem-section-head" }, el("h3", {}, title), el("span", { className: "bd-count" }, count)), ...children);
  }

  private suggestionsSection(d: MemoryData) {
    if (!d.suggestions.length) {
      return this.section(
        "Suggestions",
        "0",
        el(
          "p",
          { className: "muted mem-empty" },
          d.settings.recording
            ? "Nothing repeats often enough yet. Keep working; suggestions show up here when the same work comes back three or four times."
            : "Remembering is paused. Resume it to get new suggestions.",
        ),
      );
    }
    return this.section("Suggestions", String(d.suggestions.length), el("div", { className: "mem-cards" }, ...d.suggestions.map((s) => this.suggestionCard(s))));
  }

  private suggestionCard(s: MemorySuggestion) {
    const k = KIND[s.kind];
    const go = el("button", { type: "button", className: "primary with-icon" }, icon(s.kind === "snippet" ? "plus" : "workflow", 14), k.action);
    go.onclick = () => void this.accept(s);
    const no = el("button", { type: "button" }, "Not useful");
    no.onclick = () => void this.act(() => memoryDismiss(s.id), "Hidden. Setu will not suggest it again.");
    const chips = s.tag
      ? [el("span", { className: "bd-chip" }, icon("snippet", 11), `tag:${s.tag}`)]
      : s.kind !== "snippet"
        ? s.hosts.slice(0, 6).map((h) => el("span", { className: "bd-chip" }, icon("hosts", 11), h))
        : [];
    return el(
      "article",
      { className: `mem-card kind-${s.kind}` },
      el("div", { className: "mem-card-head" }, el("span", { className: `mem-kind kind-${s.kind}` }, icon(k.icon, 12), k.label), el("span", { className: "muted mem-when" }, `last ${when(s.last)}`)),
      el("strong", { className: "mem-card-title" }, s.title),
      el("p", { className: "muted mem-reason" }, s.reason),
      el("ol", { className: "mem-commands" }, ...s.commands.map((c) => el("li", {}, el("code", {}, c)))),
      ...(chips.length ? [el("div", { className: "bd-meta" }, ...chips)] : []),
      el("div", { className: "mem-card-actions" }, no, go),
    );
  }

  private async accept(s: MemorySuggestion) {
    if (s.kind === "snippet") {
      const title = await this.askName(s.commands[0]);
      if (title === null) return;
      await this.act(() => memorySnippetSave(title, s.commands[0]), `Saved “${title}”. Find it in the command palette under Snippet.`);
      return;
    }
    try {
      const draft = await memoryWorkflowDraft(s.id);
      this.actions.openDraft(draft);
      // It is in the builder now; do not offer it again.
      await memoryDismiss(s.id);
    } catch (err) {
      this.error = errorText(err);
      this.render();
    }
  }

  /** Asks for a snippet's name, with a guess from the command. */
  private askName(command: string): Promise<string | null> {
    return new Promise((resolve) => {
      let result: string | null = null;
      const guess = command.split(/\s*(?:&&|\|)\s*/)[0].split(" ").slice(0, 3).join(" ");
      const name = el("input", { value: guess, maxLength: 100, required: true });
      const save = el("button", { type: "submit", className: "primary" }, "Save snippet");
      const cancel = el("button", { type: "button" }, "Cancel");
      const form = el(
        "form",
        { className: "dialog small" },
        el("h2", {}, "Save as snippet"),
        el("pre", { className: "mem-preview" }, command),
        field("Name", name, "Shown in the command palette. Picking it types the command; you press Enter."),
        el("div", { className: "actions" }, cancel, save),
      );
      const { close } = modal(form, () => resolve(result));
      cancel.onclick = close;
      form.onsubmit = (e) => {
        e.preventDefault();
        result = name.value.trim();
        if (result) close();
      };
      name.focus();
      name.select();
    });
  }

  private snippetsSection(d: MemoryData) {
    if (!d.snippets.length) return el("span", { hidden: true });
    return this.section(
      "My snippets",
      String(d.snippets.length),
      el(
        "ul",
        { className: "mem-snippets" },
        ...d.snippets.map((s) => {
          const del = el("button", { type: "button", className: "icon-button", title: `Delete “${s.title}”` }, icon("trash", 14));
          del.setAttribute("aria-label", `Delete ${s.title}`);
          del.onclick = () => void this.act(() => memorySnippetDelete(s.id));
          return el("li", {}, icon("snippet", 14), el("strong", {}, s.title), el("code", {}, s.command), del);
        }),
      ),
    );
  }

  private historySection(d: MemoryData) {
    const search = el("input", { type: "search", placeholder: "Filter commands", value: this.query, className: "mem-search" });
    search.oninput = () => {
      this.query = search.value.toLowerCase();
      list.replaceChildren(...rows());
    };
    const rows = () => {
      const shown = d.entries.filter((e) => !this.query || `${e.command} ${e.host ?? ""}`.toLowerCase().includes(this.query));
      if (!shown.length) return [el("li", { className: "muted mem-empty" }, d.entries.length ? "No commands match." : "No commands kept yet.")];
      return shown.slice(0, 200).map((e) => {
        const del = el("button", { type: "button", className: "icon-button", title: "Forget this command" }, icon("close", 13));
        del.setAttribute("aria-label", "Forget this command");
        del.onclick = () => void this.act(() => memoryDelete(e.id));
        return el(
          "li",
          { className: e.masked ? "masked" : "" },
          el("span", { className: "mem-time muted" }, when(e.at)),
          el("span", { className: "mem-where" + (e.host ? "" : " local") }, icon(e.host ? "hosts" : "terminal", 11), e.host ?? "local"),
          el("code", {}, e.command),
          ...(e.masked ? [el("span", { className: "mem-masked", title: "A secret was masked; this is never suggested" }, icon("lock", 11), "masked")] : []),
          del,
        );
      });
    };
    const list = el("ul", { className: "mem-history" }, ...rows());
    const forget = el("button", { type: "button", className: "danger with-icon" }, icon("trash", 14), "Forget all commands");
    forget.disabled = !d.total;
    forget.onclick = async () => {
      if (await confirmDialog("Forget all commands?", "Every remembered command and dismissed suggestion is deleted. Your snippets and settings stay.", "Forget", true, true)) {
        await this.act(() => memoryClear(false), "Forgotten.");
      }
    };
    return this.section("Recent commands", String(d.total), el("div", { className: "mem-history-tools" }, search, forget), list);
  }

  private openSettings() {
    const d = this.data;
    if (!d) return;
    const days = el("input", { type: "number", min: "1", max: "365", value: String(d.settings.retention_days) });
    const ignore = el("textarea", { rows: 5, value: d.settings.ignore.join("\n"), placeholder: "vim\nmysql\nkubectl edit" });
    const err = el("p", { className: "error", hidden: true });
    const save = el("button", { type: "submit", className: "primary" }, "Save");
    const cancel = el("button", { type: "button" }, "Cancel");
    const wipe = el("button", { type: "button", className: "danger" }, "Delete everything");
    const form = el(
      "form",
      { className: "dialog" },
      el("h2", {}, "Work memory settings"),
      field("Keep commands for (days)", days, "Older commands are deleted. 1 to 365."),
      field("Never remember commands starting with", ignore, "One per line. Matches whole words: “vim” skips “vim notes.txt” but not “vimdiff”."),
      el("p", { className: "muted" }, "Stored in ", el("code", {}, d.path), ", readable only by you."),
      err,
      el("div", { className: "actions" }, wipe, el("span", { className: "spacer" }), cancel, save),
    );
    const { close } = modal(form, () => undefined);
    cancel.onclick = close;
    wipe.onclick = async () => {
      if (await confirmDialog("Delete all work memory?", "Commands, snippets, dismissed suggestions and these settings are deleted, and remembering turns off.", "Delete everything", true, true)) {
        close();
        await this.act(() => memoryClear(true), "Work memory deleted.");
      }
    };
    form.onsubmit = async (e) => {
      e.preventDefault();
      try {
        await memorySettings({
          recording: d.settings.recording,
          retention_days: Number(days.value),
          ignore: ignore.value.split("\n").map((l) => l.trim()).filter(Boolean),
        });
        close();
        await this.act(async () => undefined, "Settings saved.");
      } catch (e2) {
        err.textContent = errorText(e2);
        err.hidden = false;
      }
    };
    days.focus();
  }
}
