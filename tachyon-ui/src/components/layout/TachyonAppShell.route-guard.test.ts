import { beforeEach, describe, expect, it, vi } from "vitest";
import { isFeatureAvailable } from "../../stores/clusterFeaturesStore";

vi.mock("gsap", () => {
  const chainable: Record<string, unknown> = {};
  chainable["fromTo"] = vi.fn(() => chainable);
  chainable["to"] = vi.fn(() => chainable);
  chainable["then"] = vi.fn((resolve?: () => void) => Promise.resolve().then(resolve));
  return {
    default: {
      set: vi.fn(),
      to: vi.fn(),
      fromTo: vi.fn(),
      timeline: vi.fn(() => chainable),
    },
  };
});

vi.mock("../../stores/clusterFeaturesStore", () => ({
  clusterFeaturesStore: {
    subscribe: vi.fn(() => () => {}),
  },
  isFeatureAvailable: vi.fn(),
}));

vi.mock("../../utils/network", () => ({
  resilientInvoke: vi.fn().mockResolvedValue(null),
  applyAndSeal: vi.fn().mockResolvedValue(null),
}));

import "./TachyonAppShell";

function mountShell(): HTMLElement {
  const shell = document.createElement("tachyon-app-shell");
  document.body.appendChild(shell);
  return shell;
}

async function activateShell(shell: HTMLElement): Promise<void> {
  document.dispatchEvent(
    new CustomEvent("iam:authenticated", {
      detail: { user: "test-user", role: "operator", token: "tok" },
    }),
  );
  // Allow microtasks (startTransition async steps) to settle
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

describe("TachyonAppShell — route guard", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    window.location.hash = "";
    vi.mocked(isFeatureAvailable).mockReset();
    vi.mocked(clusterFeaturesStore.subscribe as ReturnType<typeof vi.fn>).mockReturnValue(() => {});
  });

  // task 7.4 — redirect to overview when gated route accessed directly
  it("redirects #ai to #overview when hasAi is false", async () => {
    vi.mocked(isFeatureAvailable).mockReturnValue(false);
    const shell = mountShell();
    await activateShell(shell);

    window.location.hash = "ai";
    window.dispatchEvent(new Event("hashchange"));

    expect(window.location.hash).toBe("#overview");
  });

  it("redirects #storage to #overview when hasStorage is false", async () => {
    vi.mocked(isFeatureAvailable).mockReturnValue(false);
    const shell = mountShell();
    await activateShell(shell);

    window.location.hash = "storage";
    window.dispatchEvent(new Event("hashchange"));

    expect(window.location.hash).toBe("#overview");
  });

  it("redirects #fleet to #overview when hasFleet is false", async () => {
    vi.mocked(isFeatureAvailable).mockReturnValue(false);
    const shell = mountShell();
    await activateShell(shell);

    window.location.hash = "fleet";
    window.dispatchEvent(new Event("hashchange"));

    expect(window.location.hash).toBe("#overview");
  });

  // task 7.4 — no redirect when feature is present
  it("does not redirect #ai when hasAi is true", async () => {
    vi.mocked(isFeatureAvailable).mockImplementation((f) => f === "hasAi");
    const shell = mountShell();
    await activateShell(shell);

    window.location.hash = "ai";
    window.dispatchEvent(new Event("hashchange"));

    expect(window.location.hash).toBe("#ai");
  });

  // task 7.5 — overview never redirected
  it("never redirects #overview regardless of feature state", async () => {
    vi.mocked(isFeatureAvailable).mockReturnValue(false);
    const shell = mountShell();
    await activateShell(shell);

    window.location.hash = "overview";
    window.dispatchEvent(new Event("hashchange"));

    expect(window.location.hash).toBe("#overview");
  });

  it("never redirects #dashboard regardless of feature state", async () => {
    vi.mocked(isFeatureAvailable).mockReturnValue(false);
    const shell = mountShell();
    await activateShell(shell);

    window.location.hash = "dashboard";
    window.dispatchEvent(new Event("hashchange"));

    expect(window.location.hash).toBe("#dashboard");
  });
});

import { clusterFeaturesStore } from "../../stores/clusterFeaturesStore";
