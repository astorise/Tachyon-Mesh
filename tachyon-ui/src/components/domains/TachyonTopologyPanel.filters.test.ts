import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import "./TachyonTopologyPanel";

const liveGraph = {
  nodes: [
    {
      id: "route:/system/logger",
      nodeType: "system-faas",
      label: "system-faas-logger",
      x: 24,
      y: 24,
      data: { component: "system-faas-logger", tags: "observability,logs" },
    },
    {
      id: "resource:billing",
      nodeType: "external-resource",
      label: "Billing API",
      x: 320,
      y: 24,
      data: { targetUrl: "https://billing.example.com" },
    },
  ],
  edges: [
    { id: "e1", from: "route:/system/logger", to: "resource:billing" },
  ],
  source: "workspace://test",
  status: "ok",
};

async function mountTopology(): Promise<HTMLElement> {
  const element = document.createElement("tachyon-topology-panel");
  document.body.appendChild(element);
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  return element as HTMLElement;
}

function canvasNodeCount(panel: HTMLElement): number {
  const canvas = panel.shadowRoot?.querySelector("tachyon-topology-canvas");
  return canvas?.shadowRoot?.querySelectorAll("[data-node-id]").length ?? 0;
}

describe("TachyonTopologyPanel filters", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    sessionStorage.clear();
    vi.mocked(invoke).mockReset();
    vi.mocked(invoke).mockResolvedValue(liveGraph);
  });

  it("keeps matching canvas nodes visible when the first text filter is typed", async () => {
    const panel = await mountTopology();
    expect(canvasNodeCount(panel)).toBe(2);

    const input = panel.shadowRoot?.getElementById("filter-text") as HTMLInputElement | null;
    expect(input).not.toBeNull();
    input!.value = "logger";
    input!.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await Promise.resolve();

    expect(canvasNodeCount(panel)).toBe(1);
    expect(panel.shadowRoot?.getElementById("filter-badge")?.style.display).toBe("");
  });
});
