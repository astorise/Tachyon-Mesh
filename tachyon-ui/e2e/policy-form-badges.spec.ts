import { test, expect, Page } from "@playwright/test";

async function installMocks(page: Page): Promise<void> {
  await page.addInitScript(() => {
    // Prevent the guided tour overlay from firing during tests; it starts after
    // a 450ms timeout and its fixed inset-0 shadow-DOM layer intercepts clicks.
    localStorage.setItem("tachyon_tour_completed", "true");
    let id = 0;
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {
      invoke: (cmd: string) => {
        if (cmd === "plugin:event|listen") return Promise.resolve(id++);
        if (cmd === "plugin:event|unlisten") return Promise.resolve(null);
        const responses: Record<string, unknown> = {
          get_mesh_graph: { source: "mock", status: "ready", routes: [], batchTargets: [] },
          get_metrics: { source: "mock", errorRate: 0, p50LatencyMs: 1, p99LatencyMs: 2, queueDepth: 0, vramUtilizationPct: 0, ramOffloadActive: false },
          get_topology_graph: { source: "live", status: "ok", nodes: [], edges: [] },
          get_hardware_status: { totalRamMb: 0, availableRamMb: 0, accelerators: [], gpus: [] },
          list_enrolled_nodes: [],
          list_registered_systems: [],
          list_deployed_systems: [],
          get_cluster_hardware_summary: { source: "mock", enrolledCount: 0, onlineCount: 0, staleCount: 0, totalRamMb: 0, gpuCount: 0 },
          get_resources: [],
          load_credentials: null,
          load_custom_ca: null,
          fetch_canary_status: [],
          tail_logs: [],
          get_shadow_diffs: [],
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
    document.dispatchEvent(
      new CustomEvent("iam:authenticated", {
        bubbles: true,
        detail: { user: "e2e-operator", role: "admin", token: "test-token" },
      }),
    );
  });
}

async function navigateTo(page: Page, route: string): Promise<void> {
  await page.evaluate((r) => { window.location.hash = r; }, route);
  await page.waitForTimeout(600);
}

// ── Policy-form panels that MUST show the badge ─────────────────────────────

const POLICY_ROUTES: Array<{ route: string; panelTag: string }> = [
  { route: "resilience",    panelTag: "tachyon-resilience-panel" },
  { route: "identity-config", panelTag: "tachyon-identity-panel" },
  { route: "rbac",          panelTag: "tachyon-rbac-panel" },
  { route: "supply-chain",  panelTag: "tachyon-supply-chain-panel" },
  { route: "fleet",         panelTag: "tachyon-fleet-panel" },
];

for (const { route, panelTag } of POLICY_ROUTES) {
  test(`${route} panel shows "Policy form" badge`, async ({ page }) => {
    await installMocks(page);
    await page.goto("/");
    await authenticate(page);
    await navigateTo(page, route);

    await page.waitForSelector(panelTag, { timeout: 10000 });
    await page.waitForTimeout(400);
    const panel = page.locator(panelTag);

    // The badge must be present in the shadow DOM (Playwright pierces shadow roots).
    const badge = panel.locator("tachyon-policy-form-badge");
    await expect(badge).toHaveCount(1, { timeout: 5000 });
  });
}

// ── State panels that MUST NOT show the badge ────────────────────────────────

test("topology panel does NOT show the policy-form badge", async ({ page }) => {
  await installMocks(page);
  await page.goto("/");
  await authenticate(page);
  await navigateTo(page, "topology");

  await page.waitForSelector("tachyon-topology-panel", { timeout: 10000 });
  await page.waitForTimeout(600);
  const panel = page.locator("tachyon-topology-panel");
  await expect(panel.locator("tachyon-policy-form-badge")).toHaveCount(0);
});

// ── Topology View/Edit mode toggle (covers manual task 5.4) ─────────────────

test("topology defaults to View mode — no add-node form, no Apply button", async ({ page }) => {
  await installMocks(page);
  await page.goto("/");
  await authenticate(page);
  await navigateTo(page, "topology");

  await page.waitForSelector("tachyon-topology-panel", { timeout: 10000 });
  await page.waitForTimeout(800);
  const panel = page.locator("tachyon-topology-panel");

  // View/Edit toggle buttons present.
  await expect(panel.locator("#btn-mode-view")).toBeVisible({ timeout: 5000 });
  await expect(panel.locator("#btn-mode-edit")).toBeVisible();

  // Edit affordances hidden in View mode.
  await expect(panel.locator("#add-node-form")).toHaveCount(0);
  await expect(panel.locator("#btn-apply-topology")).toHaveCount(0);
});

test("topology Edit mode shows add-node form and Apply button", async ({ page }) => {
  await installMocks(page);
  await page.goto("/");
  await authenticate(page);
  await navigateTo(page, "topology");

  await page.waitForSelector("tachyon-topology-panel", { timeout: 10000 });
  await page.waitForTimeout(800);
  const panel = page.locator("tachyon-topology-panel");

  await panel.locator("#btn-mode-edit").click();
  await page.waitForTimeout(400);

  await expect(panel.locator("#add-node-form")).toBeVisible({ timeout: 5000 });
  await expect(panel.locator("#btn-apply-topology")).toBeVisible();
});

test("topology mode persists to sessionStorage on toggle", async ({ page }) => {
  await installMocks(page);
  await page.goto("/");
  await authenticate(page);
  await navigateTo(page, "topology");

  await page.waitForSelector("tachyon-topology-panel", { timeout: 10000 });
  await page.waitForTimeout(800);
  await page.locator("tachyon-topology-panel").locator("#btn-mode-edit").click();
  await page.waitForTimeout(400);

  const stored = await page.evaluate(() =>
    sessionStorage.getItem("tachyon-ui:topology-mode"),
  );
  expect(stored).toBe("edit");
});
