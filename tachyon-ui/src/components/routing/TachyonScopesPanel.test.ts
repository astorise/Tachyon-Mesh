import { beforeEach, describe, expect, it, vi } from "vitest";
import * as scopesController from "../../controllers/scopesController";

vi.mock("../../controllers/scopesController", () => ({
  readRouteScopes: vi.fn(),
  writeRouteScopes: vi.fn(),
  readScopeMetrics: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import "./TachyonScopesPanel";

function makePanel(routePath = "/api/fn"): HTMLElement {
  const el = document.createElement("tachyon-scopes-panel") as HTMLElement;
  el.setAttribute("route-path", routePath);
  return el;
}

async function attachAndWait(el: HTMLElement): Promise<void> {
  document.body.appendChild(el);
  await new Promise((r) => setTimeout(r, 0));
}

describe("TachyonScopesPanel", () => {
  beforeEach(() => {
    vi.mocked(scopesController.writeRouteScopes).mockResolvedValue({
      success: true,
      message: "",
      configVersion: 1,
    });
    vi.mocked(scopesController.readScopeMetrics).mockResolvedValue({
      scopeDenialTotal: 0,
    });
  });

  it("shows ALLOW ALL badge when scopes are absent", async () => {
    vi.mocked(scopesController.readRouteScopes).mockResolvedValue({
      scopes: {},
      allowAll: true,
    });

    const panel = makePanel();
    await attachAndWait(panel);
    // Flush the async load
    await new Promise((r) => setTimeout(r, 10));

    const sr = panel.shadowRoot!;
    const badge = sr.querySelector(".bg-amber-500\\/20");
    expect(badge).not.toBeNull();
    expect(badge?.textContent?.trim().toUpperCase()).toContain("ALLOW ALL");

    panel.remove();
  });

  it("does not show ALLOW ALL badge when scope is explicit", async () => {
    vi.mocked(scopesController.readRouteScopes).mockResolvedValue({
      scopes: { secrets: ["db/*"] },
      allowAll: false,
    });

    const panel = makePanel();
    await attachAndWait(panel);
    await new Promise((r) => setTimeout(r, 10));

    const sr = panel.shadowRoot!;
    const badge = sr.querySelector(".bg-amber-500\\/20");
    expect(badge).toBeNull();

    panel.remove();
  });

  it("blocks adding an invalid glob pattern", async () => {
    vi.mocked(scopesController.readRouteScopes).mockResolvedValue({
      scopes: {},
      allowAll: true,
    });

    const panel = makePanel();
    await attachAndWait(panel);
    await new Promise((r) => setTimeout(r, 10));

    const sr = panel.shadowRoot!;
    const secretsInput = sr.querySelector<HTMLInputElement>(
      ".pattern-input[data-category='secrets']",
    );
    const addBtn = sr.querySelector<HTMLButtonElement>(
      ".btn-add-pattern[data-category='secrets']",
    );

    if (secretsInput && addBtn) {
      secretsInput.value = "{unbalanced";
      addBtn.click();
      await new Promise((r) => setTimeout(r, 10));

      const errorEl = sr.querySelector(".text-red-400");
      expect(errorEl).not.toBeNull();
      expect(scopesController.writeRouteScopes).not.toHaveBeenCalled();
    }

    panel.remove();
  });

  it("blocks routing tuple without ' -> '", async () => {
    vi.mocked(scopesController.readRouteScopes).mockResolvedValue({
      scopes: {},
      allowAll: true,
    });

    const panel = makePanel();
    await attachAndWait(panel);
    await new Promise((r) => setTimeout(r, 10));

    const sr = panel.shadowRoot!;
    const routingInput = sr.querySelector<HTMLInputElement>(
      ".pattern-input[data-category='routing']",
    );
    const addBtn = sr.querySelector<HTMLButtonElement>(
      ".btn-add-pattern[data-category='routing']",
    );

    if (routingInput && addBtn) {
      routingInput.value = "service-a-service-b";
      addBtn.click();
      await new Promise((r) => setTimeout(r, 10));

      const errorEl = sr.querySelector(".text-red-400");
      expect(errorEl).not.toBeNull();
      expect(errorEl?.textContent).toContain("->");
    }

    panel.remove();
  });
});
