# Terminal tabs

Each tab is a local shell or an SSH session. Shortcuts use Ctrl+Shift (Cmd+Shift on macOS), so plain Ctrl keys still reach the shell.

## Keyboard shortcuts

| Shortcut | Action |
|----------|--------|
| Ctrl+Shift+T | New tab with your default shell |
| Ctrl+Shift+O | New SSH connection |
| Ctrl+Shift+W | Close the tab |
| Ctrl+Tab, Ctrl+Shift+Tab | Next and previous tab |
| Ctrl+Shift+H | Show the hosts sidebar and search it |
| Ctrl+Shift+F | Find in the tab's scrollback |
| Ctrl+Shift+E | Open Files (SFTP) on the SSH tab's server |

## Find in scrollback

Press **Ctrl+Shift+F** in a terminal tab. If text is selected on one line, the search starts with it.

- Results update as you type. **Enter** goes to the next match, **Shift+Enter** to the previous one; the ↑ and ↓ buttons do the same.
- The counter shows which match is selected out of how many. Above 1,000 matches it shows "1000+" and only highlights the first 1,000.
- **Aa** matches case, **ab** matches whole words only, and **.\*** treats the text as a regular expression (JavaScript syntax). An unfinished pattern shows "Invalid pattern" instead of searching.
- **Escape** closes the bar, clears the highlights and returns to the shell.

The search covers the tab's scrollback, which keeps the last 10,000 lines.
