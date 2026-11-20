// The "New SSH connection" dialog, including the host-key trust prompt.

import { el, field } from "./dom";
import { sshTrustHost, vaultStatus, type AuthRequest, type ConnectFailure, type ConnectRequest, type VaultAvailability } from "./ipc";
import { openVaultDialog } from "./vault-dialog";

type Connect = (request: ConnectRequest) => Promise<void>;

export type AuthKind = "default" | "agent" | "keyFile" | "password";

export interface DialogValues {
  host: string;
  port: string;
  user: string;
  authKind: AuthKind;
  keyPath: string;
  jump: string;
}

export interface DialogOptions {
  /** Values to show instead of the last attempt's (for a saved host). */
  prefill?: DialogValues;
  title?: string;
  /**
   * Try to connect right away and only show the dialog when something needs
   * the user: a new host key, a passphrase, a password, or an error.
   */
  autoConnect?: boolean;
}

/** Values from the last attempt, so a retry does not mean retyping. */
let last: DialogValues = { host: "", port: "22", user: "", authKind: "default", keyPath: "~/.ssh/id_ed25519", jump: "" };

/** The default user for new connections, set at startup. */
export function setDefaultUser(user: string) {
  if (!last.user) last.user = user;
}

function isFailure(err: unknown): err is ConnectFailure {
  return typeof err === "object" && err !== null && "kind" in err;
}

export function openSshDialog(connect: Connect, options: DialogOptions = {}): Promise<void> {
  const start = options.prefill ?? last;
  return new Promise((resolve) => {
    const host = el("input", { value: start.host, placeholder: "server.example.com", required: true, autocomplete: "off", spellcheck: false });
    const port = el("input", { value: start.port, inputMode: "numeric", autocomplete: "off" });
    const user = el("input", { value: start.user, placeholder: "root", required: true, autocomplete: "off", spellcheck: false });
    const authKind = el("select", {},
      el("option", { value: "default" }, "ssh-agent and keys in ~/.ssh"),
      el("option", { value: "agent" }, "ssh-agent only"),
      el("option", { value: "keyFile" }, "Key file"),
      el("option", { value: "password" }, "Password"),
    );
    authKind.value = start.authKind;
    const keyPath = el("input", { value: start.keyPath, spellcheck: false });
    const passphrase = el("input", { type: "password", placeholder: "Only if the key is encrypted", autocomplete: "off" });
    const password = el("input", { type: "password", autocomplete: "off" });
    const savePassphrase = el("input", { type: "checkbox" });
    const savePassword = el("input", { type: "checkbox" });
    let vault: VaultAvailability = "none";
    // "Save in vault" needs an unlocked vault; offer to unlock or create one.
    const saveRow = (box: HTMLInputElement) => {
      const hint = el("button", { type: "button", className: "link-button" });
      hint.onclick = async () => {
        if (await openVaultDialog({ unlockOnly: true })) {
          vault = "unlocked";
          box.checked = true;
        }
        syncVault();
      };
      const row = el("label", { className: "check-row" }, box, el("span", {}, "Save in vault"), hint);
      return { row, hint };
    };
    const passphraseSave = saveRow(savePassphrase);
    const passwordSave = saveRow(savePassword);
    const syncVault = () => {
      for (const [box, s] of [[savePassphrase, passphraseSave], [savePassword, passwordSave]] as const) {
        box.disabled = vault !== "unlocked";
        if (box.disabled) box.checked = false;
        s.hint.hidden = vault === "unlocked";
        s.hint.textContent = vault === "locked" ? "Unlock vault" : "Set up vault";
      }
      const empty = vault === "unlocked" ? "Leave empty to use the saved one" : "";
      password.placeholder = empty;
      passphrase.placeholder = vault === "unlocked" ? "Leave empty to use the saved one, if any" : "Only if the key is encrypted";
    };
    void vaultStatus().then((s) => {
      vault = s.unlocked ? "unlocked" : s.exists ? "locked" : "none";
      syncVault();
    });
    const jump = el("input", { value: start.jump, placeholder: "ops@bastion.example.com:22 (optional)", spellcheck: false });

    const keyFields = el("div", { className: "group" }, field("Key file", keyPath), field("Passphrase", passphrase), passphraseSave.row);
    const passwordFields = el("div", { className: "group" }, field("Password", password), passwordSave.row);
    const syncAuth = () => {
      keyFields.hidden = authKind.value !== "keyFile";
      passwordFields.hidden = authKind.value !== "password";
    };
    authKind.onchange = syncAuth;
    syncAuth();

    const message = el("div", { className: "dialog-message", hidden: true });
    const trustPanel = el("div", { className: "trust", hidden: true });
    const connectButton = el("button", { type: "submit", className: "primary" }, "Connect");
    const cancelButton = el("button", { type: "button" }, "Cancel");

    const mainActions = el("div", { className: "actions" }, cancelButton, connectButton);
    const form = el("form", { className: "dialog", noValidate: false },
      el("h2", {}, options.title ?? "New SSH connection"),
      el("div", { className: "row" }, field("Host", host), field("Port", port)),
      field("User", user),
      field("Authentication", authKind),
      keyFields,
      passwordFields,
      field("Jump host", jump, "Connect through a bastion, like ssh -J."),
      message,
      trustPanel,
      mainActions,
    );
    const overlay = el("div", { className: "overlay" }, form);
    // Auto-connect keeps the dialog out of sight unless the user is needed.
    const auto = options.autoConnect === true;
    overlay.hidden = auto;
    document.body.append(overlay);
    const focusFirst = () => {
      if (start.authKind === "password" && start.host) password.focus();
      else if (start.host && start.user) connectButton.focus();
      else (start.host ? user : host).focus();
    };
    focusFirst();

    const close = () => {
      overlay.remove();
      document.removeEventListener("keydown", onKey, true);
      resolve();
    };
    const onKey = (e: KeyboardEvent) => {
      // Only the topmost dialog answers Escape.
      const top = [...document.querySelectorAll(".overlay")].pop();
      if (e.key === "Escape" && !overlay.hidden && top === overlay) {
        e.preventDefault();
        e.stopPropagation();
        close();
      }
    };
    document.addEventListener("keydown", onKey, true);
    cancelButton.onclick = close;

    const showMessage = (text: string, kind: "error" | "info" = "error", action?: { label: string; run: () => void }) => {
      message.hidden = false;
      message.className = `dialog-message ${kind}`;
      message.textContent = text;
      if (action) {
        const button = el("button", { type: "button", className: "link-button" }, action.label);
        button.onclick = action.run;
        message.append(" ", button);
      }
    };
    /** Offers to unlock the vault, then retries with the saved secret. */
    const unlockAndRetry = (what: string) => ({
      label: "Unlock vault",
      run: async () => {
        if (await openVaultDialog({ unlockOnly: true, reason: `Unlock the vault to use the saved ${what}.` })) {
          vault = "unlocked";
          syncVault();
          void attempt();
        }
      },
    });

    const request = (): ConnectRequest => {
      let auth: AuthRequest = { kind: "default" };
      if (authKind.value === "agent") auth = { kind: "agent" };
      if (authKind.value === "keyFile")
        auth = { kind: "keyFile", path: keyPath.value.trim(), passphrase: passphrase.value || null, save: savePassphrase.checked };
      if (authKind.value === "password") auth = { kind: "password", password: password.value, save: savePassword.checked };
      return {
        host: host.value.trim(),
        port: Number(port.value.trim() || "22"),
        user: user.value.trim(),
        auth,
        jump: jump.value.trim() || null,
      };
    };

    const attempt = async () => {
      last = { host: host.value.trim(), port: port.value, user: user.value.trim(), authKind: authKind.value as AuthKind, keyPath: keyPath.value, jump: jump.value.trim() };
      message.hidden = true;
      trustPanel.hidden = true;
      trustPanel.classList.remove("danger-panel");
      mainActions.hidden = false;
      connectButton.disabled = true;
      connectButton.textContent = "Connecting…";
      try {
        await connect(request());
        close();
      } catch (err) {
        handleFailure(isFailure(err) ? err : { kind: "error", message: String(err) });
      } finally {
        connectButton.disabled = false;
        connectButton.textContent = "Connect";
      }
    };

    const handleFailure = (failure: ConnectFailure) => {
      if (overlay.hidden) {
        overlay.hidden = false;
        focusFirst();
      }
      switch (failure.kind) {
        case "unknownHostKey": {
          const { key } = failure;
          const label = key.port === 22 ? key.host : `[${key.host}]:${key.port}`;
          const trust = el("button", { type: "button", className: "primary" }, "Trust and connect");
          const notNow = el("button", { type: "button" }, "Don't trust");
          trustPanel.replaceChildren(
            el("h3", {}, "First connection to this host"),
            el("p", {}, `Setu has not seen ${label} before. Check that this fingerprint matches the server's before you trust it:`),
            el("code", { className: "fingerprint" }, `${key.algorithm}  ${key.fingerprint}`),
            el("p", { className: "muted" }, "On the server: ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub. Trusted keys are saved to ~/.ssh/known_hosts, shared with OpenSSH."),
            el("div", { className: "actions" }, notNow, trust),
          );
          trustPanel.hidden = false;
          mainActions.hidden = true;
          trust.focus();
          notNow.onclick = () => {
            trustPanel.hidden = true;
            mainActions.hidden = false;
            showMessage("Not connected: the host key was not trusted.", "info");
          };
          trust.onclick = async () => {
            try {
              await sshTrustHost(key);
              await attempt();
            } catch (err) {
              showMessage(String(err));
            }
          };
          break;
        }
        case "hostKeyChanged": {
          const { key } = failure;
          trustPanel.replaceChildren(
            el("h3", { className: "danger" }, "Warning: the host key has changed"),
            el("p", {}, `The key for ${key.host}:${key.port} is different from the one saved in ${failure.knownHosts} (line ${failure.line}). Someone could be intercepting the connection, or the server was reinstalled.`),
            el("code", { className: "fingerprint" }, `now: ${key.algorithm}  ${key.fingerprint}`),
            el("p", { className: "muted" }, "Setu will not connect. If the change is expected, remove that line from known_hosts and try again."),
          );
          trustPanel.hidden = false;
          trustPanel.classList.add("danger-panel");
          break;
        }
        case "needsPassphrase":
          authKind.value = "keyFile";
          keyPath.value = failure.path;
          syncAuth();
          vault = failure.vault;
          syncVault();
          if (failure.vault === "locked") {
            showMessage(`The key ${failure.path} is encrypted. Enter its passphrase, or unlock the vault if you saved it there.`, "info", unlockAndRetry("passphrase"));
          } else {
            showMessage(`The key ${failure.path} is encrypted. Enter its passphrase.`, "info");
          }
          passphrase.focus();
          break;
        case "needsPassword":
          authKind.value = "password";
          syncAuth();
          vault = failure.vault;
          syncVault();
          if (failure.vault === "locked") {
            showMessage("Enter the password, or unlock the vault if you saved it there.", "info", unlockAndRetry("password"));
          } else if (failure.vault === "unlocked") {
            showMessage("No password is saved for this host. Enter it, and tick Save in vault to keep it.", "info");
          } else {
            showMessage("Enter the password.", "info");
          }
          password.focus();
          break;
        case "error":
          showMessage(failure.message);
          break;
      }
    };

    form.onsubmit = (e) => {
      e.preventDefault();
      if (!host.value.trim() || !user.value.trim()) {
        showMessage("Enter a host and a user.");
        return;
      }
      const p = Number(port.value.trim() || "22");
      if (!Number.isInteger(p) || p < 1 || p > 65535) {
        showMessage("The port must be a number from 1 to 65535.");
        port.focus();
        return;
      }
      void attempt();
    };
    if (auto) void attempt();
  });
}
