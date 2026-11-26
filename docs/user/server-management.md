# Server management

See and manage a Linux server: its services (which are running, which have failed and why; start, stop or restart them), its processes, and what fills its disks. Setu uses the server's own tools (`systemctl`, `journalctl`, `ps`, `kill`, `df`, `du`) over SSH, so **nothing is installed on the server**. Every change shows you the exact command first and waits for your yes.

Open the **Server** view from the rail (Classic), the **Server** switch in the title bar (Studio), or the command palette (`Go to server management`). It can be turned off in Settings → Features (`serverManagement`).

![The services on a server, with a failed one open](images/server-services.png)

Pick a saved host at the top right, then a tab: **Services**, **Processes** or **Disk**. Setu remembers the host and the tab.

## Services

 Setu lists every service systemd has loaded, and those that are installed but not loaded (for example, turned off at boot and never started).

| In the list | Means |
|---|---|
| Green dot, **Running** | The service is up. **Done (ran once)** is a job that ran and finished without trouble. |
| Red dot, **Failed** | It stopped with an error. Open it to see why in its logs. |
| Hollow dot, **Stopped** | Not running. |
| Amber dot, **Starting** / **Stopping** | systemd is in the middle of it. |
| **Masked** | Turned off completely by an administrator; nothing can start it. |

Type in **Filter services** to find one by name or description, and use **All**, **Running**, **Failed** and **Stopped** to narrow the list (each shows how many). Press ↻ to read the list again.

Click a service to see its status: its main process, since when it runs, memory, tasks, how often systemd restarted it, whether it starts at boot, and its unit file.

## Logs

Under the status are the service's newest journal lines. Choose how many (**Last 50** to **Last 5000**) and which (**All lines**, **Warnings and errors**, **Errors only**). The line under the logs shows the exact command Setu ran.

If the logs only show a hint about "messages from other users and the system", your login may not read the system journal. Turn on **Use sudo**, or ask for your user to be added to the `systemd-journal` (or `adm`) group.

## Changing a service

The buttons offer only what makes sense now:

| Button | Runs | Does |
|---|---|---|
| **Start** | `systemctl start` | Starts it now. |
| **Restart** | `systemctl restart` | Stops and starts it. Connections to it drop for a moment. |
| **Reload** | `systemctl reload` | Asks it to read its settings again without stopping (not every service can). |
| **Stop** | `systemctl stop` | Stops it until it is started again. |
| **Start at boot** | `systemctl enable` | It starts each time the server boots. It does not start it now. |
| **Not at boot** | `systemctl disable` | It no longer starts at boot. It keeps running now. |

Before anything runs, Setu shows what will happen and the **exact command**, and waits. Stopping or restarting the service you log in with (`ssh`, `sshd`) or the network or firewall warns you that you could lose access.

![Setu shows the exact command and asks first](images/server-confirm.png)

Afterwards you see whether it worked and what the command printed, and the status and logs are read again. When it fails, Setu says why in plain words where it can (for example, that it needs administrator rights).

### Use sudo

Most changes need administrator rights. Tick **Use sudo** and Setu puts `sudo -n` in front of the commands (reads too, so you can see the system journal). `-n` means sudo fails at once instead of asking for a password: **Setu never types a password into sudo.** It works where your user may run `systemctl` and `journalctl` with sudo without a password. A line like this in `sudoers` (edit it with `visudo`) allows just those two:

```
deploy ALL=(root) NOPASSWD: /usr/bin/systemctl, /usr/bin/journalctl
```

If sudo wants a password, Setu tells you so; run the command in a terminal instead.

## Processes

The **Processes** tab lists what runs on the server, busiest first (or **Most memory first**): process id, user, CPU, share of memory, memory in use, how long it has run, and the command's name. Type to filter by name, user or process id; ↻ reads the list again. Up to 500 processes are listed.

Only the command's **name** is read, never its arguments, which can hold passwords. CPU is averaged over each process's whole life, the way `ps` counts it; for live figures use [Monitoring](monitoring.md).

Pick a process to stop it:

| Button | Runs | Does |
|---|---|---|
| **Stop** | `kill -TERM <pid>` | Asks it to stop; it can finish and clean up first. Try this first. |
| **Kill** | `kill -KILL <pid>` | Stops it at once, with no clean-up. Use it when Stop did not work. |

As with services, Setu shows the exact command and asks first, and warns before stopping what keeps the server up or reachable (`sshd`, `systemd`, the network). The system itself (process 1) cannot be stopped from here. Your own processes need no sudo; other users' need **Use sudo** (and sudo allowed to run `kill` without a password).

## Disk

The **Disk** tab shows each disk (not memory-backed or snap file systems): where it is mounted, its type and device, a bar, and how much is used and free. The bar turns amber from 80% and red from 90%.

To find what fills a disk, press **What uses it**, or type a folder and press **Measure**. Setu lists the folders in it, biggest first, staying on that disk (like `du -x -d 1`). Click a folder to go into it; use the path above the list or ↑ to go back up. Measuring a big folder can take up to a minute; folders you may not read are left out, so turn on **Use sudo** to see everything (sudo must be allowed to run `timeout` and `du` without a password). It only reads: nothing is deleted.

![What uses the space in /var](images/server-disk.png)

## Good to know

- Logins work like tunnels and monitoring: the key, agent or saved password is used without asking, and the server's host key must already be trusted (connect once from **Hosts**). Setu keeps the connection for five minutes so moving between services is quick.
- A build of Setu for an organization may refuse some changes by policy, and may record each change (who, which command, where, when and whether it worked; never the output).
- Needs systemd. On a server without it, Setu says so.
- Not yet: `authorized_keys` (coming next).
