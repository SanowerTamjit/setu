import { describe, expect, it } from "vitest";
import { attemptLine, checkBroadcast, checkMac, checkTarget, fromLabel, ms, portHeadline, verdict } from "../src/net-tools";
import type { PortReport } from "../src/ipc";

describe("checkTarget", () => {
  it("takes names and addresses, trimmed", () => {
    expect(checkTarget(" db.internal ")).toBe("db.internal");
    expect(checkTarget("10.0.0.5")).toBe("10.0.0.5");
    expect(checkTarget("fe80::1")).toBe("fe80::1");
  });
  it("refuses anything a shell or ssh could read as more", () => {
    for (const bad of ["", "-oProxyCommand=x", "a b", "host;rm", "http://x", "a$(id)", "a\tb", "x".repeat(254)]) {
      expect(checkTarget(bad), bad).toBeNull();
    }
  });
});

describe("checkMac and checkBroadcast", () => {
  it("reads the common MAC spellings", () => {
    for (const m of ["3c:52:82:1a:9e:07", "3C-52-82-1A-9E-07", "3c52.821a.9e07", "3c5282 1a9e07"]) {
      expect(checkMac(m), m).toBe("3c:52:82:1a:9e:07");
    }
    expect(checkMac("3c:52:82:1a:9e")).toBeNull();
    expect(checkMac("zz:52:82:1a:9e:07")).toBeNull();
  });
  it("allows an empty or dotted broadcast address", () => {
    expect(checkBroadcast(" ")).toBe("");
    expect(checkBroadcast("192.168.1.255")).toBe("192.168.1.255");
    expect(checkBroadcast("192.168.1")).toBeNull();
    expect(checkBroadcast("192.168.1.256")).toBeNull();
    expect(checkBroadcast("host.lan")).toBeNull();
  });
});

const report = (over: Partial<PortReport> = {}): PortReport => ({
  attempts: [
    { result: "open", micros: 410, address: "10.0.0.5:5432" },
    { result: "open", micros: 700, address: "10.0.0.5:5432" },
  ],
  summary: { sent: 2, open: 2, minMs: 0.41, avgMs: 0.555, maxMs: 0.7 },
  from: "this computer",
  ...over,
});

describe("port check words", () => {
  it("formats times for people", () => {
    expect(ms(0.414)).toBe("0.41 ms");
    expect(ms(12.34)).toBe("12.3 ms");
    expect(ms(250.6)).toBe("251 ms");
  });
  it("says one line per attempt", () => {
    expect(attemptLine({ result: "open", micros: 1500, address: "10.0.0.5:22" }, 1)).toBe("1. open: connected to 10.0.0.5:22 in 1.5 ms");
    expect(attemptLine({ result: "refused", address: "h:1" }, 2)).toContain("nothing is listening");
    expect(attemptLine({ result: "timedOut" }, 3)).toContain("no answer");
    expect(attemptLine({ result: "failed", reason: "no route to host" }, 4)).toBe("4. failed: no route to host");
  });
  it("sums up open, closed and in between", () => {
    const open = report();
    expect(verdict(open)).toBe("open");
    expect(portHeadline(open, "db:5432")).toBe(
      "db:5432 is open from this computer · 2 of 2 connected · min 0.41 ms, average 0.56 ms, max 0.70 ms",
    );
    const closed = report({ attempts: [{ result: "timedOut" }], summary: { sent: 1, open: 0 }, from: "bastion" });
    expect(verdict(closed)).toBe("closed");
    expect(portHeadline(closed, "db:5432")).toBe("db:5432 is not reachable from bastion · 0 of 1 connected");
    expect(verdict(report({ summary: { sent: 3, open: 1, minMs: 1, avgMs: 1, maxMs: 1 } }))).toBe("mixed");
  });
  it("names where a check ran", () => {
    expect(fromLabel("")).toBe("from this computer");
    expect(fromLabel("bastion")).toBe("through bastion");
  });
});

import { commonName, dateText, expiryWords, tlsHeadline, tlsVerdict } from "../src/net-tools";
import type { TlsReport } from "../src/ipc";

const tls = (over: Partial<TlsReport> = {}, daysLeft = 54): TlsReport => ({
  trusted: true,
  protocol: "TLS 1.3",
  cipher: "TLS13_AES_128_GCM_SHA256",
  name: "example.com",
  from: "this computer",
  chain: [
    {
      subject: "CN=example.com,O=Example\\, Inc.",
      issuer: "CN=Example CA,O=Example",
      names: ["example.com"],
      notBefore: 1_790_000_000,
      notAfter: 1_800_000_000,
      daysLeft,
      sha256: "AA:BB",
    },
  ],
  ...over,
});

describe("TLS words", () => {
  it("dates and expiry", () => {
    expect(dateText(1_800_000_000)).toBe("2027-01-15");
    expect(expiryWords(54)).toBe("expires in 54 days");
    expect(expiryWords(1)).toBe("expires in 1 day");
    expect(expiryWords(0)).toBe("expires today");
    expect(expiryWords(-1)).toBe("expired 1 day ago");
  });
  it("judges trust and how soon it expires", () => {
    expect(tlsVerdict(tls())).toBe("good");
    expect(tlsVerdict(tls({}, 10))).toBe("mixed");
    expect(tlsVerdict(tls({}, -2))).toBe("bad");
    expect(tlsVerdict(tls({ trusted: false, problem: "the certificate has expired" }))).toBe("bad");
    expect(tlsVerdict(tls({ chain: [] }))).toBe("bad");
  });
  it("says it in one line", () => {
    expect(tlsHeadline(tls())).toBe("Trusted for example.com · expires in 54 days (2027-01-15)");
    expect(tlsHeadline(tls({ trusted: false, problem: "the certificate is not for db" }))).toBe(
      "Not trusted: the certificate is not for db · expires in 54 days (2027-01-15)",
    );
  });
  it("picks the common name", () => {
    expect(commonName("CN=example.com,O=Example\\, Inc.")).toBe("example.com");
    expect(commonName("O=Acme,CN=a\\,b")).toBe("a,b");
    expect(commonName("O=No CN")).toBe("O=No CN");
  });
});
