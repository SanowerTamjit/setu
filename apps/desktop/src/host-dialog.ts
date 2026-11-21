// Add or edit a saved host. Deleting is here too, behind a second click.

import { el, errorText, field, modal } from "./dom";
import { hostDelete, hostSave, type Host, type HostAuth, type HostsView } from "./ipc";

export function openHostDialog(
  existing: Host | null,
  folders: string[],
  defaultUser: string,
  suggestedFolder: string | null = null,
): Promise<HostsView | null> {
  return new Promise((resolve) => {
    let result: HostsView | null = null;
    const h: Host = existing ?? { name: "", host: "", folder: suggestedFolder };

    const name = el("input", { value: h.name, placeholder: "web-01", autocomplete: "off", spellcheck: false });
    const address = el("input", { value: h.host, placeholder: "10.0.4.21 or web-01.example.com", autocomplete: "off", spellcheck: false });
    const port = el("input", { value: String(h.port ?? 22), inputMode: "numeric", autocomplete: "off" });
    const user = el("input", { value: h.user ?? "", placeholder: defaultUser, autocomplete: "off", spellcheck: false });
    const folderList = el("datalist", { id: "host-folders" }, ...folders.map((f) => el("option", { value: f })));
    const folder = el("input", { value: h.folder ?? "", placeholder: "Production/Web", autocomplete: "off", spellcheck: false });
    folder.setAttribute("list", "host-folders");
    const tags = el("input", { value: (h.tags ?? []).join(", "), placeholder: "nginx, eu-west", autocomplete: "off", spellcheck: false });
    const auth = el("select", {},
      el("option", { value: "auto" }, "ssh-agent and keys in ~/.ssh"),
      el("option", { value: "agent" }, "ssh-agent only"),
      el("option", { value: "key" }, "Key file"),
      el("option", { value: "password" }, "Password (asked every time)"),
    );
    auth.value = h.auth ?? "auto";
    const keyFile = el("input", { value: h.key_file ?? "~/.ssh/id_ed25519", spellcheck: false });
    const keyField = field("Key file", keyFile, "Passphrases are asked for when you connect.");
    const syncAuth = () => (keyField.hidden = auth.value !== "key");
    auth.onchange = syncAuth;
    syncAuth();
    const jump = el("input", { value: h.jump ?? "", placeholder: "ops@bastion.example.com (optional)", spellcheck: false });
    const notes = el("textarea", { value: h.notes ?? "", rows: 2, placeholder: "Optional" });

    const message = el("div", { className: "dialog-message error", hidden: true });
    const save = el("button", { type: "submit", className: "primary" }, existing ? "Save" : "Add host");
    const cancel = el("button", { type: "button" }, "Cancel");
    const remove = el("button", { type: "button", className: "danger-button", hidden: !existing }, "Delete");
    const actions = el("div", { className: "actions" }, remove, el("span", { className: "spacer" }), cancel, save);

    const form = el("form", { className: "dialog" },
      el("h2", {}, existing ? `Edit ${existing.name}` : "Add host"),
      field("Name", name),
      el("div", { className: "row" }, field("Address", address), field("Port", port)),
      field("User", user, `Leave empty to use ${defaultUser}.`),
      el("div", { className: "row even" }, field("Folder", folder), field("Tags", tags)),
      folderList,
      field("Authentication", auth),
      keyField,
      field("Jump host", jump, "Like ssh -J; separate several hops with commas."),
      field("Notes", notes),
      message,
      actions,
    );
    const { close } = modal(form, () => resolve(result));
    cancel.onclick = close;
    (existing ? address : name).focus();

    const fail = (err: unknown) => {
      message.hidden = false;
      message.textContent = errorText(err);
    };

    let confirmDelete = false;
    remove.onclick = async () => {
      if (!existing) return;
      if (!confirmDelete) {
        confirmDelete = true;
        remove.textContent = `Delete ${existing.name}?`;
        remove.classList.add("armed");
        return;
      }
      try {
        result = await hostDelete(existing.name);
        close();
      } catch (err) {
        fail(err);
      }
    };

    form.onsubmit = async (e) => {
      e.preventDefault();
      if (!address.value.trim()) {
        fail("Enter the address to connect to.");
        address.focus();
        return;
      }
      const p = Number(port.value.trim() || "22");
      if (!Number.isInteger(p) || p < 1 || p > 65535) {
        fail("The port must be a number from 1 to 65535.");
        port.focus();
        return;
      }
      const kind = auth.value as HostAuth;
      const host: Host = {
        name: name.value.trim() || address.value.trim(),
        host: address.value.trim(),
        port: p,
        user: user.value.trim() || null,
        folder: folder.value.trim() || null,
        tags: tags.value.split(",").map((t) => t.trim()).filter(Boolean),
        auth: kind,
        key_file: kind === "key" ? keyFile.value.trim() || null : null,
        jump: jump.value.trim() || null,
        notes: notes.value.trim() || null,
      };
      save.disabled = true;
      try {
        result = await hostSave(host, existing?.name ?? null);
        close();
      } catch (err) {
        fail(err);
      } finally {
        save.disabled = false;
      }
    };
  });
}
