import { describe, expect, it } from "vitest";
import type { Finding } from "../src/ipc";
import { actionLabel, agoWords, badgeText, checkedWords, filterFindings, groupByHost, progressWords, untilWords } from "../src/problems-model";

const f = (id: string, severity: Finding["severity"], host: string | undefined, title: string, source: Finding["source"] = "check"): Finding => ({ id, severity, host, title, detail: `detail of ${title}`, source });

const list = [
  f("unit:web1:nginx.service", "error", "web1", "nginx.service failed"),
  f("reboot:web1", "info", "web1", "Restart needed to finish updates"),
  f("disk:db1:/var", "warning", "db1", "/var is 91% full"),
  f("monitor:db1:cpu", "warning", "db1", "CPU at 97%", "monitor"),
  f("workflow:deploy:r1", "error", undefined, "Workflow “Deploy” failed", "workflow"),
  f("clock:app1", "warning", "app1", "Clock is not synchronised"),
];

describe("filtering problems", () => {
  it("filters by severity, host and words", () => {
    const all = { severity: "all" as const, host: "", text: "" };
    expect(filterFindings(list, all)).toHaveLength(6);
    expect(filterFindings(list, { ...all, severity: "error" }).map((p) => p.id)).toEqual(["unit:web1:nginx.service", "workflow:deploy:r1"]);
    expect(filterFindings(list, { ...all, host: "db1" })).toHaveLength(2);
    expect(filterFindings(list, { ...all, text: "NGINX failed" })).toHaveLength(1);
    // The source name counts as a word too.
    expect(filterFindings(list, { ...all, text: "monitoring" }).map((p) => p.id)).toEqual(["monitor:db1:cpu"]);
    expect(filterFindings(list, { ...all, text: "web1 restart" }).map((p) => p.id)).toEqual(["reboot:web1"]);
  });
});

describe("grouping by host", () => {
  it("puts the worst hosts first and problems without a host last", () => {
    const groups = groupByHost(list);
    expect(groups.map((g) => g.host)).toEqual(["web1", "app1", "db1", null]);
    expect(groups.map((g) => g.worst)).toEqual(["error", "warning", "warning", "error"]);
    expect(groups[0].findings.map((p) => p.severity)).toEqual(["error", "info"]);
  });

  it("handles an empty list", () => {
    expect(groupByHost([])).toEqual([]);
  });
});

describe("words", () => {
  it("counts errors and warnings on the badge", () => {
    expect(badgeText({ errors: 0, warnings: 0, infos: 4 })).toBe("");
    expect(badgeText({ errors: 2, warnings: 3, infos: 1 })).toBe("5");
    expect(badgeText({ errors: 100, warnings: 0, infos: 0 })).toBe("99+");
  });

  it("says how long ago and how long until", () => {
    const now = 1_000_000_000_000;
    expect(agoWords(now - 20_000, now)).toBe("just now");
    expect(agoWords(now - 5 * 60_000, now)).toBe("5 min");
    expect(agoWords(now - 3 * 3600_000, now)).toBe("3 h");
    expect(agoWords(now - 3 * 86400_000, now)).toBe("3 days");
    expect(agoWords(now + 5000, now)).toBe("just now");
    expect(untilWords(now + 45 * 60_000, now)).toBe("in 45 min");
    expect(untilWords(now + 24 * 3600_000, now)).toBe("in 24 h");
    expect(untilWords(now + 7 * 86400_000, now)).toBe("in 7 days");
  });

  it("names actions", () => {
    expect(actionLabel({ kind: "service", host: "web1", unit: "nginx.service" })).toBe("Open service");
    expect(actionLabel({ kind: "connect", host: "web1" })).toBe("Connect");
  });

  it("says when hosts were checked", () => {
    const now = 1_000_000_000_000;
    expect(checkedWords({ checked: 0, checking: false }, 0, now)).toBe("No saved hosts to check.");
    expect(checkedWords({ checked: 0, checking: false }, 3, now)).toBe("Your hosts have not been checked since Setu started.");
    expect(checkedWords({ checked: 0, checking: true }, 3, now)).toBe("Checking your hosts…");
    expect(checkedWords({ checked: 3, checkedAtMs: now - 1000, checking: false }, 3, now)).toBe("3 hosts of 3 checked just now.");
    expect(checkedWords({ checked: 1, checkedAtMs: now - 600_000, checking: false }, 4, now)).toBe("1 host of 4 checked, oldest 10 min ago.");
    expect(progressWords(3, 12)).toBe("Checking… 3 of 12 done");
  });
});
