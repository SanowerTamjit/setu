import { describe, expect, it } from "vitest";
import { canStart, countsLabel, formFor, parsePort, readForm, usage, type TunnelForm } from "../src/tunnel-form";
import type { TunnelEntry, TunnelStatus } from "../src/ipc";

const form = (over: Partial<TunnelForm> = {}): TunnelForm => ({
  ...formFor(null, "db-01"),
  name: "Postgres",
  bindPort: "5433",
  targetPort: "5432",
  ...over,
});

const status = (over: Partial<TunnelStatus> = {}): TunnelStatus => ({
  phase: "running",
  active: 0,
  total: 0,
  failed: 0,
  reconnects: 0,
  ...over,
});

describe("readForm", () => {
  it("builds a loopback local tunnel by default", () => {
    const r = readForm(form());
    expect(r).toEqual({
      tunnel: {
        id: "",
        name: "Postgres",
        host: "db-01",
        kind: "local",
        bindAddress: "127.0.0.1",
        bindPort: 5433,
        targetHost: "127.0.0.1",
        targetPort: 5432,
        autoStart: false,
      },
    });
  });

  it("only listens on every address when asked", () => {
    const r = readForm(form({ shared: true }));
    expect("tunnel" in r && r.tunnel.bindAddress).toBe("0.0.0.0");
  });

  it("drops the destination for a SOCKS proxy", () => {
    const r = readForm(form({ kind: "dynamic", targetHost: "ignored", targetPort: "x" }));
    expect("tunnel" in r && r.tunnel.targetHost).toBeUndefined();
  });

  it("explains what is wrong", () => {
    expect(readForm(form({ name: " " }))).toEqual({ error: "Give the tunnel a name." });
    expect(readForm(form({ host: "" }))).toHaveProperty("error");
    expect(readForm(form({ bindPort: "0" }))).toHaveProperty("error");
    expect(readForm(form({ bindPort: "70000" }))).toHaveProperty("error");
    expect(readForm(form({ targetPort: "" }))).toHaveProperty("error");
    expect(readForm(form({ targetHost: "a b" }))).toHaveProperty("error");
    expect(readForm(form({ targetHost: "-oProxyCommand=x" }))).toHaveProperty("error");
  });
});

describe("words for a tunnel", () => {
  it("parses ports strictly", () => {
    expect(parsePort(" 22 ")).toBe(22);
    expect(parsePort("1e3")).toBeNull();
    expect(parsePort("-1")).toBeNull();
    expect(parsePort("65536")).toBeNull();
  });

  it("says how to use each kind", () => {
    const t = { id: "a", name: "A", host: "db-01", bindAddress: "127.0.0.1", bindPort: 5433, targetHost: "127.0.0.1", targetPort: 5432, autoStart: false };
    expect(usage({ ...t, kind: "local" })).toBe("Connect to localhost:5433 to reach 127.0.0.1:5432 through db-01.");
    expect(usage({ ...t, kind: "remote" })).toContain("On db-01, connections to port 5433");
    expect(usage({ ...t, kind: "dynamic", bindAddress: "0.0.0.0" })).toContain("this computer's address:5433");
  });

  it("shows counts only while running", () => {
    expect(countsLabel(status({ active: 2, total: 14 }))).toBe("2 open · 14 so far");
    expect(countsLabel(status({ active: 0, total: 3, failed: 1, reconnects: 2 }))).toBe("0 open · 3 so far · 1 refused · reconnected 2×");
    expect(countsLabel(status({ phase: "stopped", total: 9 }))).toBe("");
  });

  it("offers Start for stopped and failed tunnels", () => {
    const entry = (phase: TunnelStatus["phase"]) => ({ status: status({ phase }) }) as TunnelEntry;
    expect(canStart(entry("stopped"))).toBe(true);
    expect(canStart(entry("failed"))).toBe(true);
    expect(canStart(entry("reconnecting"))).toBe(false);
    expect(canStart(entry("running"))).toBe(false);
  });

  it("round-trips an existing tunnel through the form", () => {
    const t = { id: "a", name: "A", host: "db-01", kind: "remote" as const, bindAddress: "0.0.0.0", bindPort: 8080, targetHost: "localhost", targetPort: 3000, autoStart: true };
    expect(readForm(formFor(t, "x"))).toEqual({ tunnel: t });
  });
});
