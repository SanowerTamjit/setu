// The Network tools view: port check (a TCP ping), DNS lookup, ping and
// Wake-on-LAN, from this computer or through a saved host. Each tool is a
// card with its own form and result. The Rust side (src-tauri/src/nettools.rs)
// checks every input again and runs nothing in a shell.

import { el, errorText, field } from "./dom";
import { icon, type IconName } from "./icons";
import { netCheckPort, netLookup, netPing, netWake, type Host } from "./ipc";
import { attemptLine, checkBroadcast, checkMac, checkTarget, fromLabel, parsePort, portHeadline, verdict } from "./net-tools";

export interface NetActions {
  hosts: () => Host[];
}

interface Tool {
  card: HTMLElement;
  result: HTMLElement;
  run: HTMLButtonElement;
}

export class NetView {
  private readonly from: HTMLSelectElement;
  private readonly pingNote: HTMLElement;
  private pingRun!: HTMLButtonElement;
  private first!: HTMLInputElement;

  constructor(
    private readonly root: HTMLElement,
    private readonly actions: NetActions,
  ) {
    this.from = el("select", { className: "net-from-select" });
    this.from.setAttribute("aria-label", "Run checks from");
    this.from.onchange = () => this.syncFrom();
    this.pingNote = el("small", { className: "muted net-note" });
    this.root.replaceChildren(
      el(
        "div",
        { className: "home-inner" },
        el(
          "header",
          { className: "home-header" },
          el("div", { className: "home-title" }, el("h1", {}, "Network tools")),
          el("label", { className: "home-actions net-from" }, el("span", { className: "muted" }, "Run from"), this.from),
        ),
        el(
          "p",
          { className: "muted tunnel-intro" },
          "Check what a computer or a server can reach. Through a saved host, checks run on that server, so you see its network, not yours. Setu logs in the way the host's tunnels and workflows do.",
        ),
        el("div", { className: "net-grid" }, this.portTool(), this.lookupTool(), this.pingTool(), this.wakeTool()),
      ),
    );
  }

  focus() {
    this.first.focus();
  }

  /** Fills "Run from" with the saved hosts, keeping the choice. */
  refresh() {
    const keep = this.from.value;
    const names = this.actions.hosts().map((h) => h.name);
    this.from.replaceChildren(
      el("option", { value: "" }, "This computer"),
      ...(names.length ? [el("optgroup", { label: "Through a saved host" }, ...names.map((n) => el("option", { value: n }, n)))] : []),
    );
    this.from.value = names.includes(keep) ? keep : "";
    this.syncFrom();
  }

  private get through(): string {
    return this.from.value;
  }

  private syncFrom() {
    const local = !this.through;
    this.pingRun.disabled = local;
    this.pingNote.textContent = local
      ? "Ping needs rights an app does not have, so it runs on a server: pick a saved host in Run from. To check a service from here, use the port check."
      : `Runs ping -c 4 on ${this.through}.`;
  }

  private tool(name: IconName, title: string, help: string, fields: HTMLElement[], label: string, go: () => Promise<void>): Tool {
    const result = el("div", { className: "net-result", hidden: true });
    result.setAttribute("aria-live", "polite");
    const run = el("button", { type: "button", className: "card-connect" }, icon("play", 15), label);
    const form = el("form", { className: "net-form" }, ...fields, el("div", { className: "card-actions" }, run));
    form.onsubmit = async (e) => {
      e.preventDefault();
      if (run.disabled) return;
      run.disabled = true;
      result.hidden = false;
      result.className = "net-result busy";
      result.replaceChildren(el("span", { className: "muted" }, "Checking…"));
      try {
        await go();
      } catch (err) {
        this.show(result, "bad", errorText(err));
      } finally {
        run.disabled = false;
        this.syncFrom();
      }
    };
    run.type = "submit";
    const card = el(
      "section",
      { className: "host-card net-card" },
      el(
        "div",
        { className: "card-top" },
        el("span", { className: "avatar tunnel-kind" }, icon(name, 18)),
        el("div", { className: "card-heading" }, el("h2", { className: "card-name" }, title)),
      ),
      el("p", { className: "tunnel-usage" }, help),
      form,
      result,
    );
    return { card, result, run };
  }

  private show(result: HTMLElement, kind: "good" | "bad" | "mixed", headline: string, detail?: HTMLElement) {
    result.className = `net-result ${kind}`;
    result.hidden = false;
    result.replaceChildren(el("p", { className: "net-headline" }, headline), ...(detail ? [detail] : []));
  }

  private bad(result: HTMLElement, text: string) {
    this.show(result, "bad", text);
  }

  private portTool() {
    const host = el("input", { placeholder: "db.internal or 10.0.0.5", spellcheck: false, autocomplete: "off" });
    const port = el("input", { placeholder: "5432", inputMode: "numeric", autocomplete: "off" });
    const tries = el("select", {}, ...[1, 3, 5, 10].map((n) => el("option", { value: String(n), selected: n === 3 }, String(n))));
    this.first = host;
    const t = this.tool(
      "connect",
      "Port check",
      "Can a connection be made, and how long does it take? Works where ping is blocked, and needs no rights.",
      [el("div", { className: "net-row" }, field("Host", host), field("Port", port), field("Tries", tries))],
      "Check",
      async () => {
        const target = checkTarget(host.value);
        const p = parsePort(port.value);
        if (!target) return this.bad(t.result, "Type a host name or IP address, like db.internal or 10.0.0.5.");
        if (p === null) return this.bad(t.result, "The port is a number from 1 to 65535.");
        const r = await netCheckPort(target, p, Number(tries.value), this.through);
        const v = verdict(r);
        this.show(
          t.result,
          v === "open" ? "good" : v === "closed" ? "bad" : "mixed",
          portHeadline(r, `${target}:${p}`),
          el("pre", { className: "net-lines" }, r.attempts.map((a, i) => attemptLine(a, i + 1)).join("\n")),
        );
      },
    );
    return t.card;
  }

  private lookupTool() {
    const name = el("input", { placeholder: "api.example.com", spellcheck: false, autocomplete: "off" });
    const t = this.tool(
      "search",
      "DNS lookup",
      "What addresses a name has. Through a host, its own resolver answers: handy for names that only exist inside a network.",
      [field("Name", name)],
      "Look up",
      async () => {
        const n = checkTarget(name.value);
        if (!n) return this.bad(t.result, "Type a host name, like api.example.com.");
        const r = await netLookup(n, this.through);
        const where = r.from === "this computer" ? "from this computer" : `from ${r.from}`;
        this.show(
          t.result,
          "good",
          `${n} has ${r.addresses.length} address${r.addresses.length === 1 ? "" : "es"} ${where}`,
          el("ul", { className: "net-addresses" }, ...r.addresses.map((a) => el("li", {}, el("code", {}, a)))),
        );
      },
    );
    return t.card;
  }

  private pingTool() {
    const host = el("input", { placeholder: "10.0.0.5", spellcheck: false, autocomplete: "off" });
    const t = this.tool("pulse", "Ping", "Is a host up, and how long do packets take? Sends four pings from the server.", [field("Host", host), this.pingNote], "Ping", async () => {
      const target = checkTarget(host.value);
      if (!target) return this.bad(t.result, "Type a host name or IP address.");
      if (!this.through) return this.bad(t.result, "Pick a saved host in Run from first.");
      const r = await netPing(target, this.through);
      this.show(
        t.result,
        r.ok ? "good" : "bad",
        r.ok ? `${target} answered ${fromLabel(r.from)}` : `${target} did not answer ${fromLabel(r.from)}`,
        el("pre", { className: "net-lines" }, r.output.trim() || "(no output)"),
      );
    });
    this.pingRun = t.run;
    return t.card;
  }

  private wakeTool() {
    const mac = el("input", { placeholder: "3c:52:82:1a:9e:07", spellcheck: false, autocomplete: "off" });
    const broadcast = el("input", { placeholder: "255.255.255.255", spellcheck: false, autocomplete: "off" });
    const t = this.tool(
      "power",
      "Wake-on-LAN",
      "Turns on a computer on this computer's network that has Wake-on-LAN enabled. Always sent from this computer.",
      [
        el(
          "div",
          { className: "net-row" },
          field("MAC address", mac),
          field("Broadcast address", broadcast, "Optional, like 192.168.1.255"),
        ),
      ],
      "Wake",
      async () => {
        const m = checkMac(mac.value);
        const b = checkBroadcast(broadcast.value);
        if (!m) return this.bad(t.result, "Type a MAC address, like 3c:52:82:1a:9e:07.");
        if (b === null) return this.bad(t.result, "The broadcast address is an IPv4 address ending in .255, or empty.");
        await netWake(m, b);
        this.show(
          t.result,
          "good",
          `Sent the wake-up packet for ${m} to ${b || "255.255.255.255"}`,
          el("p", { className: "muted net-small" }, "Nothing answers a wake-up packet: give the computer a minute, then try a port check."),
        );
      },
    );
    return t.card;
  }
}
