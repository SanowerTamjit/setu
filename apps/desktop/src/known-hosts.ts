// The Known hosts tab of the SSH keys dialog: every server key you have
// trusted, searchable, with a way to forget one (after a server is rebuilt,
// for example). Shares ~/.ssh/known_hosts with OpenSSH.

import { el } from "./dom";
import { knownHostsList, knownHostsRemove, type KnownHost } from "./ipc";
import { registerKeysTab } from "./key-manager";

function label(e: KnownHost) {
  if (e.hashed) return "Hashed host name";
  return e.hosts.join(", ");
}

function matches(e: KnownHost, q: string) {
  if (!q) return true;
  const text = `${e.hosts.join(" ")} ${e.algorithm} ${e.fingerprint} ${e.comment}`.toLowerCase();
  return q
    .toLowerCase()
    .split(/\s+/)
    .every((word) => text.includes(word));
}

registerKeysTab({
  id: "known-hosts",
  title: "Known hosts",
  async render(body, flash) {
    body.replaceChildren(el("p", { className: "muted" }, "Reading known_hosts…"));
    const { path, entries } = await knownHostsList();
    const search = el("input", {
      type: "search",
      className: "known-search",
      placeholder: "Filter by host, key type or fingerprint",
      spellcheck: false,
    });
    const list = el("div", { className: "known-list" });
    const count = el("span", { className: "muted" });
    const rerender = () => void this.render(body, flash);

    const row = (e: KnownHost) => {
      const remove = el("button", { type: "button", className: "danger-button" }, "Forget");
      remove.title = "Remove this line; the next connection will ask you to check the key again";
      let armed = false;
      remove.onclick = async () => {
        if (!armed) {
          armed = true;
          remove.classList.add("armed");
          remove.textContent = "Forget it";
          setTimeout(() => {
            armed = false;
            remove.classList.remove("armed");
            remove.textContent = "Forget";
          }, 3000);
          return;
        }
        try {
          await knownHostsRemove(e.line, e.raw);
          flash(`Forgot ${label(e)}. The previous file is kept as known_hosts.old.`);
          rerender();
        } catch (err) {
          flash(String(err), true);
        }
      };
      return el(
        "div",
        { className: "known-row" },
        el(
          "div",
          { className: "known-main" },
          el(
            "span",
            { className: "known-hosts" },
            el("strong", { className: e.hashed ? "muted" : "" }, label(e)),
            ...(e.marker ? [el("span", { className: "badge changed" }, e.marker.slice(1))] : []),
          ),
          el("code", { className: "key-print" }, `${e.algorithm}  ${e.fingerprint || "unreadable key"}`),
        ),
        el("small", { className: "muted known-line" }, `line ${e.line}`),
        remove,
      );
    };

    const draw = () => {
      const shown = entries.filter((e) => matches(e, search.value.trim()));
      count.textContent = `${shown.length} of ${entries.length}`;
      list.replaceChildren(
        ...(shown.length
          ? shown.map(row)
          : [el("div", { className: "empty-box" }, entries.length ? "Nothing matches." : "No hosts trusted yet.")]),
      );
    };
    search.oninput = draw;
    draw();
    body.replaceChildren(
      el("div", { className: "keys-toolbar" }, search, count),
      list,
      el("p", { className: "muted agent-line" }, `${path}, shared with OpenSSH.`),
    );
    search.focus();
  },
});
