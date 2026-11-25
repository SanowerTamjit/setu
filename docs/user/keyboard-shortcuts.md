# Keyboard shortcuts

On macOS, Cmd works wherever this page says Ctrl. Setu's own shortcuts use **Ctrl+Shift**, so plain Ctrl keys (Ctrl+C, Ctrl+R, Ctrl+W…) still reach your shell. Every command, with its shortcut, is also in the command palette (**Ctrl+Shift+P**).

## Everywhere

| Shortcut | Action |
|---|---|
| Ctrl+Shift+P | Command palette |
| Ctrl+, | Settings |
| Ctrl+Shift+T | New tab with your default shell |
| Ctrl+Shift+O | New SSH connection |
| Ctrl+Tab, Ctrl+Shift+Tab | Next and previous tab |
| Ctrl+Shift+H | Show the hosts sidebar and search it |
| Ctrl+Shift+N | Move the tab into a window of its own |

## In a terminal

| Shortcut | Action |
|---|---|
| Ctrl+Shift+D | Split the focused pane side by side |
| Ctrl+Alt+Shift+D | Split the focused pane stacked |
| Alt+Arrow keys | Move focus to the pane in that direction |
| Ctrl+Shift+W | Close the focused pane (or the tab, if it has one pane) |
| Ctrl+Shift+F | Find in the scrollback (Enter next, Shift+Enter previous, Escape closes) |
| Ctrl+Shift+E | Open Files (SFTP) on this SSH tab's server |
| Ctrl+Shift+C, Ctrl+Shift+V | Copy the selection, paste |

## Vim and Emacs profiles

Settings → **Keybindings** adds a prefix key, as tmux and Emacs do. Every Ctrl+Shift shortcut above keeps working. Press the prefix, then one key, within two seconds; the status bar shows the prefix while it waits. Press the prefix twice to send it to the shell.

| Action | Vim: Ctrl+Space, then | Emacs |
|---|---|---|
| Command palette | `:` | Alt+X |
| New tab | `t` | Ctrl+X `t` |
| New SSH connection | `o` | Ctrl+X Ctrl+F |
| Next / previous tab | `n` / `p` | Ctrl+X Right / Ctrl+X Left |
| Split side by side / stacked | `v` / `s` | Ctrl+X `3` / Ctrl+X `2` |
| Close pane | `x` | Ctrl+X `0` |
| Close tab | | Ctrl+X `k` |
| Move between panes | `h` `j` `k` `l` | Ctrl+X `o` |
| Find | `/` | Ctrl+X `s` |
| Files | `f` | Ctrl+X `d` |

## Hosts view

| Key | Action |
|---|---|
| Typing | Search by name, address, user, folder, jump host or tag |
| Enter | Connect to the focused card (in the search box: to the first match) |
| E or F2 | Edit the focused host |
| Arrow keys | Move between cards |

## Files tab

| Key | Action |
|---|---|
| Enter, double-click | Open a folder |
| Backspace | Go up one folder |
| Shift+click, Ctrl+click, Ctrl+A | Select a range, one more item, everything |
| F2 | Rename (server side) |
| Delete | Delete (server side; asks first) |
| F4 | Edit the file in your own editor |

## Task board

| Key | Action |
|---|---|
| Alt+← / Alt+→ | Move the focused card to the previous or next column |
| Alt+↑ / Alt+↓ | Move it up or down in its column |
| Enter | Open the card |
