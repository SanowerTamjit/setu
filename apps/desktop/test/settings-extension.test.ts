import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a), Channel: class {} }));

const { badgeText, loadPanel } = await import("../src/settings-extension");

describe("extension settings page", () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it("is absent in the open-source build", async () => {
    invoke.mockResolvedValue(null);
    expect(await loadPanel()).toBeNull();
    expect(invoke).toHaveBeenCalledWith("extension_panel");
  });

  it("treats a failure to load as no page", async () => {
    invoke.mockImplementation(() => Promise.reject(new Error("boom")));
    expect(await loadPanel()).toBeNull();
  });

  it("passes the page through", async () => {
    const view = { title: "Organization", sections: [{ title: "Licence", status: "ok", rows: [["Customer", "Example Bank"]] }] };
    invoke.mockResolvedValue(view);
    expect(await loadPanel()).toEqual(view);
  });

  it("labels statuses", () => {
    expect(badgeText("ok")).toBe("OK");
    expect(badgeText("error")).toBe("Problem");
    expect(badgeText(undefined)).toBeNull();
  });
});
