// The vault dialog: create, unlock, see what is saved, change the master
// password, lock. Stored secrets are never shown or sent here.

import { el, errorText, field, modal } from "./dom";
import {
  vaultChangePassword,
  vaultCreate,
  vaultLock,
  vaultRemember,
  vaultRemove,
  vaultStatus,
  vaultUnlock,
  type VaultEntry,
  type VaultStatus,
} from "./ipc";

const MIN_CHARS = 8;

type Listener = (status: VaultStatus) => void;
const listeners: Listener[] = [];

/** Called with the new status whenever the dialog changes it. */
export function onVaultChange(listener: Listener) {
  listeners.push(listener);
}

function notify(status: VaultStatus) {
  for (const l of listeners) l(status);
}

const KIND_LABEL = { sshPassword: "Password", keyPassphrase: "Key passphrase" } as const;

function when(seconds: number) {
  const d = new Date(seconds * 1000);
  return d.toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
}

/** The "unlock automatically on this computer" tick box, or nothing when
 * this computer has no usable keychain. */
function rememberRow(status: VaultStatus, checked: boolean) {
  const box = el("input", { type: "checkbox", checked });
  if (!status.remember.available) return { row: null, box: null };
  const row = el("label", { className: "check-row remember-row" },
    box,
    el("span", {},
      "Unlock automatically on this computer",
      el("small", { className: "muted" },
        `Keeps the vault key in ${status.remember.store}. Anyone who can use your logged-in account can then open the vault.`,
      ),
    ),
  );
  return { row, box };
}

function passwordInput(placeholder = "") {
  return el("input", { type: "password", autocomplete: "off", placeholder });
}

/**
 * Opens the vault. With `unlockOnly`, it closes as soon as the vault is
 * unlocked (used when a connection needs a saved secret). Resolves with
 * whether the vault ended up unlocked.
 */
export function openVaultDialog(options: { unlockOnly?: boolean; reason?: string } = {}): Promise<boolean> {
  return new Promise((resolve) => {
    let status: VaultStatus | null = null;
    const body = el("div", { className: "group" });
    const message = el("div", { className: "dialog-message error", hidden: true });
    const form = el("form", { className: "dialog" }, el("h2", {}, "Vault"), body, message);
    const { close } = modal(form, () => resolve(status?.unlocked ?? false));

    const fail = (err: unknown) => {
      message.hidden = false;
      message.textContent = errorText(err);
    };
    const clearMessage = () => (message.hidden = true);

    const set = (next: VaultStatus) => {
      status = next;
      notify(next);
      if (options.unlockOnly && next.unlocked) {
        close();
        return;
      }
      render();
    };

    const busy = (button: HTMLButtonElement, label: string) => {
      const original = button.textContent;
      button.disabled = true;
      button.textContent = label;
      return () => {
        button.disabled = false;
        button.textContent = original;
      };
    };

    function renderCreate(s: VaultStatus) {
      const pw = passwordInput(`At least ${MIN_CHARS} characters`);
      const remember = rememberRow(s, false);
      const confirm = passwordInput();
      const create = el("button", { type: "submit", className: "primary" }, "Create vault");
      const cancel = el("button", { type: "button" }, "Not now");
      cancel.onclick = close;
      body.replaceChildren(
        el("p", {}, "Save SSH passwords and key passphrases, encrypted with one master password."),
        ...(options.reason ? [el("div", { className: "dialog-message info" }, options.reason)] : []),
        field("Master password", pw),
        field("Confirm master password", confirm),
        ...(remember.row ? [remember.row] : []),
        el("div", { className: "dialog-message warning" },
          "If you forget the master password, the saved secrets cannot be recovered. Nobody can reset it, including Setu.",
        ),
        el("div", { className: "actions" }, cancel, create),
      );
      pw.focus();
      form.onsubmit = async (e) => {
        e.preventDefault();
        clearMessage();
        if ([...pw.value].length < MIN_CHARS) return fail(`Use at least ${MIN_CHARS} characters.`);
        if (pw.value !== confirm.value) return fail("The two passwords are different.");
        const done = busy(create, "Creating…");
        try {
          set(await vaultCreate(pw.value, remember.box?.checked));
        } catch (err) {
          fail(err);
        } finally {
          done();
          pw.value = confirm.value = "";
        }
      };
    }

    function renderUnlock(s: VaultStatus) {
      const pw = passwordInput();
      const remember = rememberRow(s, s.remember.enabled);
      const unlock = el("button", { type: "submit", className: "primary" }, "Unlock");
      const cancel = el("button", { type: "button" }, "Cancel");
      cancel.onclick = close;
      body.replaceChildren(
        el("p", { className: "muted" }, options.reason ?? "Enter your master password to use saved passwords and passphrases."),
        field("Master password", pw),
        ...(remember.row ? [remember.row] : []),
        ...(s.remember.problem && s.remember.available ? [el("div", { className: "dialog-message info" }, s.remember.problem)] : []),
        el("div", { className: "actions" }, cancel, unlock),
      );
      pw.focus();
      form.onsubmit = async (e) => {
        e.preventDefault();
        clearMessage();
        const done = busy(unlock, "Unlocking…");
        try {
          set(await vaultUnlock(pw.value, remember.box ? remember.box.checked : undefined));
        } catch (err) {
          fail(err);
          pw.select();
        } finally {
          done();
          if (status?.unlocked) pw.value = "";
        }
      };
    }

    function entryRow(entry: VaultEntry) {
      const remove = el("button", { type: "button", className: "row-button", title: "Remove from vault", ariaLabel: `Remove ${entry.target}` }, "✕");
      let armed = false;
      remove.onclick = async () => {
        if (!armed) {
          armed = true;
          remove.textContent = "Remove?";
          remove.classList.add("armed");
          return;
        }
        try {
          set(await vaultRemove(entry.kind, entry.target));
        } catch (err) {
          fail(err);
        }
      };
      return el("div", { className: "vault-row" },
        el("span", { className: `badge ${entry.kind}` }, KIND_LABEL[entry.kind]),
        el("span", { className: "vault-target" }, entry.target),
        el("span", { className: "muted vault-date" }, when(entry.updated)),
        remove,
      );
    }

    function renderUnlocked(s: VaultStatus) {
      const lock = el("button", { type: "button" }, "Lock now");
      const change = el("button", { type: "button" }, "Change master password");
      const done = el("button", { type: "submit", className: "primary" }, "Done");
      lock.onclick = async () => set(await vaultLock());
      const remember = rememberRow(s, s.remember.enabled);
      if (remember.box) {
        const box = remember.box;
        box.onchange = async () => {
          clearMessage();
          box.disabled = true;
          try {
            set(await vaultRemember(box.checked));
          } catch (err) {
            fail(err);
            box.checked = !box.checked;
          } finally {
            box.disabled = false;
          }
        };
      }
      const keychainNote = !s.remember.available && s.remember.problem
        ? [el("small", { className: "muted" }, `Automatic unlock is not available: ${s.remember.problem}`)]
        : [];
      change.onclick = renderChange;
      form.onsubmit = (e) => {
        e.preventDefault();
        close();
      };
      body.replaceChildren(
        el("p", { className: "vault-state" }, el("span", { className: "dot unlocked" }), "Unlocked"),
        el("p", { className: "muted" },
          s.remember.enabled
            ? `Clears itself from memory after ${s.autoLockMinutes} minutes without use and reopens from ${s.remember.store} when needed. "Lock now" locks it until you enter the password. Saved secrets are filled in when you connect; they are never shown.`
            : `Locks by itself after ${s.autoLockMinutes} minutes without use. Saved secrets are filled in when you connect; they are never shown.`,
        ),
        ...(remember.row ? [remember.row] : []),
        ...keychainNote,
        s.entries.length
          ? el("div", { className: "vault-list" }, ...s.entries.map(entryRow))
          : el("div", { className: "empty-box muted" }, "Nothing saved yet. Tick “Save in vault” when you connect with a password or an encrypted key."),
        el("small", { className: "muted path" }, s.path),
        el("div", { className: "actions" }, change, el("span", { className: "spacer" }), lock, done),
      );
      done.focus();
    }

    function renderChange() {
      clearMessage();
      const current = passwordInput();
      const next = passwordInput(`At least ${MIN_CHARS} characters`);
      const confirm = passwordInput();
      const save = el("button", { type: "submit", className: "primary" }, "Change password");
      const back = el("button", { type: "button" }, "Back");
      back.onclick = render;
      body.replaceChildren(
        field("Current master password", current),
        field("New master password", next),
        field("Confirm new master password", confirm),
        el("div", { className: "actions" }, back, save),
      );
      current.focus();
      form.onsubmit = async (e) => {
        e.preventDefault();
        clearMessage();
        if ([...next.value].length < MIN_CHARS) return fail(`Use at least ${MIN_CHARS} characters.`);
        if (next.value !== confirm.value) return fail("The two new passwords are different.");
        const done = busy(save, "Re-encrypting…");
        try {
          set(await vaultChangePassword(current.value, next.value));
        } catch (err) {
          fail(err);
        } finally {
          done();
        }
      };
    }

    function render() {
      clearMessage();
      if (!status) return;
      if (!status.exists) renderCreate(status);
      else if (!status.unlocked) renderUnlock(status);
      else renderUnlocked(status);
    }

    void vaultStatus().then((s) => {
      status = s;
      if (options.unlockOnly && s.unlocked) close();
      else render();
    }, fail);
  });
}
