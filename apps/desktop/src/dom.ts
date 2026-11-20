// Small DOM helpers shared by the dialogs and panels.

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
    if (e.key === "Escape") {
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
  return { overlay, close };
}

export function errorText(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object" && "message" in err) return String((err as { message: unknown }).message);
  return String(err);
}
