// The task board: ops work as cards in To do, Doing and Done. A card can
// list the hosts it is about and link the workflow that does the job, with
// that workflow's last run shown on the card. Drag cards between columns
// (or Alt+arrow keys); click one to edit it. Saved to board.toml as you go.

import { confirmDialog, el, errorText, field, modal } from "./dom";
import { icon } from "./icons";
import { boardLoad, boardSave, type Board, type BoardCard, type BoardColumn, type CardPriority, type Host, type RunStatus, type Workflow } from "./ipc";
import { expandTargets } from "./workflows-view";

export interface BoardActions {
  hosts: () => Host[];
  workflows: () => Workflow[];
  /** The status of a workflow's latest run, if it ran. */
  lastRun: (id: string) => RunStatus | undefined;
  openWorkflow: (id: string) => void;
  /** The Workflows | Board switch, drawn in the board's header. */
  tabs: () => HTMLElement;
}

const COLUMNS: { id: BoardColumn; title: string; empty: string }[] = [
  { id: "todo", title: "To do", empty: "Nothing planned. Add a card for the next job." },
  { id: "doing", title: "Doing", empty: "Drag a card here when you start on it." },
  { id: "done", title: "Done", empty: "Finished work lands here." },
];

const PRIORITY_LABEL: Record<CardPriority, string> = { low: "Low", normal: "Normal", high: "High" };

function today() {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

function newId() {
  return `c${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

/** Moves card `id` to `column` before the card at `index` among that column's cards. */
export function moveCard(board: Board, id: string, column: BoardColumn, index: number) {
  const from = board.card.findIndex((c) => c.id === id);
  if (from < 0) return;
  const [card] = board.card.splice(from, 1);
  card.column = column;
  const inColumn = board.card.filter((c) => c.column === column);
  const before = inColumn[index];
  const at = before ? board.card.indexOf(before) : board.card.length;
  board.card.splice(at, 0, card);
}

export class BoardView {
  private board: Board = { card: [] };
  private path = "";
  private error = "";
  private saved = true;
  private query = "";
  private saveTimer: number | undefined;
  private loaded = false;
  readonly element: HTMLElement;
  private readonly columnsEl: HTMLElement;
  private readonly header: HTMLElement;

  constructor(private readonly actions: BoardActions) {
    this.header = el("div", { className: "bd-header" });
    this.columnsEl = el("div", { className: "bd-columns" });
    this.element = el("section", { className: "bd-board", hidden: true }, this.header, this.columnsEl);
    this.element.setAttribute("aria-label", "Task board");
  }

  async refresh() {
    try {
      const { board, path } = await boardLoad();
      this.board = board;
      this.board.card ??= [];
      this.path = path;
      this.error = "";
      this.loaded = true;
    } catch (err) {
      this.error = `Could not open the board: ${errorText(err)}`;
      this.loaded = false;
    }
    this.render();
  }

  private change() {
    this.saved = false;
    this.render();
    window.clearTimeout(this.saveTimer);
    this.saveTimer = window.setTimeout(() => void this.save(), 300);
  }

  private async save() {
    // Never write over a file that failed to load: the person may fix it by hand.
    if (!this.loaded) return;
    try {
      await boardSave(this.board);
      this.saved = true;
      this.error = "";
    } catch (err) {
      this.error = `Could not save: ${errorText(err)}`;
    }
    this.renderHeader();
  }

  render() {
    this.renderHeader();
    this.renderColumns();
  }

  focus() {
    const first = this.columnsEl.querySelector<HTMLElement>(".bd-card") ?? this.header.querySelector<HTMLElement>(".bd-search");
    first?.focus({ preventScroll: true });
  }

  private renderHeader() {
    const hadFocus = document.activeElement instanceof HTMLElement && document.activeElement.classList.contains("bd-search");
    const search = el("input", { type: "search", className: "bd-search", placeholder: "Filter cards", value: this.query, spellcheck: false });
    search.setAttribute("aria-label", "Filter cards");
    search.oninput = () => {
      this.query = search.value.trim().toLowerCase();
      this.renderColumns();
    };
    const add = el("button", { type: "button", className: "primary with-icon", disabled: !this.loaded }, icon("plus", 15), "Add card");
    add.onclick = () => void this.edit(null, "todo");
    const open = this.board.card.filter((c) => c.column !== "done").length;
    const state = el("span", { className: "muted bd-state", title: this.path }, this.saved ? "Saved" : "Saving…");
    this.header.replaceChildren(
      this.actions.tabs(),
      el("div", { className: "bd-title" }, el("h2", {}, "Task board"), el("span", { className: "muted" }, `${open} open`)),
      el("div", { className: "bd-tools" }, search, state, add),
      ...(this.error ? [el("div", { className: "wf-banner error bd-error" }, this.error)] : []),
    );
    if (hadFocus) {
      search.focus();
      search.setSelectionRange(search.value.length, search.value.length);
    }
  }

  private visible(card: BoardCard) {
    if (!this.query) return true;
    const wf = card.workflow ? this.actions.workflows().find((w) => w.id === card.workflow)?.name ?? card.workflow : "";
    return [card.title, card.notes ?? "", ...(card.hosts ?? []), wf].join(" ").toLowerCase().includes(this.query);
  }

  private renderColumns() {
    this.columnsEl.replaceChildren(
      ...COLUMNS.map((col) => {
        const cards = this.board.card.filter((c) => c.column === col.id);
        const shown = cards.filter((c) => this.visible(c));
        const add = el("button", { type: "button", className: "bd-col-add", title: `Add a card to ${col.title}`, disabled: !this.loaded }, icon("plus", 14));
        add.setAttribute("aria-label", `Add a card to ${col.title}`);
        add.onclick = () => void this.edit(null, col.id);
        const list = el("div", { className: "bd-cards" }, ...shown.map((c) => this.card(c)));
        list.dataset.column = col.id;
        if (!cards.length) list.append(el("p", { className: "bd-empty muted" }, col.empty));
        else if (!shown.length) list.append(el("p", { className: "bd-empty muted" }, "No cards match the filter."));
        return el(
          "div",
          { className: `bd-column ${col.id}` },
          el("div", { className: "bd-col-head" }, el("span", { className: `bd-col-dot ${col.id}` }), el("h3", {}, col.title), el("span", { className: "bd-count" }, String(cards.length)), add),
          list,
        );
      }),
    );
  }

  private card(card: BoardCard) {
    const c = el("article", { className: `bd-card prio-${card.priority ?? "normal"}`, tabIndex: 0 });
    c.dataset.card = card.id;
    c.setAttribute("aria-label", `${card.title}, ${COLUMNS.find((x) => x.id === card.column)?.title}`);
    const head = el("div", { className: "bd-card-head" }, el("span", { className: "bd-card-title" }, card.title));
    if (card.priority === "high") head.prepend(el("span", { className: "bd-prio high", title: "High priority" }, "High"));
    c.append(head);
    if (card.notes) c.append(el("p", { className: "bd-notes" }, card.notes));
    const meta = el("div", { className: "bd-meta" });
    const hosts = this.actions.hosts();
    for (const h of card.hosts ?? []) {
      const unknown = expandTargets([h], hosts).unknown.length > 0;
      meta.append(el("span", { className: "bd-chip" + (unknown ? " unknown" : ""), title: unknown ? "Not a saved host or tag" : "" }, icon(h.startsWith("tag:") ? "snippet" : "hosts", 11), h));
    }
    if (card.workflow) {
      const wf = this.actions.workflows().find((w) => w.id === card.workflow);
      const status = this.actions.lastRun(card.workflow);
      const link = el(
        "button",
        { type: "button", className: "bd-chip bd-workflow" + (wf ? "" : " unknown"), title: wf ? `Open “${wf.name}”` : "This workflow no longer exists" },
        el("span", { className: `wf-dot ${status ?? "none"}` }),
        icon("workflow", 12),
        wf?.name ?? card.workflow,
      );
      link.onclick = (e) => {
        e.stopPropagation();
        if (wf) this.actions.openWorkflow(wf.id);
      };
      link.onpointerdown = (e) => e.stopPropagation();
      meta.append(link);
    }
    if (card.due) {
      const overdue = card.column !== "done" && card.due < today();
      meta.append(el("span", { className: "bd-chip bd-due" + (overdue ? " overdue" : ""), title: overdue ? "Overdue" : "Due" }, icon("calendar", 11), card.due));
    }
    if (meta.childNodes.length) c.append(meta);
    c.onclick = () => void this.edit(card);
    c.onkeydown = (e) => this.onCardKey(e, card);
    c.onpointerdown = (e) => this.startDrag(e, card, c);
    return c;
  }

  private onCardKey(e: KeyboardEvent, card: BoardCard) {
    if (e.key === "Enter") {
      e.preventDefault();
      void this.edit(card);
      return;
    }
    if (!e.altKey || !this.loaded) return;
    const order: BoardColumn[] = ["todo", "doing", "done"];
    const col = order.indexOf(card.column);
    const inCol = this.board.card.filter((c) => c.column === card.column);
    const pos = inCol.indexOf(card);
    let target: [BoardColumn, number] | null = null;
    if (e.key === "ArrowRight" && col < 2) target = [order[col + 1], 0];
    if (e.key === "ArrowLeft" && col > 0) target = [order[col - 1], 0];
    if (e.key === "ArrowUp" && pos > 0) target = [card.column, pos - 1];
    if (e.key === "ArrowDown" && pos < inCol.length - 1) target = [card.column, pos + 1];
    if (!target) return;
    e.preventDefault();
    moveCard(this.board, card.id, target[0], target[1]);
    this.change();
    this.columnsEl.querySelector<HTMLElement>(`[data-card="${CSS.escape(card.id)}"]`)?.focus();
  }

  /** Drags a card with the pointer, showing where it will land. */
  private startDrag(e: PointerEvent, card: BoardCard, node: HTMLElement) {
    if (e.button !== 0 || !this.loaded) return;
    const start = { x: e.clientX, y: e.clientY };
    let ghost: HTMLElement | null = null;
    let marker: HTMLElement | null = null;
    let drop: { column: BoardColumn; index: number } | null = null;
    const rect = node.getBoundingClientRect();
    node.setPointerCapture(e.pointerId);
    const where = (x: number, y: number) => {
      for (const list of this.columnsEl.querySelectorAll<HTMLElement>(".bd-cards")) {
        const r = list.parentElement!.getBoundingClientRect();
        if (x < r.left || x > r.right) continue;
        const cards = [...list.querySelectorAll<HTMLElement>(".bd-card")].filter((c) => c !== node);
        let index = cards.findIndex((c) => {
          const cr = c.getBoundingClientRect();
          return y < cr.top + cr.height / 2;
        });
        if (index < 0) index = cards.length;
        return { column: list.dataset.column as BoardColumn, index, list, before: cards[index] ?? null };
      }
      return null;
    };
    node.onpointermove = (m) => {
      if (!ghost) {
        if (Math.hypot(m.clientX - start.x, m.clientY - start.y) < 5) return;
        window.getSelection()?.removeAllRanges();
        ghost = node.cloneNode(true) as HTMLElement;
        ghost.classList.add("bd-ghost");
        ghost.style.width = `${rect.width}px`;
        document.body.append(ghost);
        node.classList.add("bd-dragging");
        marker = el("div", { className: "bd-marker" });
      }
      ghost.style.left = `${m.clientX - (start.x - rect.left)}px`;
      ghost.style.top = `${m.clientY - (start.y - rect.top)}px`;
      const w = where(m.clientX, m.clientY);
      if (w && marker) {
        w.list.querySelector(".bd-empty")?.remove();
        w.list.insertBefore(marker, w.before);
        // Index among the column's cards, not only the filtered ones.
        const all = this.board.card.filter((c) => c.column === w.column && c.id !== card.id);
        const beforeId = w.before?.dataset.card;
        drop = { column: w.column, index: beforeId ? all.findIndex((c) => c.id === beforeId) : all.length };
      }
    };
    node.onpointerup = () => {
      node.onpointermove = null;
      node.onpointerup = null;
      if (!ghost) return;
      // A drag is not a click.
      node.addEventListener("click", (c) => c.stopImmediatePropagation(), { capture: true, once: true });
      ghost.remove();
      marker?.remove();
      node.classList.remove("bd-dragging");
      if (drop) {
        moveCard(this.board, card.id, drop.column, drop.index);
        this.change();
      } else this.renderColumns();
    };
  }

  /** Opens the card editor; `card` null makes a new card in `column`. */
  private edit(card: BoardCard | null, column: BoardColumn = "todo"): Promise<void> {
    return new Promise((resolve) => {
      const draft: BoardCard = card ? JSON.parse(JSON.stringify(card)) : { id: newId(), title: "", column };
      const title = el("input", { value: draft.title, placeholder: "Renew the TLS certificate on web servers", spellcheck: false });
      const notes = el("textarea", { rows: 4, value: draft.notes ?? "", placeholder: "Links, steps, who asked" });
      const col = el("select", {}, ...COLUMNS.map((c) => el("option", { value: c.id, selected: c.id === draft.column }, c.title)));
      const prio = el("select", {}, ...(["low", "normal", "high"] as CardPriority[]).map((p) => el("option", { value: p, selected: p === (draft.priority ?? "normal") }, PRIORITY_LABEL[p])));
      const due = el("input", { type: "date", value: draft.due ?? "" });
      const wfSelect = el(
        "select",
        {},
        el("option", { value: "" }, "None"),
        ...this.actions.workflows().map((w) => el("option", { value: w.id, selected: w.id === draft.workflow }, w.name)),
      );
      const hosts = (draft.hosts ??= []);
      const chips = el("div", { className: "wf-chips" });
      const all = this.actions.hosts();
      const drawChips = () =>
        chips.replaceChildren(
          ...hosts.map((h, i) => {
            const x = el("button", { type: "button", className: "wf-chip-x", title: "Remove" }, icon("close", 11));
            x.setAttribute("aria-label", `Remove ${h}`);
            x.onclick = () => {
              hosts.splice(i, 1);
              drawChips();
            };
            return el("span", { className: "wf-chip" }, icon(h.startsWith("tag:") ? "snippet" : "hosts", 11), h, x);
          }),
        );
      drawChips();
      const tags = [...new Set(all.flatMap((h) => h.tags ?? []))].sort();
      const picker = el(
        "select",
        {},
        el("option", { value: "" }, "Add a host or tag…"),
        ...(tags.length ? [el("optgroup", { label: "Tags" }, ...tags.map((t) => el("option", { value: `tag:${t}` }, `tag:${t}`)))] : []),
        el("optgroup", { label: "Hosts" }, ...all.map((h) => el("option", { value: h.name }, h.name))),
      );
      picker.onchange = () => {
        if (picker.value && !hosts.includes(picker.value)) hosts.push(picker.value);
        picker.value = "";
        drawChips();
      };
      const message = el("div", { className: "dialog-message error", hidden: true });
      const save = el("button", { type: "submit", className: "primary" }, card ? "Save" : "Add card");
      const cancel = el("button", { type: "button" }, "Cancel");
      const del = el("button", { type: "button", className: "danger-button", hidden: !card }, "Delete");
      const form = el(
        "form",
        { className: "dialog bd-dialog" },
        el("h2", {}, card ? "Edit card" : "New card"),
        field("Title", title),
        field("Notes", notes),
        el("div", { className: "row even" }, field("Column", col), field("Priority", prio), field("Due", due)),
        el("label", { className: "field" }, el("span", {}, "Hosts"), chips, picker),
        field("Workflow", wfSelect, "The workflow that does this job; its last run shows on the card."),
        message,
        el("div", { className: "actions" }, del, el("span", { className: "spacer" }), cancel, save),
      );
      const { close } = modal(form, () => {
        // Back to the card (it may have moved), so the keyboard carries on from it.
        const back = this.columnsEl.querySelector<HTMLElement>(`[data-card="${CSS.escape(draft.id)}"]`);
        back?.focus({ preventScroll: true });
        resolve();
      });
      cancel.onclick = close;
      del.onclick = async () => {
        if (!card) return;
        if (!(await confirmDialog("Delete card?", `“${card.title}” is removed from the board.`, "Delete", true, true))) return;
        this.board.card = this.board.card.filter((c) => c.id !== card.id);
        close();
        this.change();
      };
      form.onsubmit = (e) => {
        e.preventDefault();
        const t = title.value.trim();
        if (!t) {
          message.hidden = false;
          message.textContent = "Give the card a title.";
          title.focus();
          return;
        }
        draft.title = t;
        draft.notes = notes.value.trim() || undefined;
        draft.priority = prio.value === "normal" ? undefined : (prio.value as CardPriority);
        draft.due = due.value || null;
        draft.workflow = wfSelect.value || null;
        draft.hosts = hosts.length ? hosts : undefined;
        const newColumn = col.value as BoardColumn;
        if (card) {
          const i = this.board.card.findIndex((c) => c.id === card.id);
          if (i >= 0) this.board.card[i] = { ...draft, column: card.column };
          if (newColumn !== card.column) moveCard(this.board, card.id, newColumn, 0);
        } else {
          draft.column = newColumn;
          this.board.card.push(draft);
          // New cards go to the top of their column.
          moveCard(this.board, draft.id, newColumn, 0);
        }
        close();
        this.change();
      };
      title.focus();
    });
  }
}
