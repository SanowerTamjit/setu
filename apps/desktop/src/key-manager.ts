// The SSH keys dialog: the keys in ~/.ssh with their fingerprint, whether
// they have a passphrase and whether ssh-agent holds them; making a new key;
// copying a public key. A second tab manages known_hosts.

import { copyText, el, errorText, field, modal } from "./dom";
import { enabled } from "./features";
import {
  sshKeyAddToAgent,
  sshKeyGenerate,
  sshKeyLoadResident,
  sshKeysList,
  type KeyKind,
  type KeyList,
  type KeyView,
} from "./ipc";

/** Tabs other features add to this dialog, such as Known hosts. */
export interface KeysTab {
  id: string;
  title: string;
  render: (body: HTMLElement, flash: (text: string, error?: boolean) => void) => void | Promise<void>;
}

const extraTabs: KeysTab[] = [];

export function registerKeysTab(tab: KeysTab) {
  extraTabs.push(tab);
}

const KINDS: { id: KeyKind; name: string; note: string }[] = [
  { id: "ed25519", name: "Ed25519", note: "Recommended. Short, fast and accepted by any recent server." },
  { id: "ecdsa", name: "ECDSA P-256", note: "For servers that require a NIST curve." },
  { id: "rsa", name: "RSA 4096", note: "Only for old servers that accept nothing newer. Takes a few seconds." },
  {
    id: "ed25519-sk",
    name: "Security key (Ed25519)",
    note: "Lives on a YubiKey or other FIDO2 key: you touch it to sign in, and the file alone is useless. Servers need OpenSSH 8.2 or newer.",
  },
  {
    id: "ecdsa-sk",
    name: "Security key (ECDSA)",
    note: "For older security keys that cannot do Ed25519, such as some U2F-only keys.",
  },
];

const DEFAULT_NAMES: Record<KeyKind, string> = {
  ed25519: "id_ed25519",
  ecdsa: "id_ecdsa",
  rsa: "id_rsa",
  "ed25519-sk": "id_ed25519_sk",
  "ecdsa-sk": "id_ecdsa_sk",
};

const isSecurityKey = (kind: KeyKind) => kind.endsWith("-sk");

function shortType(algorithm: string | null) {
  if (!algorithm) return "unknown type";
  if (algorithm === "ssh-ed25519") return "Ed25519";
  if (algorithm.startsWith("ecdsa-sha2-")) return `ECDSA ${algorithm.slice(11).toUpperCase()}`;
  if (algorithm === "ssh-rsa") return "RSA";
  if (algorithm === "sk-ssh-ed25519@openssh.com") return "Ed25519-SK";
  if (algorithm === "sk-ecdsa-sha2-nistp256@openssh.com") return "ECDSA-SK";
  return algorithm;
}

function keyCard(key: KeyView, list: KeyList, flash: (t: string, e?: boolean) => void, refresh: () => void) {
  const badges = [
    el("span", { className: "badge" }, shortType(key.algorithm)),
    ...(key.securityKey ? [el("span", { className: "badge security-key" }, "Security key")] : []),
    // A security key file without a passphrase is normal: it is useless without the device.
    ...(key.state === "encrypted"
      ? [el("span", { className: "badge keyPassphrase" }, "Passphrase")]
      : key.state === "plain"
        ? key.securityKey
          ? []
          : [el("span", { className: "badge changed" }, "No passphrase")]
        : [el("span", { className: "badge danger" }, "Unreadable")]),
    ...(key.inAgent ? [el("span", { className: "badge new" }, "In agent")] : []),
  ];
  const actions: HTMLElement[] = [];
  if (key.publicLine) {
    const copy = el("button", { type: "button" }, "Copy public key");
    copy.onclick = async () => {
      await copyText(key.publicLine!);
      flash(`Copied the public key of ${key.name}. Paste it into the server's ~/.ssh/authorized_keys.`);
    };
    actions.push(copy);
  }
  const unlock = el("div", { className: "agent-unlock", hidden: true });
  if (list.agent === "running" && !key.inAgent && key.state !== "unreadable") {
    const add = el("button", { type: "button" }, "Add to agent");
    add.onclick = async () => {
      if (key.state === "encrypted") {
        const pass = el("input", { type: "password", autocomplete: "off", placeholder: "Passphrase" });
        const go = el("button", { type: "submit", className: "primary" }, "Add");
        const form = el("form", { className: "inline-form" }, pass, go);
        form.onsubmit = async (e) => {
          e.preventDefault();
          go.disabled = true;
          try {
            await sshKeyAddToAgent(key.path, pass.value);
            pass.value = "";
            flash(`${key.name} is in the agent now.`);
            refresh();
          } catch (err) {
            flash(errorText(err), true);
            go.disabled = false;
            pass.select();
          }
        };
        unlock.replaceChildren(form);
        unlock.hidden = false;
        pass.focus();
        return;
      }
      try {
        await sshKeyAddToAgent(key.path, null);
        flash(`${key.name} is in the agent now.`);
        refresh();
      } catch (err) {
        flash(errorText(err), true);
      }
    };
    actions.push(add);
  }
  return el(
    "div",
    { className: "key-card" },
    el(
      "div",
      { className: "key-head" },
      el("strong", {}, key.name),
      el("span", { className: "key-badges" }, ...badges),
    ),
    el(
      "code",
      { className: "key-print" },
      key.fingerprint ?? (key.problem ? key.problem : "No .pub file next to it, so no fingerprint"),
    ),
    ...(key.comment ? [el("small", { className: "muted" }, key.comment)] : []),
    ...(actions.length ? [el("div", { className: "key-actions" }, ...actions)] : []),
    unlock,
  );
}

function newKeyForm(onDone: (made: boolean) => void, flash: (t: string, e?: boolean) => void) {
  let kind: KeyKind = "ed25519";
  const name = el("input", { type: "text", value: DEFAULT_NAMES[kind], spellcheck: false });
  let nameTouched = false;
  name.oninput = () => (nameTouched = true);
  const comment = el("input", { type: "text", placeholder: "you@laptop", spellcheck: false });
  const pass = el("input", { type: "password", autocomplete: "new-password" });
  const again = el("input", { type: "password", autocomplete: "new-password" });
  const kinds = el(
    "div",
    { className: "choice-list" },
    ...KINDS.filter((k) => !isSecurityKey(k.id) || enabled("securityKeys")).map((k) => {
      const radio = el("input", { type: "radio", name: "key-kind", checked: k.id === kind });
      radio.onchange = () => {
        kind = k.id;
        if (!nameTouched) name.value = DEFAULT_NAMES[kind];
        showKind();
      };
      return el(
        "label",
        { className: "choice" },
        radio,
        el("span", {}, el("strong", {}, k.name), el("small", { className: "muted" }, k.note)),
      );
    }),
  );
  const resident = el("input", { type: "checkbox" });
  const verify = el("input", { type: "checkbox" });
  const pin = el("input", { type: "password", autocomplete: "off", placeholder: "Only if the key has one" });
  const skOptions = el(
    "div",
    { className: "group sk-options", hidden: true },
    el(
      "p",
      { className: "muted sk-hint" },
      "Plug in your security key. When you click Make key it blinks: touch it. The file saved here only points at the key on the device.",
    ),
    el(
      "label",
      { className: "check-row" },
      resident,
      el(
        "span",
        {},
        "Store it on the security key too",
        el("small", { className: "muted" }, "So you can load it onto another computer with Load from security key. Needs a FIDO2 key with a PIN."),
      ),
    ),
    el(
      "label",
      { className: "check-row" },
      verify,
      el(
        "span",
        {},
        "Ask for the PIN every time, not just a touch",
        el("small", { className: "muted" }, "Stronger, but the server must allow it (verify-required)."),
      ),
    ),
    field("Security key PIN", pin, "Used once to make the key. It is not saved."),
  );
  const showKind = () => {
    skOptions.hidden = !isSecurityKey(kind);
  };
  const make = el("button", { type: "submit", className: "primary" }, "Make key");
  const cancel = el("button", { type: "button" }, "Cancel");
  cancel.onclick = () => onDone(false);
  const form = el(
    "form",
    { className: "group" },
    kinds,
    skOptions,
    el("div", { className: "row even" }, field("File name", name, "Saved in your .ssh folder"), field("Comment", comment, "Shown next to the key on servers")),
    el(
      "div",
      { className: "row even" },
      field("Passphrase", pass, "Recommended. Leave empty for none."),
      field("Passphrase again", again),
    ),
    el("div", { className: "actions" }, el("span", { className: "spacer" }), cancel, make),
  );
  form.onsubmit = async (e) => {
    e.preventDefault();
    if (pass.value !== again.value) return flash("The two passphrases are different.", true);
    make.disabled = true;
    const sk = isSecurityKey(kind);
    make.textContent = kind === "rsa" ? "Making key (a few seconds)…" : sk ? "Touch your security key…" : "Making key…";
    if (sk) flash("Touch your security key now. It blinks while it waits.");
    try {
      const made = await sshKeyGenerate(
        name.value,
        kind,
        comment.value,
        pass.value || null,
        sk ? { resident: resident.checked, verifyRequired: verify.checked, pin: pin.value || null } : null,
      );
      pass.value = again.value = pin.value = "";
      flash(`Made ${made.path}. Its fingerprint is ${made.fingerprint}.`);
      onDone(true);
    } catch (err) {
      flash(errorText(err), true);
      make.disabled = false;
      make.textContent = "Make key";
    }
  };
  queueMicrotask(() => name.focus());
  return form;
}

export function openKeyManager(tabId = "keys"): Promise<void> {
  return new Promise((resolve) => {
    const tabsBar = el("div", { className: "dialog-tabs" });
    tabsBar.setAttribute("role", "tablist");
    const body = el("div", { className: "keys-body" });
    const message = el("div", { className: "dialog-message", hidden: true });
    const done = el("button", { type: "button", className: "primary" }, "Done");
    const dialog = el(
      "div",
      { className: "dialog keys-dialog" },
      el("h2", {}, "SSH keys and hosts"),
      tabsBar,
      message,
      body,
      el("div", { className: "actions" }, el("span", { className: "spacer" }), done),
    );
    dialog.setAttribute("role", "dialog");
    dialog.setAttribute("aria-label", "SSH keys and hosts");
    const { close } = modal(dialog, resolve);
    done.onclick = close;

    const flash = (text: string, error = false) => {
      message.textContent = text;
      message.className = `dialog-message ${error ? "error" : "info"}`;
      message.hidden = false;
    };

    const keysTab: KeysTab = {
      id: "keys",
      title: "Keys",
      async render(target) {
        target.replaceChildren(el("p", { className: "muted" }, "Reading your .ssh folder…"));
        const list = await sshKeysList();
        const refresh = () => void show(keysTab);
        const load = el("button", { type: "button" }, "Load from security key…");
        load.title = "Copy keys stored on your YubiKey or other FIDO2 key into your .ssh folder";
        load.onclick = () => {
          const pin = el("input", { type: "password", autocomplete: "off", placeholder: "Security key PIN" });
          const go = el("button", { type: "submit", className: "primary" }, "Load keys");
          const cancel = el("button", { type: "button" }, "Cancel");
          cancel.onclick = refresh;
          const form = el(
            "form",
            { className: "group" },
            el(
              "p",
              { className: "muted" },
              "Plug in the security key and enter its PIN. Keys that were made with \u201cStore it on the security key too\u201d are copied into your .ssh folder; existing files are kept.",
            ),
            field("PIN", pin),
            el("div", { className: "actions" }, el("span", { className: "spacer" }), cancel, go),
          );
          form.onsubmit = async (e) => {
            e.preventDefault();
            go.disabled = true;
            go.textContent = "Touch your security key…";
            try {
              const names = await sshKeyLoadResident(pin.value || null);
              pin.value = "";
              flash(names.length ? `Loaded ${names.join(", ")} from the security key.` : "The security key holds no keys that are not here already.");
              refresh();
            } catch (err) {
              flash(errorText(err), true);
              go.disabled = false;
              go.textContent = "Load keys";
            }
          };
          target.replaceChildren(el("h3", {}, "Load from security key"), form);
          pin.focus();
        };
        const add = el("button", { type: "button" }, "New key…");
        add.onclick = () => {
          target.replaceChildren(
            el("h3", {}, "New key"),
            newKeyForm(() => refresh(), flash),
          );
        };
        const agentLine =
          list.agent === "running"
            ? `ssh-agent is running and holds ${list.keys.filter((k) => k.inAgent).length + list.agentOnly.length} key(s).`
            : list.agent === "unreachable"
              ? "ssh-agent is set up but did not answer."
              : "No ssh-agent is running, so keys are unlocked each time you connect.";
        target.replaceChildren(
          el(
            "div",
            { className: "keys-toolbar" },
            el("span", { className: "muted" }, list.dir),
            el("span", { className: "spacer" }),
            ...(enabled("securityKeys") ? [load] : []),
            add,
          ),
          ...(list.keys.length
            ? list.keys.map((k) => keyCard(k, list, flash, refresh))
            : [el("div", { className: "empty-box" }, "No private keys here yet. Make one with New key.")]),
          el("p", { className: "muted agent-line" }, agentLine),
          ...(list.agentOnly.length
            ? [el("p", { className: "muted" }, `Also in the agent: ${list.agentOnly.join(", ")}`)]
            : []),
        );
      },
    };

    const tabs = [keysTab, ...extraTabs];
    let current = tabs.find((t) => t.id === tabId) ?? keysTab;
    const show = async (tab: KeysTab) => {
      current = tab;
      tabsBar.replaceChildren(
        ...tabs.map((t) => {
          const b = el("button", { type: "button", className: "dialog-tab" + (t === current ? " active" : "") }, t.title);
          b.setAttribute("role", "tab");
          b.setAttribute("aria-selected", String(t === current));
          b.onclick = () => {
            message.hidden = true;
            void show(t);
          };
          return b;
        }),
      );
      try {
        await tab.render(body, flash);
      } catch (err) {
        flash(errorText(err), true);
      }
    };
    void show(current);
    done.focus();
  });
}
