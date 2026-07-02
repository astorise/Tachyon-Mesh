import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { resilientInvoke } from "../../utils/network";
import { setLanguage } from "../../utils/i18n";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("../../utils/network", () => ({
  resilientInvoke: vi.fn(),
}));

import "./TachyonWorkloadsPanel";

const tauriInvokeMock = vi.mocked(tauriInvoke);
const resilientInvokeMock = vi.mocked(resilientInvoke);

function mountPanel(): HTMLElement {
  const el = document.createElement("tachyon-workloads-panel");
  document.body.appendChild(el);
  return el;
}

describe("TachyonWorkloadsPanel", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    setLanguage("en");
    resilientInvokeMock.mockResolvedValue([]);
    tauriInvokeMock.mockImplementation(async (command: string) => {
      if (command === "get_manifest_config") {
        return {
          routes: [
            { path: "/api/orders", name: "orders-v1", version: "1.0.0" },
            { path: "/api/users", name: "users-v1", version: "1.0.0" },
          ],
          kv_caches: [],
        };
      }
      if (command === "apply_manifest_config") {
        return { success: true, message: "applied", configVersion: 3 };
      }
      return null;
    });
  });

  afterEach(() => {
    document.body.innerHTML = "";
    vi.clearAllMocks();
  });

  it("writes canary configuration to routes[].canary through the runtime manifest", async () => {
    const panel = mountPanel();
    await new Promise((resolve) => setTimeout(resolve, 0));
    await new Promise((resolve) => setTimeout(resolve, 0));

    const root = panel.shadowRoot;
    const strategy = root?.getElementById("strategy") as HTMLSelectElement;
    strategy.value = "canary";
    strategy.dispatchEvent(new Event("change"));

    (root?.getElementById("canary-route") as HTMLSelectElement).value = "/api/users";
    (root?.getElementById("canary-next-version") as HTMLInputElement).value = "users-v2";
    (root?.getElementById("canary-step-weight") as HTMLInputElement).value = "25";
    (root?.getElementById("canary-interval-secs") as HTMLInputElement).value = "30";
    (root?.getElementById("canary-max-error-rate") as HTMLInputElement).value = "0.02";

    root?.querySelector("form")?.dispatchEvent(new Event("submit", { cancelable: true }));
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(tauriInvokeMock).toHaveBeenCalledWith("apply_manifest_config", {
      config: expect.objectContaining({
        routes: [
          expect.objectContaining({ path: "/api/orders" }),
          expect.objectContaining({
            path: "/api/users",
            canary: {
              next_version: "users-v2",
              step_weight: 25,
              interval_secs: 30,
              max_error_rate: 0.02,
            },
          }),
        ],
      }),
    });
  });
});
