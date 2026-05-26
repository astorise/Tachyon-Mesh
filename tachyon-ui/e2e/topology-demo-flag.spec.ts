import { test, expect, Page } from "@playwright/test";

async function installMocksEmpty(page: Page): Promise<void> {
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
          get_cluster_features: { hasEnrolledNodes: true, hasFleet: true, hasAi: true, hasRouting: true, hasResilience: true, hasIdentity: true, hasRbac: true, hasStorage: true, hasObservability: true, hasSupplyChain: true },
          get_hardware_status: { totalRamMb: 0, availableRamMb: 0, accelerators: [], gpus: [] },
          get_resources: [],
          load_credentials: null,
          load_custom_ca: null,
        };
        if (cmd in responses) return Promise.resolve(responses[cmd]);
        return Promise.reject(new Error(`[mock] unhandled: ${cmd}`));
      },
      transformCallback: () => id++,
      unregisterCallback: () => undefined,
    };
    (window as unknown as Record<string, unknown>).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener: () => undefined,
    };
  });
}

async function authenticate(page: Page): Promise<void> {
  await page.evaluate(() => {
    window.dispatchEvent(new CustomEvent("connection:connected"));
    document.dispatchEvent(
      new CustomEvent("iam:authenticated", {
        bubbles: true,
        detail: { user: "e2e-operator", role: "admin", token: "test-token" },
      }),
    );
  });
  await page.waitForTimeout(200);
}

test.describe("Topology demo flag (?demo=1)", () => {
  test("?demo=1 renders sample nodes and does not show the empty-state CTA", async ({ page }) => {
    await installMocksEmpty(page);
    // Load with the demo flag; backend still returns an empty graph.
    await page.goto("/?demo=1");
    await authenticate(page);

    await page.evaluate(() => { window.location.hash = "topology"; });
    await page.waitForSelector("tachyon-topology-panel", { timeout: 10000 });

    const panel = page.locator("tachyon-topology-panel");

    // Give the panel time to load topology data and re-render.
    await page.waitForTimeout(800);

    // With ?demo=1 the DEMO_NODES are loaded → at least one [data-node-id] button.
    const nodeCards = panel.locator("[data-node-id]");
    const count = await nodeCards.count();
    expect(count).toBeGreaterThan(0);

    // The empty-state CTA (#btn-goto-nodes) must NOT be shown when demo data is present.
    await expect(panel.locator("#btn-goto-nodes")).toHaveCount(0);
  });

  test("without ?demo=1 shows the empty-state CTA (control)", async ({ page }) => {
    await installMocksEmpty(page);
    await page.goto("/");
    await authenticate(page);

    await page.evaluate(() => { window.location.hash = "topology"; });
    await page.waitForSelector("tachyon-topology-panel", { timeout: 10000 });

    const panel = page.locator("tachyon-topology-panel");
    await page.waitForTimeout(800);

    // Without ?demo=1 and empty backend → empty state, no node cards.
    await expect(panel.locator("#btn-goto-nodes")).toBeVisible({ timeout: 3000 });
    await expect(panel.locator("[data-node-id]")).toHaveCount(0);
  });
});
