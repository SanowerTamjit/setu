// Import hosts from ~/.ssh/config: pick which ones, choose a folder, import.

import { el, errorText, field, modal } from "./dom";
import { sshConfigImport, sshConfigScan, type Imported, type ImportCandidate } from "./ipc";

function describe(c: ImportCandidate) {
  const h = c.host;
  const user = h.user ? `${h.user}@` : "";
  const port = h.port && h.port !== 22 ? `:${h.port}` : "";
  const via = h.jump ? ` via ${h.jump}` : "";
  return `${user}${h.host}${port}${via}`;
}

const STATUS_LABEL: Record<ImportCandidate["status"], string> = {
  new: "New",
  changed: "Update",
  unchanged: "Saved",
  invalid: "Skipped",
};

export function openImportDialog(): Promise<Imported | null> {
  return new Promise((resolve) => {
    let result: Imported | null = null;
    const body = el("div", { className: "group" }, el("p", { className: "muted" }, "Reading ~/.ssh/config…"));
    const message = el("div", { className: "dialog-message error", hidden: true });
    const importButton = el("button", { type: "submit", className: "primary", disabled: true }, "Import");
    const cancel = el("button", { type: "button" }, "Cancel");
    const form = el("form", { className: "dialog wide" },
      el("h2", {}, "Import from SSH config"),
      body,
      message,
      el("div", { className: "actions" }, cancel, importButton),
    );
    const { close } = modal(form, () => resolve(result));
    cancel.onclick = close;

    const fail = (err: unknown) => {
      message.hidden = false;
      message.textContent = errorText(err);
    };

    void (async () => {
      let scan;
      try {
        scan = await sshConfigScan();
      } catch (err) {
        body.replaceChildren();
        fail(err);
        return;
      }
      if (!scan.found || scan.candidates.length === 0) {
        body.replaceChildren(
          el("p", {}, scan.found ? `No hosts found in ${scan.path}.` : `There is no SSH config at ${scan.path}.`),
          el("p", { className: "muted" }, "Hosts you add in Setu are saved in Setu's own hosts file."),
        );
        cancel.textContent = "Close";
        importButton.hidden = true;
        return;
      }

      const boxes: { box: HTMLInputElement; candidate: ImportCandidate }[] = [];
      const rows = scan.candidates.map((candidate) => {
        const box = el("input", {
          type: "checkbox",
          checked: candidate.status === "new",
          disabled: candidate.status === "unchanged" || candidate.status === "invalid",
        });
        boxes.push({ box, candidate });
        box.onchange = sync;
        return el("label", { className: `import-row ${candidate.status}` },
          box,
          el("span", { className: "import-main" },
            el("span", { className: "import-name" }, candidate.host.name),
            el("span", { className: "muted" }, describe(candidate)),
            ...candidate.notes.map((n) => el("small", { className: "import-note" }, n)),
          ),
          el("span", { className: `badge ${candidate.status}` }, STATUS_LABEL[candidate.status]),
        );
      });

      const all = el("input", { type: "checkbox" });
      const folder = el("input", { value: "SSH config", spellcheck: false, autocomplete: "off" });
      function sync() {
        const selectable = boxes.filter((b) => !b.box.disabled);
        const picked = selectable.filter((b) => b.box.checked).length;
        all.checked = picked > 0 && picked === selectable.length;
        all.indeterminate = picked > 0 && picked < selectable.length;
        importButton.disabled = picked === 0;
        importButton.textContent = picked === 1 ? "Import 1 host" : `Import ${picked} hosts`;
      }
      all.onchange = () => {
        for (const b of boxes) if (!b.box.disabled) b.box.checked = all.checked;
        sync();
      };
      sync();

      body.replaceChildren(
        el("p", { className: "muted" }, `Found ${scan.candidates.length} hosts in ${scan.path}. Your SSH config is not changed.`),
        ...scan.warnings.map((w) => el("div", { className: "dialog-message info" }, w)),
        el("label", { className: "import-all" }, all, el("span", {}, "Select all")),
        el("div", { className: "import-list" }, ...rows),
        field("Put new hosts in folder", folder),
      );

      form.onsubmit = async (e) => {
        e.preventDefault();
        const hosts = boxes.filter((b) => b.box.checked && !b.box.disabled).map((b) => b.candidate.host);
        importButton.disabled = true;
        try {
          result = await sshConfigImport(hosts, folder.value.trim() || null);
          close();
        } catch (err) {
          fail(err);
          importButton.disabled = false;
        }
      };
    })();
  });
}
