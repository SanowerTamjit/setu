# SSH keys and known hosts

`Manage SSH keys` in the command palette (Ctrl+Shift+P) opens one dialog with two tabs. Everything here works on the same files as OpenSSH (`~/.ssh`), so `ssh`, `scp` and Setu stay in agreement.

## Keys

Lists the private keys in `~/.ssh` with their type, SHA-256 fingerprint and comment, and shows whether each has a passphrase and whether ssh-agent holds it. Setu never shows or sends private key material to the window.

- **New key** makes an Ed25519 key (recommended), ECDSA P-256 or RSA 4096 key in OpenSSH format, or a key on a security key (below),, with an optional passphrase (recommended). The private file is readable only by you (mode 0600), the folder is created private if missing, and an existing file is never overwritten.
- **Copy public key** copies the `.pub` line, ready for a server's `~/.ssh/authorized_keys`.
- **Add to agent** gives the key to the running ssh-agent after checking its passphrase, so connections stop asking for it.

## Security keys (YubiKey and other FIDO2 keys)

A security key keeps the private key on the device: the file in `~/.ssh` only points at it, so a copied file is useless without the key, and every sign-in needs a touch. Setu uses the same `sk-ssh-ed25519@openssh.com` and `sk-ecdsa-sha2-nistp256@openssh.com` keys as OpenSSH 8.2 and newer, so the server side is the usual `authorized_keys` line.

- **New key → Security key (Ed25519)** (or ECDSA, for older U2F-only keys) makes the key with OpenSSH's `ssh-keygen`, which must be installed. Plug the key in, click **Make key** and touch it when it blinks. Options:
  - **Store it on the security key too** (`-O resident`) so you can load it on another computer. Needs a FIDO2 key with a PIN.
  - **Ask for the PIN every time** (`-O verify-required`) for a PIN or fingerprint on each sign-in, not just a touch. The server must allow it.
  - **Security key PIN**, if the key has one. It is used once and not saved. On Windows the system's own security key window asks for the PIN and touch instead.
  - A passphrase is optional; it protects the handle file.
- **Load from security key** copies keys stored on the device into `~/.ssh` (like `ssh-keygen -K`). It needs the PIN, and never overwrites a file: a different key with a taken name gets a number added.
- Security keys show a **Security key** badge in the list.

### Signing in with a security key

Signing needs the device, and OpenSSH's ssh-agent is what talks to it. So when a host uses a security key file (`id_ed25519_sk`, `id_ecdsa_sk`, or one you pick), Setu hands the key to the running agent if it does not hold it yet, and the agent asks the device to sign: touch it when it blinks. The connect button says **Touch your security key…** while it waits.

- An agent must be running: on Linux and macOS your desktop's ssh-agent (`SSH_AUTH_SOCK`), on Windows the **OpenSSH Authentication Agent** service. Without one Setu says so instead of failing silently.
- If the key is unplugged or not touched in time, the error says the key did not sign. With **ssh-agent and keys in ~/.ssh**, a security key in the agent that cannot sign is skipped and the agent's other keys are still tried.
- `id_ed25519_sk` and `id_ecdsa_sk` are among the default keys tried, as in OpenSSH.

## Known hosts

Lists every line of `~/.ssh/known_hosts`: the host names (or "Hashed host name" when OpenSSH hashed them), key type and fingerprint, with `@cert-authority` and `@revoked` lines marked. Type to filter by name, type or fingerprint.

**Forget** (click twice) removes one entry, for example after a server was rebuilt and its key changed. The next connection then asks you to check the new key. Setu removes the line only if the file has not changed since it was read, writes the new file atomically, and keeps the previous one as `known_hosts.old`, as `ssh-keygen -R` does.
