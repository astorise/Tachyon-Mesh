import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

import "./TachyonBundleConflictModal";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

describe("TachyonBundleConflictModal", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    vi.mocked(invoke).mockReset();
  });

  it("renders conflicts and disables retry until every conflict has a resolution", () => {
    const modal = document.createElement("tachyon-bundle-conflict-modal") as HTMLElement & {
      open: (conflicts: Array<{ name: string; bundledVersion: string; clusterVersion: string }>) => void;
    };
    document.body.appendChild(modal);

    modal.open([
      { name: "system-faas-authn", bundledVersion: "1.2.0", clusterVersion: "1.1.0" },
    ]);

    const retry = modal.shadowRoot?.getElementById("btn-retry") as HTMLButtonElement | null;
    expect(modal.shadowRoot?.textContent).toContain("system-faas-authn");
    expect(retry?.disabled).toBe(true);

    modal.shadowRoot?.querySelector<HTMLButtonElement>('button[data-action="local"]')?.click();

    const enabledRetry = modal.shadowRoot?.getElementById("btn-retry") as HTMLButtonElement | null;
    expect(enabledRetry?.disabled).toBe(false);
  });

  it("retries with selected dependency resolutions and emits a success notification", async () => {
    const modal = document.createElement("tachyon-bundle-conflict-modal") as HTMLElement & {
      open: (conflicts: Array<{ name: string; bundledVersion: string; clusterVersion: string }>) => void;
    };
    const notify = vi.fn();
    window.addEventListener("app:notify", notify);
    document.body.appendChild(modal);
    vi.mocked(invoke).mockResolvedValue({
      success: true,
      message: "applied",
      configVersion: 42,
      conflicts: [],
      requiresResolution: false,
    });

    modal.open([
      { name: "system-faas-authn", bundledVersion: "1.2.0", clusterVersion: "1.1.0" },
    ]);
    modal.shadowRoot?.querySelector<HTMLButtonElement>('button[data-action="local"]')?.click();
    modal.shadowRoot?.getElementById("btn-retry")?.click();
    await Promise.resolve();
    await Promise.resolve();

    expect(invoke).toHaveBeenCalledWith("bundle_and_apply_manifest", {
      dependencies: [
        {
          name: "system-faas-authn",
          versionConstraint: "=1.2.0",
          source: "./assets/system-faas-authn.wasm",
        },
      ],
    });
    expect(notify).toHaveBeenCalledWith(
      expect.objectContaining({
        detail: expect.objectContaining({ type: "success" }),
      }),
    );
    window.removeEventListener("app:notify", notify);
  });
});
