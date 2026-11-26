import { describe, expect, it } from "vitest";
import type { Service, ServiceDetails } from "../src/ipc";
import { actionsFor, bytes, confirmText, counts, cutsAccess, facts, filterServices, serviceState, shortName, startupLabel } from "../src/server-model";

const svc = (name: string, active: string, sub: string, startup = "enabled", description = "", load = "loaded"): Service => ({ name, description, load, active, sub, startup });

const list = [
  svc("nginx.service", "active", "running", "enabled", "A high performance web server"),
  svc("cron.service", "active", "running", "enabled", "Regular background program processing daemon"),
  svc("backup.service", "failed", "failed", "disabled", "Nightly backup"),
  svc("postgresql.service", "inactive", "dead", "disabled", ""),
  svc("setup.service", "active", "exited", "static", "One-off setup"),
  svc("boot.service", "activating", "start", "enabled"),
];

describe("service state", () => {
  it("names each state in words", () => {
    expect(serviceState(list[0])).toEqual({ kind: "running", label: "Running" });
    expect(serviceState(list[2])).toEqual({ kind: "failed", label: "Failed" });
    expect(serviceState(list[3])).toEqual({ kind: "stopped", label: "Stopped" });
    expect(serviceState(list[4])).toEqual({ kind: "running", label: "Done (ran once)" });
    expect(serviceState(list[5])).toEqual({ kind: "busy", label: "Starting" });
    expect(serviceState(svc("x.service", "inactive", "dead", "", "", "not-found")).label).toBe("Not found");
    expect(serviceState(svc("x.service", "inactive", "dead", "masked", "", "masked")).label).toBe("Masked");
    // Installed but masked, so systemd never loaded it.
    expect(serviceState(svc("x.service", "inactive", "dead", "masked", "", "")).label).toBe("Masked");
  });

  it("says when it starts", () => {
    expect(startupLabel("enabled")).toBe("Starts at boot");
    expect(startupLabel("disabled")).toBe("Not at boot");
    expect(startupLabel("static")).toBe("Started by others");
    expect(startupLabel("")).toBe("");
    expect(startupLabel("weird")).toBe("Weird");
  });
});

describe("the list", () => {
  it("counts and filters by state", () => {
    expect(counts(list)).toEqual({ all: 6, running: 4, failed: 1, stopped: 1 });
    expect(filterServices(list, "", "failed").map((s) => s.name)).toEqual(["backup.service"]);
    expect(filterServices(list, "", "stopped").map((s) => s.name)).toEqual(["postgresql.service"]);
  });

  it("matches every word in the name or description", () => {
    expect(filterServices(list, "web server", "all").map((s) => s.name)).toEqual(["nginx.service"]);
    expect(filterServices(list, "  CRON ", "all").map((s) => s.name)).toEqual(["cron.service"]);
    expect(filterServices(list, "web cron", "all")).toEqual([]);
    expect(filterServices(list, "nginx", "stopped")).toEqual([]);
  });
});

describe("changes", () => {
  const offered = (s: Service) => actionsFor(s).filter((a) => a.available).map((a) => a.action);

  it("offers only what makes sense now", () => {
    expect(offered(list[0])).toEqual(["restart", "reload", "stop", "disable"]);
    expect(offered(list[2])).toEqual(["start", "restart", "enable"]);
    expect(offered(list[3])).toEqual(["start", "enable"]);
    // A one-off that ran: no reload, as nothing is running to reload.
    expect(offered(list[4])).toEqual(["restart", "stop"]);
    expect(offered(svc("x.service", "inactive", "dead", "masked", "", "masked"))).toEqual([]);
    expect(offered(svc("x.service", "inactive", "dead", "", "", "not-found"))).toEqual([]);
  });

  it("marks changes that stop things", () => {
    const risky = actionsFor(list[0]).filter((a) => a.danger).map((a) => a.action);
    expect(risky).toEqual(["restart", "stop", "disable"]);
  });

  it("asks in plain words and warns about the login service", () => {
    const q = confirmText("restart", "nginx.service", "web1");
    expect(q.title).toBe("Restart nginx?");
    expect(q.what).toContain("web1");
    expect(q.warning).toBeNull();
    expect(confirmText("stop", "ssh.service", "web1").warning).toContain("lose access");
    expect(confirmText("disable", "sshd.service", "web1").warning).not.toBeNull();
    expect(confirmText("start", "sshd.service", "web1").warning).toBeNull();
    expect(cutsAccess("ufw.service")).toBe(true);
    expect(cutsAccess("sshguard.service")).toBe(false);
    expect(shortName("getty@tty1.service")).toBe("getty@tty1");
  });
});

describe("facts", () => {
  it("leaves out what is unknown", () => {
    const d: ServiceDetails = { ...list[0], mainPid: 812, since: "Thu 2026-11-26 10:00:00 UTC", memoryBytes: 5_452_595, tasks: 3, restarts: 0, path: "/lib/systemd/system/nginx.service" };
    expect(facts(d)).toEqual([
      ["Main process", "812"],
      ["Running since", "Thu 2026-11-26 10:00:00 UTC"],
      ["Memory", "5.2 MB"],
      ["Tasks", "3"],
      ["At boot", "Starts at boot"],
      ["Unit file", "/lib/systemd/system/nginx.service"],
    ]);
    const stopped: ServiceDetails = { ...list[3], mainPid: null, since: null, memoryBytes: null, tasks: null, restarts: 2, path: null };
    expect(facts(stopped)).toEqual([
      ["Restarts", "2"],
      ["At boot", "Not at boot"],
    ]);
    expect(bytes(512)).toBe("512 B");
    expect(bytes(20 * 1024 * 1024)).toBe("20 MB");
  });
});
