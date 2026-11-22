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
