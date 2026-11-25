# Monitoring

See how busy your servers are, live: CPU, memory, network traffic and how full each disk is. Setu reads the figures Linux keeps in `/proc` and from `df` over SSH, so **nothing is installed on the server** and no port is opened.

Open the **Monitoring** view from the rail (Classic), the **Monitoring** switch in the title bar (Studio), or the command palette (`Go to monitoring`). It can be turned off in Settings → Features (`monitoring`).

![Watching a host with high CPU and a nearly full disk](images/monitoring.png)

## Watching a host

Pick a saved host at the top right and press **Watch**. A panel appears and fills in after a few seconds:

| Tile | Shows |
|---|---|
| **CPU** | How much of all CPUs was busy since the last reading, the number of CPUs and the load average (1, 5 and 15 minutes), with a two-minute history. |
| **Memory** | Memory in use (what Linux reports as not available) out of the total, and swap in use. |
| **Network** | Bytes a second received (↓, solid line) and sent (↑, dashed line) on every interface except loopback. |
| **Disks** | Each real file system (not `tmpfs`, `overlay` or snap loop devices): its mount point, a bar, and space used. |

The panel's subtitle shows how long the server has been up. Panels keep updating while you work in other views; close a panel (✕) to stop watching that host.

## Details

Under the disks, tabs open more detail (press the open tab again to close it). **Last hour** is described under [Alerts](#the-last-hour); the others are:

| Tab | Shows |
|---|---|
| **Processes** | The twelve busiest processes: process id, command name, CPU and memory. CPU counts like `top`, so 100% is one whole core and a busy process on a 4-CPU server can show up to 400%. The note under the table says how many processes the server runs. |
| **CPU cores** | One bar per CPU, coloured like the tiles (amber near the limit, red at it), so you can tell one stuck process from a server that is busy all over. |
| **Interfaces** | Bytes a second received and sent on each network interface, loopback included. |
| **Disk activity** | Bytes a second read and written on each whole disk (partitions, loop and RAM disks are left out). Containers often do not report this, and the tab says so. |

![The busiest processes on a host](images/monitoring-processes.png)

Process names come from the server and are shown only as text, cut to 64 characters. Setu does not read command lines or arguments, which can hold passwords.

## Many hosts at once

Watch as many as 24 hosts at once. **Watch all** starts every saved host (up to 24) in one go.

With more than a couple of hosts, switch from **Panels** to **Table** (at the right, above the panels). The table has one row per host: a coloured dot for its health, whether it is live, CPU, memory, its fullest disk, network traffic and its alerts. A host that stopped says why.

| Dot | Means |
|---|---|
| Green | Every figure is well under its limit. |
| Amber | A figure is within 15 points of its limit, or over it but not for long enough to alert yet. |
| Red | An alert is firing, a figure is at its limit, or the host stopped. |
| Hollow | Connecting or reconnecting. |

Click a host's name to open its panel. Setu remembers your choice of panels or table.

![Four hosts in the table: two alerting, one waiting, one stopped](images/monitoring-table.png)

### Watch when Setu opens

Tick **Watch when Setu opens** on a host's panel, and Setu starts watching it each time it opens, without you opening Monitoring. Its alerts and notifications work from the start. The list is kept on this computer; a host you delete or rename drops off it.

## Alerts

Under **Alert above**, set the limits for CPU, memory and disk (90% by default), and how long a reading must stay over its limit before it counts as an alert: **right away**, or **for** 1, 2 (the default), 5 or 10 minutes. Waiting means a short spike, such as a build or a backup starting, does not alert anyone.

- While a reading is over its limit but has not lasted that long, the panel shows an amber chip that says when it will alert, such as *CPU at 97% · alerts in 1 min*. Figures within 15 points of a limit also turn amber.
- Once it has lasted, the chip turns red and says for how long, such as *Disk /var is 93% full · 12 min*. The view's title counts these alerts.
- A reading that drops under its limit, even once, starts its wait again.

![One alert waiting, one firing](images/monitoring-alerts.png)

**Every** sets how often Setu reads the server: 2, 5, 10 or 30 seconds. Changes apply to every watched host straight away, and Setu remembers them on this computer.

### Desktop notifications

With **Notify me** ticked (the default), Setu shows a desktop notification when an alert starts, such as *db-01: CPU at 100%*, and another when it is back under the limit. You see them while you work in other apps, as long as Setu is open and the host is watched. Untick it to keep alerts inside the app.

Notifications use your system's own notification centre, so its settings (do not disturb, sounds) apply. On macOS, allow notifications for Setu the first time it asks. On Linux, they need a notification service, which every desktop has.

### The last hour

The **Last hour** tab draws CPU and memory since you started watching the host, up to an hour, with the alert limits as dashed lines. The time scale grows with the readings (5, 15, 30, then 60 minutes), and each point is the highest reading in its slice, so a short spike still shows. The history is kept only while the panel is open and is never saved.

![CPU over the last few minutes, with the alert limit](images/monitoring-hour.png)

## Logins and problems

Monitoring logs in the way [tunnels](tunnels.md) do, and never asks a question:

- The server's host key must already be trusted. Connect once from [Hosts](hosts.md) to check and trust it.
- A password or key passphrase comes from the [vault](vault.md), or use ssh-agent.

If the connection drops, the panel says **Reconnecting…** and tries again. After ten failures in a row, or a problem that will not fix itself (an unknown host key, a locked vault, a server that is not Linux), the panel stops and says why. Close it and press **Watch** again once the problem is fixed.

![A host whose key is not trusted yet](images/monitoring-problem.png)

## What runs on the server

Each reading runs one fixed, read-only command, the same for every server:

```sh
export LC_ALL=C; for f in stat meminfo loadavg uptime net/dev diskstats; do echo "@@$f"; cat /proc/$f 2>/dev/null; done; echo @@df; df -P -k 2>/dev/null; echo @@pagesize; getconf PAGESIZE 2>/dev/null; echo @@procs; cat /proc/[0-9]*/stat 2>/dev/null | head -c 524288
```

It takes nothing from you or from the host's settings. Setu reads only numbers from what comes back (and disk mount points, device, interface and process names, cleaned of control characters), keeps nothing on disk, and logs nothing. It works on Linux servers; other systems (macOS, BSD, Windows) are not supported yet.
