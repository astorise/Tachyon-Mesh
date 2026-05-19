import { test, expect, Page } from "@playwright/test";

async function installMocks(page: Page): Promise<void> {
  await page.addInitScript(() => {
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
  await page.waitForTimeout(300);
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

    await page.waitForSelector(panelTag);
    const panel = page.locator(panelTag);

    // The badge must be present in the DOM (in shadow root, pierced by Playwright).
    const badge = panel.locator("tachyon-policy-form-badge");
    await expect(badge).toHaveCount(1, { timeout: 4000 });

    // The badge must render non-empty label text.
    const label = badge.locator("span");
    await expect(label).not.toBeEmpty();
  });
}

// ── State panels that MUST NOT show the badge ────────────────────────────────

test("topology panel does NOT show the policy-form badge", async ({ page }) => {
  await installMocks(page);
  await page.goto("/");
  await authenticate(page);
  await navigateTo(page, "topology");

  await page.waitForSelector("tachyon-topology-panel");
  const panel = page.locator("tachyon-topology-panel");
  await expect(panel.locator("tachyon-policy-form-badge")).toHaveCount(0);
});

// ── Topology View/Edit mode toggle (covers manual task 5.4) ─────────────────

test("topology defaults to View mode — no add-node form, no Apply button", async ({ page }) => {
  await installMocks(page);
  await page.goto("/");
  await authenticate(page);
  await navigateTo(page, "topology");

  await page.waitForSelector("tachyon-topology-panel");
  const panel = page.locator("tachyon-topology-panel");

  // View/Edit toggle buttons present.
  await expect(panel.locator("#btn-mode-view")).toBeVisible({ timeout: 4000 });
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

  await page.waitForSelector("tachyon-topology-panel");
  const panel = page.locator("tachyon-topology-panel");

  await panel.locator("#btn-mode-edit").click();
  await page.waitForTimeout(200);

  await expect(panel.locator("#add-node-form")).toBeVisible({ timeout: 3000 });
  await expect(panel.locator("#btn-apply-topology")).toBeVisible();
});

test("topology mode persists to sessionStorage on toggle", async ({ page }) => {
  await installMocks(page);
  await page.goto("/");
  await authenticate(page);
  await navigateTo(page, "topology");

  await page.waitForSelector("tachyon-topology-panel");
  await page.locator("tachyon-topology-panel").locator("#btn-mode-edit").click();
  await page.waitForTimeout(200);

  const stored = await page.evaluate(() =>
    sessionStorage.getItem("tachyon-ui:topology-mode"),
  );
  expect(stored).toBe("edit");
});
