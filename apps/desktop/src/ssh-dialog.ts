// The "New SSH connection" dialog, including the host-key trust prompt.

import { sshTrustHost, type AuthRequest, type ConnectFailure, type ConnectRequest } from "./ipc";

type Connect = (request: ConnectRequest) => Promise<void>;

/** Values from the last attempt, so a retry does not mean retyping. */
let last = { host: "", port: "22", user: "", authKind: "default", keyPath: "~/.ssh/id_ed25519", jump: "" };

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Record<string, unknown> = {},
  ...children: (Node | string)[]
): HTMLElementTagNameMap[K] {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

function field(label: string, input: HTMLElement, hint?: string) {
  return el("label", { className: "field" }, el("span", {}, label), input, ...(hint ? [el("small", { className: "muted" }, hint)] : []));
}

function isFailure(err: unknown): err is ConnectFailure {
  return typeof err === "object" && err !== null && "kind" in err;
}

export function openSshDialog(connect: Connect): Promise<void> {
  return new Promise((resolve) => {
    const host = el("input", { value: last.host, placeholder: "server.example.com", required: true, autocomplete: "off", spellcheck: false });
    const port = el("input", { value: last.port, inputMode: "numeric", autocomplete: "off" });
    const user = el("input", { value: last.user, placeholder: "root", required: true, autocomplete: "off", spellcheck: false });
    const authKind = el("select", {},
      el("option", { value: "default" }, "ssh-agent and keys in ~/.ssh"),
      el("option", { value: "keyFile" }, "Key file"),
      el("option", { value: "password" }, "Password"),
    );
    authKind.value = last.authKind;
    const keyPath = el("input", { value: last.keyPath, spellcheck: false });
    const passphrase = el("input", { type: "password", placeholder: "Only if the key is encrypted", autocomplete: "off" });
    const password = el("input", { type: "password", autocomplete: "off" });
    const jump = el("input", { value: last.jump, placeholder: "ops@bastion.example.com:22 (optional)", spellcheck: false });

    const keyFields = el("div", { className: "group" }, field("Key file", keyPath), field("Passphrase", passphrase));
    const passwordFields = el("div", { className: "group" }, field("Password", password, "Sent to the server only; not saved."));
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
      el("h2", {}, "New SSH connection"),
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
    document.body.append(overlay);
    (last.host ? user : host).focus();
    if (last.host && last.user) connectButton.focus();

    const close = () => {
      overlay.remove();
      document.removeEventListener("keydown", onKey, true);
      resolve();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        close();
      }
    };
    document.addEventListener("keydown", onKey, true);
    cancelButton.onclick = close;

    const showMessage = (text: string, kind: "error" | "info" = "error") => {
      message.hidden = false;
      message.className = `dialog-message ${kind}`;
      message.textContent = text;
    };

    const request = (): ConnectRequest => {
      let auth: AuthRequest = { kind: "default" };
      if (authKind.value === "keyFile") auth = { kind: "keyFile", path: keyPath.value.trim(), passphrase: passphrase.value || null };
      if (authKind.value === "password") auth = { kind: "password", password: password.value };
      return {
        host: host.value.trim(),
        port: Number(port.value.trim() || "22"),
        user: user.value.trim(),
        auth,
        jump: jump.value.trim() || null,
      };
    };

    const attempt = async () => {
      last = { host: host.value.trim(), port: port.value, user: user.value.trim(), authKind: authKind.value, keyPath: keyPath.value, jump: jump.value.trim() };
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
          showMessage(`The key ${failure.path} is encrypted. Enter its passphrase.`, "info");
          passphrase.focus();
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
  });
}
