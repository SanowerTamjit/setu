# Terminal tabs

Each tab is a local shell or an SSH session. Shortcuts use Ctrl+Shift (Cmd+Shift on macOS), so plain Ctrl keys still reach the shell.

## Keyboard shortcuts

| Shortcut | Action |
|----------|--------|
| Ctrl+Shift+T | New tab with your default shell |
| Ctrl+Shift+O | New SSH connection |
| Ctrl+Shift+W | Close the focused pane, or the tab when it has one pane |
| Ctrl+Shift+D | Split the focused pane side by side |
| Ctrl+Alt+Shift+D | Split the focused pane stacked (one above the other) |
| Alt+Arrow keys | Move focus to the pane in that direction |
| Ctrl+Tab, Ctrl+Shift+Tab | Next and previous tab |
| Ctrl+Shift+H | Show the hosts sidebar and search it |
| Ctrl+Shift+F | Find in the tab's scrollback |
| Ctrl+Shift+E | Open Files (SFTP) on the SSH tab's server |

## Split panes

A tab can hold several terminals. **Ctrl+Shift+D** splits the focused pane side by side, **Ctrl+Alt+Shift+D** splits it stacked; both are also in the **▾** menu next to the new-tab button. Splits nest, so you can build any grid.

- **What opens in the new pane:** for an SSH pane, another shell on the same server over the same connection, so there is no new login, password or passphrase prompt (jump hosts are shared too). For a local pane, a new shell of the same kind (bash, PowerShell, …).
- **Focus:** click a pane or use **Alt+Arrow keys**. The focused pane has an orange line along its top; the others are slightly dimmed. The tab title, the status bar, **Files** and **Find** follow the focused pane.
- **Resize:** drag the line between two panes. Double-click it to share the space evenly again.
- **Close:** **Ctrl+Shift+W** closes the focused pane and its neighbour takes the space. A shell that exits (`exit`, Ctrl+D) closes its pane too, unless it is the last one in the tab, which stays to show the exit code. The **×** on the tab closes all of its panes.
- The tab shows **⊞ 2**, **⊞ 3**, … when it holds more than one pane.

Closing one SSH pane leaves the connection open for the others; it closes when the last pane or Files tab using it closes.

## Find in scrollback

Press **Ctrl+Shift+F** in a terminal tab (in a split tab, it searches the focused pane). If text is selected on one line, the search starts with it.

- Results update as you type. **Enter** goes to the next match, **Shift+Enter** to the previous one; the ↑ and ↓ buttons do the same.
- The counter shows which match is selected out of how many. Above 1,000 matches it shows "1000+" and only highlights the first 1,000.
- **Aa** matches case, **ab** matches whole words only, and **.\*** treats the text as a regular expression (JavaScript syntax). An unfinished pattern shows "Invalid pattern" instead of searching.
- **Escape** closes the bar, clears the highlights and returns to the shell.

The search covers the tab's scrollback, which keeps the last 10,000 lines.
