# SSH keys and known hosts

`Manage SSH keys` in the command palette (Ctrl+Shift+P) opens one dialog with two tabs. Everything here works on the same files as OpenSSH (`~/.ssh`), so `ssh`, `scp` and Setu stay in agreement.

## Keys

Lists the private keys in `~/.ssh` with their type, SHA-256 fingerprint and comment, and shows whether each has a passphrase and whether ssh-agent holds it. Setu never shows or sends private key material to the window.

- **New key** makes an Ed25519 key (recommended), ECDSA P-256 or RSA 4096 key in OpenSSH format, with an optional passphrase (recommended). The private file is readable only by you (mode 0600), the folder is created private if missing, and an existing file is never overwritten.
- **Copy public key** copies the `.pub` line, ready for a server's `~/.ssh/authorized_keys`.
- **Add to agent** gives the key to the running ssh-agent after checking its passphrase, so connections stop asking for it.

## Known hosts

Lists every line of `~/.ssh/known_hosts`: the host names (or "Hashed host name" when OpenSSH hashed them), key type and fingerprint, with `@cert-authority` and `@revoked` lines marked. Type to filter by name, type or fingerprint.

**Forget** (click twice) removes one entry, for example after a server was rebuilt and its key changed. The next connection then asks you to check the new key. Setu removes the line only if the file has not changed since it was read, writes the new file atomically, and keeps the previous one as `known_hosts.old`, as `ssh-keygen -R` does.
