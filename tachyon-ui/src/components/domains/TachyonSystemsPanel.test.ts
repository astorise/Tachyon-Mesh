import { beforeEach, describe, expect, it, vi } from "vitest";

import "./TachyonSystemsPanel";
import { resilientInvoke } from "../../utils/network";

vi.mock("../../utils/network", () => ({
  resilientInvoke: vi.fn(),
}));

const registered = Array.from({ length: 35 }, (_, index) => ({
  slug: `system-${index}`,
  crateName: `system-faas-${index}`,
  version: "1.1.0-alpha",
  description: `System ${index}`,
}));

const deployed = [
  {
    slug: "system-0",
    catalogVersion: "1.1.0-alpha",
    status: "deployed",
    hostNodeCount: 2,
    hasDrift: false,
    nodes: [
      { nodeId: "node-a", version: "1.1.0-alpha" },
      { nodeId: "node-b", version: "1.1.0-alpha" },
    ],
  },
  {
    slug: "system-1",
    catalogVersion: "1.1.0-alpha",
    status: "version-drift",
    hostNodeCount: 1,
    hasDrift: true,
    nodes: [{ nodeId: "node-c", version: "1.0.0" }],
  },
];

describe("TachyonSystemsPanel", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    vi.mocked(resilientInvoke).mockReset();
  });

  it("renders 35 catalog entries with mixed deployment states", async () => {
    vi.mocked(resilientInvoke).mockImplementation(async (command: string) => {
      if (command === "list_registered_systems") return registered;
      if (command === "list_deployed_systems") return deployed;
      if (command === "get_staged_system_routes") return [];
      return null;
    });

    const panel = document.createElement("tachyon-systems-panel");
    document.body.appendChild(panel);
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();

    expect(panel.shadowRoot?.textContent).toContain("35");
    expect(panel.shadowRoot?.textContent).toContain("catalog entries");
    expect(panel.shadowRoot?.textContent).toContain("version-drift");
    expect(panel.shadowRoot?.textContent).toContain("not-deployed");
  });

  it("expands rows and exposes no mutating controls", async () => {
    vi.mocked(resilientInvoke).mockImplementation(async (command: string) => {
      if (command === "list_registered_systems") return registered;
      if (command === "list_deployed_systems") return deployed;
      if (command === "get_staged_system_routes") return [];
      return null;
    });

    const panel = document.createElement("tachyon-systems-panel");
    document.body.appendChild(panel);
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    panel.shadowRoot?.querySelector<HTMLButtonElement>('[data-system-slug="system-0"]')?.click();

    expect(panel.shadowRoot?.textContent).toContain("node-a");
    // No form submission or destructive delete controls.
    expect(panel.shadowRoot?.querySelector("form")).toBeNull();
    expect(panel.shadowRoot?.querySelector('[data-action="delete"]')).toBeNull();
    // Enable/Disable are present but not delete.
    expect(panel.shadowRoot?.querySelectorAll('[data-toggle-slug]').length).toBeGreaterThan(0);
  });
});
