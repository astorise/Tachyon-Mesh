import { test, expect, Page } from "@playwright/test";

async function installMocks(page: Page): Promise<void> {
  await page.addInitScript(() => {
    localStorage.setItem("tachyon_tour_completed", "true");
    let id = 0;
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {
      invoke: (cmd: string) => {
        if (cmd === "plugin:event|listen") return Promise.resolve(id++);
        if (cmd === "plugin:event|unlisten") return Promise.resolve(null);
        const responses: Record<string, unknown> = {
          get_topology_graph: { source: "live", status: "ok", nodes: [], edges: [] },
          get_mesh_graph: { source: "mock", status: "ready", routes: [], batchTargets: [] },
          get_metrics: { source: "mock", errorRate: 0, p50LatencyMs: 1, p99LatencyMs: 2, queueDepth: 0, vramUtilizationPct: 0, ramOffloadActive: false },
          list_enrolled_nodes: [],
          list_registered_systems: [],
          list_deployed_systems: [],
          get_cluster_hardware_summary: { source: "mock", enrolledCount: 0, onlineCount: 0, staleCount: 0, totalRamMb: 0, gpuCount: 0 },
          get_hardware_status: { totalRamMb: 0, availableRamMb: 0, accelerators: [], gpus: [] },
          get_resources: [],
          load_credentials: null,
          load_custom_ca: null,
        };
        if (cmd in responses) return Promise.resolve(responses[cmd]);
        return Promise.reject(new Error(`[mock] unhandled: ${cmd}`));
      },
      transformCallback: (cb: unknown) => { const i = id++; return i; },
      unregisterCallback: () => undefined,
    };
    (window as unknown as Record<string, unknown>).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener: () => undefined,
    };
  });
}

async function authenticate(page: Page): Promise<void> {
  await page.evaluate(() => {
    document.dispatchEvent(
      new CustomEvent("iam:authenticated", {
        bubbles: true,
        detail: { user: "e2e-operator", role: "admin", token: "test-token" },
      }),
    );
  });
}

test.describe("Topology empty state (no nodes from backend)", () => {
  test("renders empty-state card and no node buttons when graph is empty", async ({ page }) => {
    await installMocks(page);
    await page.goto("/");
    await authenticate(page);

    await page.evaluate(() => { window.location.hash = "topology"; });
    await page.waitForSelector("tachyon-topology-panel", { timeout: 10000 });

    const panel = page.locator("tachyon-topology-panel");
    // Allow loadLiveTopology to complete and re-render.
    await page.waitForTimeout(800);

    // The "Go to Nodes" CTA button from the empty-state card must be visible.
    await expect(panel.locator("#btn-goto-nodes")).toBeVisible({ timeout: 5000 });

    // No node cards should exist.
    const nodeCards = panel.locator("[data-node-id]");
    await expect(nodeCards).toHaveCount(0);
  });

  test("View mode is the default — add-node form is absent", async ({ page }) => {
    await installMocks(page);
    await page.goto("/");
    await authenticate(page);

    await page.evaluate(() => { window.location.hash = "topology"; });
    await page.waitForSelector("tachyon-topology-panel", { timeout: 10000 });
    await page.waitForTimeout(800);

    const panel = page.locator("tachyon-topology-panel");
    await expect(panel.locator("#add-node-form")).toHaveCount(0);
    await expect(panel.locator("#btn-apply-topology")).toHaveCount(0);
  });
});
