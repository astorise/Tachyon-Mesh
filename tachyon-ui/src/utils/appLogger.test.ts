import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

import { loggedInvoke } from "./appLogger";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

describe("loggedInvoke", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("logs a rejected invocation without recording its arguments", async () => {
    vi.mocked(invoke)
      .mockRejectedValueOnce(new Error("backend failed"))
      .mockResolvedValueOnce(undefined);

    await expect(
      loggedInvoke("save_credentials", { password: "sensitive-value" }),
    ).rejects.toThrow("backend failed");

    expect(invoke).toHaveBeenNthCalledWith(2, "log_frontend_event", {
      payload: {
        level: "error",
        source: "ipc.save_credentials",
        message: "backend failed",
      },
    });
    expect(JSON.stringify(vi.mocked(invoke).mock.calls[1])).not.toContain("sensitive-value");
  });
});
