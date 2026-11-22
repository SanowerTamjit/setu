// The jump-host map: which saved hosts are reached through which bastions,
// drawn left to right from this computer. Click a host to connect.

import { el, modal } from "./dom";
import type { Host } from "./ipc";

interface MapNode {
  id: string;
  label: string;
  detail: string;
  host: Host | null;
  level: number;
  row: number;
}

const SVG = "http://www.w3.org/2000/svg";
const COL = 230;
const ROW = 58;
const W = 180;
const H = 40;
const PAD = 24;

function svg<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string | number>, ...children: (Node | string)[]) {
  const node = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, String(v));
  for (const c of children) node.append(c);
  return node;
}

function parseHop(hop: string) {
  let rest = hop.trim();
  let user: string | null = null;
  const at = rest.lastIndexOf("@");
  if (at > 0) {
    user = rest.slice(0, at);
    rest = rest.slice(at + 1);
  }
  let port = 22;
  const m = /^(.*):(\d+)$/.exec(rest);
  if (m && !rest.startsWith("[")) {
    rest = m[1];
    port = Number(m[2]);
  }
  return { user, address: rest.replace(/^\[|\]$/g, ""), port };
}

function shortAddress(h: Host) {
  const port = h.port && h.port !== 22 ? `:${h.port}` : "";
  return `${h.user ? h.user + "@" : ""}${h.host}${port}`;
}

/** Nodes and edges for the hosts that use, or are used as, a jump host. */
export function buildMap(hosts: Host[]) {
  const nodes = new Map<string, MapNode>();
  const edges: [string, string][] = [];
  const find = (address: string, port: number) =>
    hosts.find((h) => (h.host === address || h.name === address) && (h.port ?? 22) === port) ??
    hosts.find((h) => h.host === address || h.name === address);
  const nodeFor = (host: Host | null, fallback: string, level: number) => {
    const id = host ? `host:${host.name}` : `ext:${fallback}`;
    const existing = nodes.get(id);
    if (existing) {
      existing.level = Math.min(existing.level, level);
      return existing;
    }
    const node: MapNode = {
      id,
      label: host ? host.name : fallback,
      detail: host ? shortAddress(host) : "not a saved host",
      host,
      level,
      row: 0,
    };
    nodes.set(id, node);
    return node;
  };
  for (const h of hosts) {
    if (!h.jump) continue;
    const hops = h.jump.split(",").map(parseHop);
    let previous = "local";
    hops.forEach((hop, i) => {
      const n = nodeFor(find(hop.address, hop.port) ?? null, `${hop.user ? hop.user + "@" : ""}${hop.address}${hop.port !== 22 ? ":" + hop.port : ""}`, i + 1);
      edges.push([previous, n.id]);
      previous = n.id;
    });
    const target = nodeFor(h, h.name, hops.length + 1);
    target.level = Math.max(target.level, hops.length + 1);
    edges.push([previous, target.id]);
  }
  const unique = [...new Set(edges.map((e) => e.join("→")))].map((e) => e.split("→") as [string, string]);
  // Rows per column, in the order hosts were seen, children near their parents.
  const columns = new Map<number, MapNode[]>();
  for (const n of nodes.values()) {
    const col = columns.get(n.level) ?? [];
    col.push(n);
    columns.set(n.level, col);
  }
  const parentRow = (n: MapNode) => {
    const from = unique.find(([, to]) => to === n.id)?.[0];
    return from && from !== "local" ? (nodes.get(from)?.row ?? 0) : 0;
  };
  for (const level of [...columns.keys()].sort((a, b) => a - b)) {
    const col = columns.get(level)!;
    col.sort((a, b) => parentRow(a) - parentRow(b) || a.label.localeCompare(b.label));
    col.forEach((n, i) => (n.row = i));
  }
  const direct = hosts.filter((h) => !h.jump && !nodes.has(`host:${h.name}`)).length;
  return { nodes: [...nodes.values()], edges: unique, direct };
}

export function openJumpMap(hosts: Host[], onConnect: (host: Host) => void): Promise<void> {
  return new Promise((resolve) => {
    const { nodes, edges, direct } = buildMap(hosts);
    const done = el("button", { type: "button", className: "primary" }, "Done");
    const canvas = el("div", { className: "jump-canvas" });
    const dialog = el(
      "div",
      { className: "dialog jump-dialog" },
      el("h2", {}, "Jump-host map"),
      el(
        "p",
        { className: "muted" },
        nodes.length
          ? `How your hosts are reached. ${direct} other ${direct === 1 ? "host connects" : "hosts connect"} directly. Click a host to connect.`
          : "None of your saved hosts uses a jump host yet. Set one in a host's settings, or import hosts that have one.",
      ),
      canvas,
      el("div", { className: "actions" }, el("span", { className: "spacer" }), done),
    );
    dialog.setAttribute("role", "dialog");
    dialog.setAttribute("aria-label", "Jump-host map");
    const { close } = modal(dialog, resolve);
    done.onclick = close;
    if (!nodes.length) return;

    const rowsByLevel = new Map<number, number>();
    for (const n of nodes) rowsByLevel.set(n.level, Math.max(rowsByLevel.get(n.level) ?? 0, n.row + 1));
    const maxRows = Math.max(1, ...rowsByLevel.values());
    const maxLevel = Math.max(...nodes.map((n) => n.level));
    const height = PAD * 2 + maxRows * ROW - (ROW - H);
    const width = PAD * 2 + maxLevel * COL + W;
    const pos = (n: MapNode) => {
      const rows = rowsByLevel.get(n.level) ?? 1;
      const offset = ((maxRows - rows) * ROW) / 2;
      return { x: PAD + n.level * COL, y: PAD + offset + n.row * ROW };
    };
    const local = { x: PAD, y: PAD + ((maxRows - 1) * ROW) / 2 };
    const byId = new Map(nodes.map((n) => [n.id, n]));

    const map = svg("svg", { width, height, viewBox: `0 0 ${width} ${height}`, class: "jump-svg", role: "img" });
    map.setAttribute("aria-label", "Jump-host map");
    for (const [from, to] of edges) {
      const a = from === "local" ? { x: local.x, y: local.y } : pos(byId.get(from)!);
      const b = pos(byId.get(to)!);
      const x1 = a.x + W;
      const y1 = a.y + H / 2;
      const x2 = b.x;
      const y2 = b.y + H / 2;
      const mid = (x1 + x2) / 2;
      map.append(svg("path", { d: `M${x1},${y1} C${mid},${y1} ${mid},${y2} ${x2},${y2}`, class: "jump-edge" }));
    }
    const box = (x: number, y: number, label: string, detail: string, cls: string) =>
      svg(
        "g",
        { transform: `translate(${x},${y})`, class: cls },
        svg("rect", { width: W, height: H, rx: 7 }),
        svg("text", { x: 12, y: 17, class: "jump-label" }, label.length > 24 ? label.slice(0, 23) + "…" : label),
        svg("text", { x: 12, y: 31, class: "jump-detail" }, detail.length > 30 ? detail.slice(0, 29) + "…" : detail),
      );
    map.append(box(local.x, local.y, "This computer", "where Setu runs", "jump-node local"));
    for (const n of nodes) {
      const { x, y } = pos(n);
      const isHop = edges.some(([from]) => from === n.id);
      const g = box(x, y, n.label, n.detail, `jump-node ${n.host ? "saved" : "external"} ${isHop ? "hop" : ""}`);
      if (n.host) {
        const host = n.host;
        g.setAttribute("tabindex", "0");
        g.setAttribute("role", "button");
        g.append(svg("title", {}, `Connect to ${host.name}`));
        const go = () => {
          close();
          onConnect(host);
        };
        g.addEventListener("click", go);
        g.addEventListener("keydown", (e) => {
          if ((e as KeyboardEvent).key === "Enter") go();
        });
      }
      map.append(g);
    }
    canvas.append(map);
    const legend = el(
      "div",
      { className: "jump-legend muted" },
      el("span", { className: "swatch hop" }),
      "jump host",
      el("span", { className: "swatch saved" }),
      "saved host",
      el("span", { className: "swatch external" }),
      "not saved",
    );
    canvas.after(legend);
  });
}
