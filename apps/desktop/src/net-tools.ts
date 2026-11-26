// The DOM-free parts of the Network tools view: checking what was typed and
// the words shown for each result. Kept apart so they can be tested in Node
// (test/net-tools.test.ts). Rust checks every input again (crates/setu-net).

import type { PortAttempt, PortReport, TlsReport } from "./ipc";
import { parsePort } from "./tunnel-form";

export { parsePort };

/** Mirrors setu_net::check_target: a host name or address, nothing else. */
export function checkTarget(text: string): string | null {
  const t = text.trim();
  if (!t || t.length > 253 || t.startsWith("-")) return null;
  return /^[A-Za-z0-9._:-]+$/.test(t) ? t : null;
}

/** Mirrors setu_net::parse_mac; returns it as 3c:52:82:1a:9e:07. */
export function checkMac(text: string): string | null {
  const hex = text.replace(/[:.\- ]/g, "");
  if (!/^[0-9a-fA-F]{12}$/.test(hex)) return null;
  return hex.toLowerCase().match(/../g)!.join(":");
}

/** Empty, or a dotted IPv4 address. Rust checks it is a broadcast address. */
export function checkBroadcast(text: string): string | null {
  const t = text.trim();
  if (!t) return "";
  const parts = t.split(".");
  return parts.length === 4 && parts.every((p) => /^\d{1,3}$/.test(p) && Number(p) <= 255) ? t : null;
}

export function ms(value: number): string {
  if (value < 1) return `${value.toFixed(2)} ms`;
  if (value < 100) return `${value.toFixed(1)} ms`;
  return `${Math.round(value)} ms`;
}

/** One line per attempt, like ping prints. */
export function attemptLine(a: PortAttempt, n: number): string {
  switch (a.result) {
    case "open":
      return `${n}. open: connected to ${a.address} in ${ms(a.micros / 1000)}`;
    case "refused":
      return `${n}. closed: ${a.address} refused the connection (nothing is listening)`;
    case "timedOut":
      return `${n}. no answer in 5 seconds (a firewall that drops, or the host is down)`;
    case "failed":
      return `${n}. failed: ${a.reason}`;
  }
}

export type Verdict = "open" | "closed" | "mixed";

export function verdict(r: PortReport): Verdict {
  if (r.summary.open === r.summary.sent) return "open";
  return r.summary.open === 0 ? "closed" : "mixed";
}

/** The headline, e.g. "Open from db-01 · 3 of 3 · 0.41 / 0.52 / 0.70 ms". */
export function portHeadline(r: PortReport, target: string): string {
  const s = r.summary;
  const from = r.from === "this computer" ? "from this computer" : `from ${r.from}`;
  const v = verdict(r);
  const word = v === "open" ? `${target} is open` : v === "closed" ? `${target} is not reachable` : `${target} answered sometimes`;
  const times =
    s.minMs != null && s.avgMs != null && s.maxMs != null
      ? ` · min ${ms(s.minMs)}, average ${ms(s.avgMs)}, max ${ms(s.maxMs)}`
      : "";
  return `${word} ${from} · ${s.open} of ${s.sent} connected${times}`;
}

/** Where a check runs from: "" is this computer, else a saved host's name. */
export function fromLabel(through: string): string {
  return through ? `through ${through}` : "from this computer";
}

/** A date as YYYY-MM-DD (UTC), from seconds since 1970. */
export function dateText(secs: number): string {
  return new Date(secs * 1000).toISOString().slice(0, 10);
}

export function expiryWords(days: number): string {
  if (days < 0) return `expired ${-days} day${days === -1 ? "" : "s"} ago`;
  if (days === 0) return "expires today";
  return `expires in ${days} day${days === 1 ? "" : "s"}`;
}

/** Soon means under three weeks: time to renew before people notice. */
export const SOON_DAYS = 21;

export type TlsVerdict = "good" | "mixed" | "bad";

export function tlsVerdict(r: TlsReport): TlsVerdict {
  const leaf = r.chain[0];
  if (!r.trusted || !leaf || leaf.daysLeft < 0) return "bad";
  return leaf.daysLeft < SOON_DAYS ? "mixed" : "good";
}

/** The headline, e.g. "Trusted for example.com · expires in 54 days (2027-01-18)". */
export function tlsHeadline(r: TlsReport): string {
  const leaf = r.chain[0];
  const when = leaf ? ` · ${expiryWords(leaf.daysLeft)} (${dateText(leaf.notAfter)})` : "";
  if (r.trusted) return `Trusted for ${r.name}${when}`;
  return `Not trusted: ${r.problem ?? "unknown reason"}${when}`;
}

/** The short name of a certificate: its CN, else the whole subject. */
export function commonName(subject: string): string {
  const m = /(?:^|,)\s*CN=((?:\\,|[^,])+)/.exec(subject);
  return m ? m[1].replace(/\\,/g, ",") : subject;
}
