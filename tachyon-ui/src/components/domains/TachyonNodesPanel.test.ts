import { beforeEach, describe, expect, it, vi } from "vitest";

import "./TachyonNodesPanel";
import { resilientInvoke } from "../../utils/network";

vi.mock("../../utils/network", () => ({
  resilientInvoke: vi.fn(),
}));

const nodes = [
  {
    nodeId: "node-a",
    status: "online",
    lastSeen: 10,
    capabilities: {
      totalRamMb: 32768,
      availableRamMb: 12000,
      accelerators: ["cpu", "gpu"],
      gpus: [
        {
          id: "cuda:0",
          model: "NVIDIA GPU",
          vramTotalMb: 24576,
          vramUsedMb: 1024,
          computeUtilization: 0,
        },
      ],
    },
  },
  {
    nodeId: "node-b",
    status: "stale",
    lastSeen: 5,
    capabilities: {
      totalRamMb: 16384,
      availableRamMb: 8000,
      accelerators: ["cpu"],
      gpus: [],
    },
  },
  {
    nodeId: "node-c",
    status: "awaiting-capabilities",
    lastSeen: 1,
    capabilities: {
      totalRamMb: 0,
      availableRamMb: 0,
      accelerators: [],
      gpus: [],
    },
  },
];

describe("TachyonNodesPanel", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    vi.useFakeTimers();
    vi.setSystemTime(2_000);
    vi.mocked(resilientInvoke).mockReset();
  });

  it("renders three nodes and opens GPU details", async () => {
    vi.mocked(resilientInvoke).mockImplementation(async (command: string) => {
      if (command === "list_enrolled_nodes") return nodes;
      if (command === "get_cluster_hardware_summary") {
        return {
          source: "test",
          enrolledCount: 3,
          onlineCount: 1,
          staleCount: 1,
          totalRamMb: 49152,
          gpuCount: 1,
        };
      }
      return null;
    });
    const panel = document.createElement("tachyon-nodes-panel");
    document.body.appendChild(panel);
    await Promise.resolve();
    await Promise.resolve();

    expect(panel.shadowRoot?.textContent).toContain("node-a");
    expect(panel.shadowRoot?.textContent).toContain("node-b");
    panel.shadowRoot?.querySelector<HTMLButtonElement>('[data-node-id="node-a"]')?.click();
    expect(panel.shadowRoot?.textContent).toContain("NVIDIA GPU");
  });

  it("renders the empty state", async () => {
    vi.mocked(resilientInvoke).mockImplementation(async (command: string) => {
      if (command === "list_enrolled_nodes") return [];
      if (command === "get_cluster_hardware_summary") {
        return {
          source: "offline",
          enrolledCount: 0,
          onlineCount: 0,
          staleCount: 0,
          totalRamMb: 0,
          gpuCount: 0,
        };
      }
      return null;
    });
    const panel = document.createElement("tachyon-nodes-panel");
    document.body.appendChild(panel);
    await Promise.resolve();
    await Promise.resolve();

    expect(panel.shadowRoot?.textContent).toContain("No enrolled nodes yet");
  });

  it("debounces refresh clicks", async () => {
    vi.mocked(resilientInvoke).mockImplementation(async (command: string) => {
      if (command === "list_enrolled_nodes") return nodes;
      if (command === "get_cluster_hardware_summary") {
        return {
          source: "test",
          enrolledCount: 3,
          onlineCount: 1,
          staleCount: 1,
          totalRamMb: 49152,
          gpuCount: 1,
        };
      }
      return null;
    });
    const panel = document.createElement("tachyon-nodes-panel");
    document.body.appendChild(panel);
    await Promise.resolve();
    await Promise.resolve();
    vi.mocked(resilientInvoke).mockClear();

    const refresh = panel.shadowRoot?.getElementById("nodes-refresh") as HTMLButtonElement;
    refresh.click();
    refresh.click();
    await Promise.resolve();
    await Promise.resolve();

    expect(vi.mocked(resilientInvoke).mock.calls.filter(([name]) => name === "list_enrolled_nodes")).toHaveLength(1);
  });
});
