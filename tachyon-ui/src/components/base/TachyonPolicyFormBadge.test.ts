import { beforeEach, describe, expect, it, vi } from "vitest";
import { applyAndSeal, resilientInvoke } from "../../utils/network";

vi.mock("../../utils/network", () => ({
  applyAndSeal: vi.fn(),
  resilientInvoke: vi.fn(),
}));

// ── Panels that MUST have the badge ──────────────────────────────────────────

import "../traffic/TachyonResiliencePanel";
import "../domains/TachyonIdentityPanel";
import "../domains/TachyonRbacPanel";
import "../domains/TachyonSupplyChainPanel";
import "../domains/TachyonFleetPanel";

// ── Panels that MUST NOT have the badge ──────────────────────────────────────

import "../domains/TachyonOverviewPanel";
import "../domains/TachyonNodesPanel";
import "../domains/TachyonSystemsPanel";
import "../domains/TachyonTopologyPanel";
import "../domains/TachyonUsersPanel";
import "../domains/TachyonWorkloadsPanel";
import "../domains/TachyonObservabilityPanel";
import "../domains/TachyonStoragePanel";
import "../domains/TachyonAIPanel";

function hasBadge(el: HTMLElement): boolean {
  const sr = el.shadowRoot;
  if (!sr) return false;
  if (sr.querySelector("tachyon-policy-form-badge")) return true;
  return sr.innerHTML.includes("tachyon-policy-form-badge");
}

async function mount(tag: string): Promise<HTMLElement> {
  const el = document.createElement(tag);
  document.body.appendChild(el);
  await Promise.resolve();
  await Promise.resolve();
  return el as HTMLElement;
}

describe("tachyon-policy-form-badge presence", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    vi.mocked(applyAndSeal).mockResolvedValue({ success: true, message: "" });
    vi.mocked(resilientInvoke).mockResolvedValue([]);
  });

  it.each([
    "tachyon-resilience-panel",
    "tachyon-identity-panel",
    "tachyon-rbac-panel",
    "tachyon-supply-chain-panel",
    "tachyon-fleet-panel",
  ])("%s has the policy-form badge", async (tag) => {
    const el = await mount(tag);
    expect(hasBadge(el)).toBe(true);
  });

  it.each([
    "tachyon-overview-panel",
    "tachyon-nodes-panel",
    "tachyon-systems-panel",
    "tachyon-topology-panel",
    "tachyon-users-panel",
    "tachyon-workloads-panel",
    "tachyon-observability-panel",
    "tachyon-storage-panel",
    "tachyon-ai-panel",
  ])("%s does NOT have the policy-form badge", async (tag) => {
    const el = await mount(tag);
    expect(hasBadge(el)).toBe(false);
  });
});
