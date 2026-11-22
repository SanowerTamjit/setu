// Import hosts: from ~/.ssh/config, or from another tool's file (PuTTY,
// MobaXterm, a CSV such as Termius exports, an Ansible inventory). Pick
// which ones, choose a folder, import.

import { el, errorText, field, modal } from "./dom";
import { hostsParseFile, sshConfigImport, sshConfigScan, type Imported, type ImportCandidate } from "./ipc";

function describe(c: ImportCandidate) {
  const h = c.host;
  const user = h.user ? `${h.user}@` : "";
  const port = h.port && h.port !== 22 ? `:${h.port}` : "";
  const via = h.jump ? ` via ${h.jump}` : "";
  const folder = h.folder ? ` · ${h.folder}` : "";
  return `${user}${h.host}${port}${via}${folder}`;
}

const STATUS_LABEL: Record<ImportCandidate["status"], string> = {
  new: "New",
  changed: "Update",
  unchanged: "Saved",
  invalid: "Skipped",
};

const SOURCES = [
  { id: "ssh", name: "SSH config", note: "Your ~/.ssh/config" },
  { id: "putty", name: "PuTTY", note: "A .reg export of its saved sessions" },
  { id: "moba", name: "MobaXterm", note: "A .mxtsessions export" },
  { id: "csv", name: "Termius or CSV", note: "Columns such as name, host, port, user, group" },
  { id: "ansible", name: "Ansible", note: "An inventory, INI or YAML" },
] as const;

const PUTTY_HELP =
  "In PowerShell or cmd, run: reg export HKCU\\Software\\SimonTatham\\PuTTY\\Sessions putty.reg";

export function openImportDialog(): Promise<Imported | null> {
  return new Promise((resolve) => {
    let result: Imported | null = null;
    const title = el("h2", {}, "Import hosts");
    const body = el("div", { className: "group" });
    const message = el("div", { className: "dialog-message error", hidden: true });
    const importButton = el("button", { type: "submit", className: "primary", hidden: true }, "Import");
    const back = el("button", { type: "button", hidden: true }, "Back");
    const cancel = el("button", { type: "button" }, "Cancel");
    const form = el(
      "form",
      { className: "dialog wide" },
      title,
      body,
      message,
      el("div", { className: "actions" }, back, el("span", { className: "spacer" }), cancel, importButton),
    );
    const { close } = modal(form, () => resolve(result));
    cancel.onclick = close;
    form.onsubmit = (e) => e.preventDefault();

    const fail = (err: unknown) => {
      message.hidden = false;
      message.textContent = errorText(err);
    };

    const fileInput = el("input", {
      type: "file",
      accept: ".reg,.mxtsessions,.csv,.ini,.yml,.yaml,.txt,*",
      hidden: true,
    });
    form.append(fileInput);

    const chooseSource = () => {
      title.textContent = "Import hosts";
      message.hidden = true;
      importButton.hidden = true;
      back.hidden = true;
      cancel.textContent = "Cancel";
      body.replaceChildren(
        el("p", { className: "muted" }, "Where are your hosts now? Only connection settings come across; saved passwords are never read."),
        el(
          "div",
          { className: "source-grid" },
          ...SOURCES.map((s) => {
            const b = el(
              "button",
              { type: "button", className: "source-card" },
              el("strong", {}, s.name),
              el("small", { className: "muted" }, s.note),
            );
            b.onclick = () => {
              if (s.id === "ssh") void fromSshConfig();
              else fileInput.click();
            };
            return b;
          }),
        ),
        el("p", { className: "muted small-print" }, `PuTTY keeps sessions in the Windows registry. ${PUTTY_HELP}`),
      );
    };

    fileInput.onchange = async () => {
      const file = fileInput.files?.[0];
      fileInput.value = "";
      if (!file) return;
      message.hidden = true;
      body.replaceChildren(el("p", { className: "muted" }, `Reading ${file.name}…`));
      try {
        const bytes = new Uint8Array(await file.arrayBuffer());
        const scan = await hostsParseFile(file.name, bytes);
        showCandidates({
          heading: `Import from ${scan.format}`,
          intro: `Found ${scan.candidates.length} hosts in ${file.name}.`,
          candidates: scan.candidates,
          notes: scan.skipped.length
            ? [`Left out ${scan.skipped.length}: ${scan.skipped.slice(0, 4).join("; ")}${scan.skipped.length > 4 ? "; …" : ""}`]
            : [],
          folder: "",
          folderHint: "Leave empty to keep the folders they had there.",
        });
      } catch (err) {
        chooseSource();
        fail(err);
      }
    };

    const fromSshConfig = async () => {
      body.replaceChildren(el("p", { className: "muted" }, "Reading ~/.ssh/config…"));
      try {
        const scan = await sshConfigScan();
        if (!scan.found || scan.candidates.length === 0) {
          chooseSource();
          fail(scan.found ? `No hosts found in ${scan.path}.` : `There is no SSH config at ${scan.path}.`);
          return;
        }
        showCandidates({
          heading: "Import from SSH config",
          intro: `Found ${scan.candidates.length} hosts in ${scan.path}. Your SSH config is not changed.`,
          candidates: scan.candidates,
          notes: scan.warnings,
          folder: "SSH config",
        });
      } catch (err) {
        chooseSource();
        fail(err);
      }
    };

    const showCandidates = (o: {
      heading: string;
      intro: string;
      candidates: ImportCandidate[];
      notes: string[];
      folder: string;
      folderHint?: string;
    }) => {
      title.textContent = o.heading;
      back.hidden = false;
      back.onclick = chooseSource;
      if (o.candidates.length === 0) {
        body.replaceChildren(el("p", {}, o.intro), ...o.notes.map((w) => el("div", { className: "dialog-message info" }, w)));
        importButton.hidden = true;
        return;
      }
      importButton.hidden = false;
      const boxes: { box: HTMLInputElement; candidate: ImportCandidate }[] = [];
      const rows = o.candidates.map((candidate) => {
        const box = el("input", {
          type: "checkbox",
          checked: candidate.status === "new",
          disabled: candidate.status === "unchanged" || candidate.status === "invalid",
        });
        boxes.push({ box, candidate });
        box.onchange = sync;
        return el(
          "label",
          { className: `import-row ${candidate.status}` },
          box,
          el(
            "span",
            { className: "import-main" },
            el("span", { className: "import-name" }, candidate.host.name),
            el("span", { className: "muted" }, describe(candidate)),
            ...candidate.notes.map((n) => el("small", { className: "import-note" }, n)),
          ),
          el("span", { className: `badge ${candidate.status}` }, STATUS_LABEL[candidate.status]),
        );
      });

      const all = el("input", { type: "checkbox" });
      const folder = el("input", { value: o.folder, spellcheck: false, autocomplete: "off" });
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
        el("p", { className: "muted" }, o.intro),
        ...o.notes.map((w) => el("div", { className: "dialog-message info" }, w)),
        el("label", { className: "import-all" }, all, el("span", {}, "Select all")),
        el("div", { className: "import-list" }, ...rows),
        field("Put new hosts in folder", folder, o.folderHint),
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
    };

    chooseSource();
  });
}
