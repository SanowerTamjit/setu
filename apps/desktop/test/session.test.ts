import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => null) }));
const { connectionOf, describe: describeConn, dialogValues } = await import("../src/session");

describe("saved connections never hold secrets", () => {
  it("drops the password", () => {
    const c = connectionOf({
      host: " db.example ",
      port: 2222,
      user: "ops",
      auth: { kind: "password", password: "hunter2", save: true },
      jump: "bastion",
    } as never);
    expect(JSON.stringify(c)).not.toContain("hunter2");
    expect(c).toEqual({ host: "db.example", port: 2222, user: "ops", auth: "password", jump: "bastion" });
  });

  it("drops a key's passphrase but keeps its path", () => {
    const c = connectionOf({
      host: "web",
      user: "me",
      auth: { kind: "keyFile", path: "~/.ssh/id_ed25519", passphrase: "pp-secret", save: false },
    } as never);
    expect(JSON.stringify(c)).not.toContain("pp-secret");
    expect(c.keyPath).toBe("~/.ssh/id_ed25519");
    expect(c.port).toBe(22);
  });

  it("round-trips into the connect dialog", () => {
    const c = { host: "h", port: 22, user: "u", auth: "agent" as const };
    expect(dialogValues(c)).toMatchObject({ host: "h", port: "22", user: "u", authKind: "agent", jump: "" });
    expect(describeConn(c)).toBe("u@h");
    expect(describeConn({ ...c, port: 2200 })).toBe("u@h:2200");
  });
});
