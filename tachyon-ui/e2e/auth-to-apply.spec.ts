import { test, expect, Page } from "@playwright/test";

// ---------------------------------------------------------------------------
// Network route mocks
// In the Tauri app the frontend calls Tauri IPC commands (not HTTP).
// For routes that *do* go through HTTP (remote cluster endpoints forwarded
// by the Rust backend), we intercept here. For Tauri-specific IPC we use
// addInitScript to stub window.__TAURI_INTERNALS__.
// ---------------------------------------------------------------------------

async function installTauriMocks(page: Page): Promise<void> {
  await page.addInitScript(() => {
    localStorage.setItem("tachyon_tour_completed", "true");
    // Stub the Tauri invoke bridge so calls resolve predictably in tests.
    const responses: Record<string, unknown> = {
      get_hardware_status: { totalRamMb: 8192, availableRamMb: 4096, accelerators: ["cpu"], gpus: [] },
      get_cluster_hardware_summary: { source: "mock", enrolledCount: 0, onlineCount: 0, staleCount: 0, totalRamMb: 0, gpuCount: 0 },
      get_mesh_graph: { source: "mock", status: "ready", routes: [], batchTargets: [] },
      get_topology_graph: { source: "mock", status: "ready", nodes: [], edges: [] },
      list_enrolled_nodes: [],
      list_registered_systems: [],
      list_deployed_systems: [],
      get_metrics: { source: "mock", errorRate: 0, p50LatencyMs: 1, p99LatencyMs: 2, queueDepth: 0, vramUtilizationPct: 0, ramOffloadActive: false },
      get_resources: [],
      load_credentials: null,
      load_custom_ca: null,
    };
    // Patch the Tauri internal invoke hook
    let callbackId = 0;
    const callbacks = new Map<number, unknown>();
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {
      invoke: (cmd: string) => {
        if (cmd === "plugin:event|listen") return Promise.resolve(callbackId++);
        if (cmd === "plugin:event|unlisten") return Promise.resolve(null);
        if (cmd in responses) return Promise.resolve(responses[cmd]);
        return Promise.reject(new Error(`[mock] unhandled command: ${cmd}`));
      },
      transformCallback: (callback: unknown) => {
        const id = callbackId++;
        callbacks.set(id, callback);
        return id;
      },
      unregisterCallback: (id: number) => {
        callbacks.delete(id);
      },
    };
    (window as unknown as Record<string, unknown>).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener: () => undefined,
    };
  });
}

async function installNetworkMocks(page: Page, applyDelayMs = 0): Promise<void> {
  // Mock the Tauri HTTP plugin routes if the app uses them for cluster calls.
  await page.route("**/auth/login/stage", async (route) => {
    await route.fulfill({ json: { sessionId: "mock-session-123", requiresMfa: true, username: "e2e-operator", endpoint: "http://localhost:8080" } });
  });
  await page.route("**/auth/login/finalize", async (route) => {
    await route.fulfill({ json: { username: "e2e-operator", endpoint: "http://localhost:8080", requiresMfa: false } });
  });
  await page.route("**/admin/manifest/bundle", async (route) => {
    if (applyDelayMs > 0) {
      await new Promise((r) => setTimeout(r, applyDelayMs));
    }
    await route.fulfill({ json: { success: true, message: "Bundle applied.", configVersion: 2, conflicts: [], requiresResolution: false } });
  });
  await page.route("**/admin/manifest", async (route) => {
    await route.fulfill({ json: { success: true, message: "Manifest applied.", configVersion: 2 } });
  });
}

// ---------------------------------------------------------------------------

test.describe("Critical Path: Login → topology → seal manifest", () => {
  test("auth step renders credentials form inside shadow DOM", async ({ page }) => {
    await installTauriMocks(page);
    await page.goto("/");

    const credStep = page.locator("tachyon-iam").locator("auth-step-credentials");
    await expect(credStep).toBeAttached();

    await expect(credStep.locator("#cred-url")).toBeVisible();
    await expect(credStep.locator("#cred-username")).toBeVisible();
    await expect(credStep.locator("#cred-password")).toBeVisible();
  });

  test("IAM panel has correct ARIA dialog role", async ({ page }) => {
    await installTauriMocks(page);
    await page.goto("/");

    const panel = page.locator("tachyon-iam").locator("#iam-panel");
    await expect(panel).toHaveAttribute("role", "dialog");
    await expect(panel).toHaveAttribute("aria-modal", "true");
    await expect(panel).toHaveAttribute("aria-labelledby", "iam-dialog-title");
  });

  test("shell nav renders after authentication event", async ({ page }) => {
    await installTauriMocks(page);
    await page.goto("/");

    await page.evaluate(() => {
      document.dispatchEvent(
        new CustomEvent("iam:authenticated", {
          bubbles: true,
          detail: { user: "e2e-operator", role: "admin", token: "test-token" },
        }),
      );
    });

    const shell = page.locator("tachyon-app-shell");
    await expect(shell).not.toHaveClass(/hidden/, { timeout: 3_000 });
    await expect(shell.locator("tachyon-app-shell-nav")).toBeAttached();
  });

  test("seal button toggles on config:staged", async ({ page }) => {
    await installTauriMocks(page);
    await page.goto("/");

    await page.evaluate(() => {
      document.dispatchEvent(
        new CustomEvent("iam:authenticated", { bubbles: true, detail: { user: "e2e-operator", role: "admin", token: "test-token" } }),
      );
    });

    const shell = page.locator("tachyon-app-shell");
    await expect(shell).not.toHaveClass(/hidden/, { timeout: 3_000 });

    const sealBtn = shell.locator("#btn-seal-apply");
    await expect(sealBtn).toHaveClass(/hidden/);

    await page.evaluate(() => window.dispatchEvent(new CustomEvent("config:staged")));
    await expect(sealBtn).not.toHaveClass(/hidden/);
  });

  test("global loader is aria-live and shows sr-only text while applying", async ({ page }) => {
    await installTauriMocks(page);
    // Apply mock with 1.5 s delay so we can assert the loader is visible.
    await installNetworkMocks(page, 1500);
    await page.goto("/");

    // Authenticate
    await page.evaluate(() => {
      document.dispatchEvent(
        new CustomEvent("iam:authenticated", { bubbles: true, detail: { user: "e2e-operator", role: "admin", token: "test-token" } }),
      );
    });
    const shell = page.locator("tachyon-app-shell");
    await expect(shell).not.toHaveClass(/hidden/, { timeout: 3_000 });

    // Stage a config change so the seal button appears
    await page.evaluate(() => window.dispatchEvent(new CustomEvent("config:staged")));
    const sealBtn = shell.locator("#btn-seal-apply");
    await expect(sealBtn).not.toHaveClass(/hidden/);

    // Click seal — the loader should appear while the mock delay is active
    void sealBtn.click();

    const loader = shell.locator("#global-apply-loader");
    await expect(loader).toBeVisible({ timeout: 2_000 });
    await expect(loader).toHaveAttribute("aria-live", "polite");

    const srText = loader.locator(".sr-only");
    await expect(srText).toHaveText(/Applying configuration/);

    // main-content should be aria-busy during the operation
    const mainContent = shell.locator("#main-content");
    await expect(mainContent).toHaveAttribute("aria-busy", "true");
  });
});
