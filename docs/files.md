# Files (SFTP)

A Files tab shows your computer on the left and an SSH host on the right, and copies files between them over SFTP. It uses the connection of an open SSH tab, so there is no second login.

## Opening a Files tab

- From an SSH tab: click **Files** in the status bar, or press **Ctrl+Shift+E**.
- From the hosts sidebar: click **⇅** on a host. Setu connects first if needed, then opens the Files tab.

The Files tab keeps the connection open even if you close its SSH tab.

## Browsing

- Double-click a folder (or press **Enter**) to open it; **Backspace** or the **↑** button goes up. Type a path in the path box and press **Enter** to jump there.
- Click to select, **Shift**-click for a range, **Ctrl**-click to add or remove one item, **Ctrl+A** for all.
- **.\*** shows or hides hidden files (names starting with a dot).
- On the remote side: **New folder**, **Rename** (**F2**) and **Delete** (**Delete** key). Deleting asks first, and deletes folders with everything inside.

## Copying

- Select items and click **Upload →** or **← Download**, or drag them from one side to the other. Items go into the folder open on the other side.
- If something with the same name is already there, Setu asks before replacing it.
- Folders are copied with everything inside. Symbolic links to folders are skipped, so a link loop cannot copy forever.
- Copies run one after another in the **Transfers** list, with bytes done, speed and file count. **✕** cancels a running or waiting copy.
- A cancelled or failed copy deletes the partly written file, so you are never left with a truncated file that looks complete.

## Limits in this version

- Permissions and modification times are not copied; new files get the server's or your system's defaults.
- No resume of an interrupted copy, and no copy between two remote hosts.
- No editing of remote files in place yet.
