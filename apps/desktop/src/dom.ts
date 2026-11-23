// Small DOM helpers shared by the dialogs and panels.

import { icon } from "./icons";

export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Record<string, unknown> = {},
  ...children: (Node | string)[]
): HTMLElementTagNameMap[K] {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

export function field(label: string, input: HTMLElement, hint?: string) {
  return el(
    "label",
    { className: "field" },
    el("span", {}, label),
    input,
    ...(hint ? [el("small", { className: "muted" }, hint)] : []),
  );
}

/**
 * Shows `content` in a modal overlay. Escape or `close()` removes it; the
 * returned promise-style callback runs once when it closes.
 */
export function modal(content: HTMLElement, onClose: () => void) {
  const overlay = el("div", { className: "overlay" }, content);
  document.body.append(overlay);
  let closed = false;
  const onKey = (e: KeyboardEvent) => {
    // Only the topmost dialog answers Escape.
    if (e.key === "Escape" && [...document.querySelectorAll(".overlay")].pop() === overlay) {
      e.preventDefault();
      e.stopPropagation();
      close();
    }
  };
  const close = () => {
    if (closed) return;
    closed = true;
    overlay.remove();
    document.removeEventListener("keydown", onKey, true);
    onClose();
  };
  document.addEventListener("keydown", onKey, true);
  // Every dialog gets the same close button in its top corner.
  if (content.classList.contains("dialog")) addCloseButton(content, close);
  return { overlay, close };
}

/** A small yes-or-no dialog; resolves true when the action is chosen. */
export function confirmDialog(
  title: string,
  text: string | HTMLElement,
  action: string,
  danger = false,
  /** Start on Cancel, so a reflex Enter does not confirm. */
  safeDefault = false,
): Promise<boolean> {
  return new Promise((resolve) => {
    let yes = false;
    const ok = el("button", { type: "submit", className: danger ? "danger-solid" : "primary" }, action);
    const cancel = el("button", { type: "button" }, "Cancel");
    const body = typeof text === "string" ? el("p", {}, text) : text;
    const form = el("form", { className: "dialog small" }, el("h2", {}, title), body, el("div", { className: "actions" }, cancel, ok));
    const { close } = modal(form, () => resolve(yes));
    cancel.onclick = close;
    (safeDefault ? cancel : ok).focus();
    form.onsubmit = (e) => {
      e.preventDefault();
      yes = true;
      close();
    };
  });
}

/** The close button in a dialog's top corner. */
export function addCloseButton(dialog: HTMLElement, close: () => void) {
  if (dialog.querySelector(":scope > .dialog-close")) return;
  const x = el("button", { type: "button", className: "dialog-close", title: "Close (Esc)" }, icon("close", 16));
  x.setAttribute("aria-label", "Close");
  x.onclick = close;
  dialog.prepend(x);
}

export function errorText(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object" && "message" in err) return String((err as { message: unknown }).message);
  return String(err);
}

/** Copies text to the clipboard, falling back to a hidden text box. */
export async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    return;
  } catch {
    // Some webviews refuse the async clipboard; use the older route.
  }
  const area = el("textarea", { value: text });
  area.style.position = "fixed";
  area.style.opacity = "0";
  document.body.append(area);
  area.select();
  document.execCommand("copy");
  area.remove();
}
