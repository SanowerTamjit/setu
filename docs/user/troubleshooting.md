# Troubleshooting

Start with **`setu doctor`** in any terminal (see [The command line](cli.md)). It checks your keys, ssh-agent, `~/.ssh/config`, `known_hosts`, saved hosts and vault file, and prints a fix for each problem. It only reads; it changes nothing.

## Connecting

**"The host key has changed" and Setu will not connect.**
The server presents a different key from the one in `~/.ssh/known_hosts`. If you know why (the server was rebuilt), open SSH keys → **Known hosts**, **Forget** the old entry, and connect again to check the new fingerprint. If you don't know why, ask the server's administrator before going further. See [Connecting with SSH](ssh.md#trusting-a-server-host-keys).

**My key is not accepted, but `ssh` works.**
Check which key `ssh` uses (`ssh -v host`). If it comes from `IdentityFile` in `~/.ssh/config`, import the host (Hosts → Import) so Setu uses the same key, or choose **Key file** in the connect dialog. `setu doctor` flags keys other users can read, which OpenSSH and Setu refuse.

**It asks for my passphrase every time.**
Tick **Save in vault**, or add the key to ssh-agent (SSH keys → **Add to agent**).

**A security key (YubiKey) does not sign.**
An ssh-agent must be running (on Windows, the **OpenSSH Authentication Agent** service). Touch the key when it blinks. See [Security keys](ssh-keys.md#signing-in-with-a-security-key).

**A workflow step fails with a host-key or password error.**
Workflow runs never ask anything. Connect to the host once from a terminal to trust its key, and unlock the vault (or use ssh-agent) before running. See [Workflows](workflows.md#running).

## The vault

**I forgot the master password.**
There is no recovery, by design. Delete `vault.json` (see [Settings and your files](settings.md#where-setu-keeps-its-files)) and set up a new vault; your hosts are not affected.

**"Unlock automatically" is not offered on Linux.**
It needs a Secret Service (GNOME Keyring, KWallet or KeePassXC) running on D-Bus. The vault dialog says why it is missing.

## The terminal

**New output shows up late, or the screen flickers.**
Settings → Terminal → **Renderer**: try **Compatible (DOM)**. On Linux it is already the default, because some WebKitGTK setups draw GPU output one frame late.

**Prompt icons show as boxes, or right-hand prompts are misaligned.**
Check that the **Prompt themes** feature is on (Settings → Features), and open a new tab: character widths apply to new tabs. Run **Prompt check** from the command palette to see every kind of glyph. See [Appearance](appearance.md#prompt-themes-powerlevel10k-oh-my-zsh-starship-oh-my-posh).

**Colours look washed out over SSH.**
Many servers ignore the `COLORTERM` Setu sends. Add `export COLORTERM=truecolor` to your start-up file on the server.

**A paste asks me to confirm.**
That is the paste guard: the shell would run each pasted line at once. Check the lines, then paste. Turn it off in Settings → Features if you prefer.

**Setu reopens tabs I don't want.**
Quit Setu and delete `session.json` to start with one fresh tab.

## Installing

**macOS says the app can't be opened, or Windows shows SmartScreen.**
Nightly builds are not code-signed yet. See [Getting started](getting-started.md#1-install-setu) for the one-time steps.

## Reporting a problem

Open an issue on [GitHub](https://github.com/SanowerTamjit/setu/issues) with your system, the Setu version (bottom right of the status bar) and what you did. `setu doctor --json` output helps with SSH problems; it never includes passwords, passphrases or key contents, but read it before you paste it.

Found a security problem? Please do not open a public issue; follow [SECURITY.md](../../SECURITY.md).
