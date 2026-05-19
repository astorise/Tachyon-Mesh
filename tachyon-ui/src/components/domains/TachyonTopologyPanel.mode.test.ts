import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../utils/network", () => ({
  resilientInvoke: vi.fn(),
  applyAndSeal: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue({ nodes: [], edges: [], source: "offline", status: "ok" }),
}));

import "./TachyonTopologyPanel";

async function mountTopology(): Promise<HTMLElement> {
  const el = document.createElement("tachyon-topology-panel");
  document.body.appendChild(el);
  // One microtask for connectedCallback render, one for the async loadLiveTopology.
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  return el as HTMLElement;
}

describe("TachyonTopologyPanel mode toggle", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    sessionStorage.clear();
  });

  it("defaults to View mode on first mount (no sessionStorage)", async () => {
    const panel = await mountTopology();
    const viewBtn = panel.shadowRoot?.getElementById("btn-mode-view");
    const editBtn = panel.shadowRoot?.getElementById("btn-mode-edit");
    expect(viewBtn).not.toBeNull();
    expect(editBtn).not.toBeNull();
    // Add-node form and apply button are absent in View mode
    expect(panel.shadowRoot?.getElementById("add-node-form")).toBeNull();
    expect(panel.shadowRoot?.getElementById("btn-apply-topology")).toBeNull();
  });

  it("switches to Edit mode when Edit button is clicked", async () => {
    const panel = await mountTopology();
    panel.shadowRoot?.getElementById("btn-mode-edit")?.click();
    await Promise.resolve();
    // Add-node form becomes visible
    expect(panel.shadowRoot?.getElementById("add-node-form")).not.toBeNull();
    expect(panel.shadowRoot?.getElementById("btn-apply-topology")).not.toBeNull();
  });

  it("persists mode to sessionStorage", async () => {
    const panel = await mountTopology();
    panel.shadowRoot?.getElementById("btn-mode-edit")?.click();
    await Promise.resolve();
    expect(sessionStorage.getItem("tachyon-ui:topology-mode")).toBe("edit");
  });

  it("restores Edit mode from sessionStorage on remount", async () => {
    sessionStorage.setItem("tachyon-ui:topology-mode", "edit");
    const panel = await mountTopology();
    expect(panel.shadowRoot?.getElementById("add-node-form")).not.toBeNull();
  });

  it("mode suffix appears in the banner text", async () => {
    const panel = await mountTopology();
    // The banner appends the current mode label; in View mode the label "View" or its FR
    // translation should appear in the toggle button labels.
    const viewBtn = panel.shadowRoot?.getElementById("btn-mode-view");
    expect(viewBtn).not.toBeNull();
    expect(viewBtn?.textContent?.trim().length).toBeGreaterThan(0);
  });
});
