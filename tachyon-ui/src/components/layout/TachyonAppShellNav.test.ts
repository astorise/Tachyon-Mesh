import { beforeEach, describe, expect, it, vi } from "vitest";
import { clusterFeaturesStore, isFeatureAvailable } from "../../stores/clusterFeaturesStore";

import "./TachyonAppShellNav";

vi.mock("../../stores/clusterFeaturesStore", () => ({
  clusterFeaturesStore: {
    subscribe: vi.fn(() => () => {}),
  },
  isFeatureAvailable: vi.fn(),
}));

function mountNav(): HTMLElement {
  const nav = document.createElement("tachyon-app-shell-nav");
  document.body.appendChild(nav);
  return nav;
}

function navRoutes(nav: HTMLElement): string[] {
  return Array.from(nav.shadowRoot?.querySelectorAll<HTMLButtonElement>("[data-route]") ?? []).map(
    (b) => b.dataset.route ?? "",
  );
}

describe("TachyonAppShellNav — cluster feature filtering", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    vi.mocked(isFeatureAvailable).mockReset();
    vi.mocked(clusterFeaturesStore.subscribe as ReturnType<typeof vi.fn>).mockReset();
    vi.mocked(clusterFeaturesStore.subscribe as ReturnType<typeof vi.fn>).mockReturnValue(() => {});
  });

  // task 7.5 — always-visible routes present regardless of features
  it("always renders overview regardless of cluster features", () => {
    vi.mocked(isFeatureAvailable).mockReturnValue(false);
    expect(navRoutes(mountNav())).toContain("overview");
  });

  it("always renders dashboard regardless of cluster features", () => {
    vi.mocked(isFeatureAvailable).mockReturnValue(false);
    expect(navRoutes(mountNav())).toContain("dashboard");
  });

  // task 7.3 — gated panels absent when features missing
  it("hides AI Orchestration when hasAi is false", () => {
    vi.mocked(isFeatureAvailable).mockReturnValue(false);
    expect(navRoutes(mountNav())).not.toContain("ai");
  });

  it("hides Storage when hasStorage is false", () => {
    vi.mocked(isFeatureAvailable).mockReturnValue(false);
    expect(navRoutes(mountNav())).not.toContain("storage");
  });

  it("hides Fleet when hasFleet is false", () => {
    vi.mocked(isFeatureAvailable).mockReturnValue(false);
    expect(navRoutes(mountNav())).not.toContain("fleet");
  });

  it("hides Users, RBAC and Identity when identity features are false", () => {
    vi.mocked(isFeatureAvailable).mockReturnValue(false);
    const routes = navRoutes(mountNav());
    expect(routes).not.toContain("users");
    expect(routes).not.toContain("rbac");
    expect(routes).not.toContain("identity-config");
  });

  it("hides Topology, Nodes and Hardware when hasEnrolledNodes is false", () => {
    vi.mocked(isFeatureAvailable).mockReturnValue(false);
    const routes = navRoutes(mountNav());
    expect(routes).not.toContain("topology");
    expect(routes).not.toContain("nodes");
    expect(routes).not.toContain("hardware");
  });

  it("hides Supply Chain when hasSupplyChain is false", () => {
    vi.mocked(isFeatureAvailable).mockReturnValue(false);
    expect(navRoutes(mountNav())).not.toContain("supply-chain");
  });

  // task 7.3 — gated panels appear when feature present
  it("shows AI Orchestration when hasAi is true", () => {
    vi.mocked(isFeatureAvailable).mockImplementation((f) => f === "hasAi");
    expect(navRoutes(mountNav())).toContain("ai");
  });

  it("shows Storage when hasStorage is true", () => {
    vi.mocked(isFeatureAvailable).mockImplementation((f) => f === "hasStorage");
    expect(navRoutes(mountNav())).toContain("storage");
  });

  it("shows all gated panels when all features are present", () => {
    vi.mocked(isFeatureAvailable).mockReturnValue(true);
    const routes = navRoutes(mountNav());
    for (const route of ["ai", "storage", "fleet", "users", "rbac", "topology", "nodes", "supply-chain", "observability"]) {
      expect(routes).toContain(route);
    }
  });

  // reactive re-render when store notifies
  it("re-renders when clusterFeaturesStore notifies a change", () => {
    let storeListener: (() => void) | undefined;
    vi.mocked(clusterFeaturesStore.subscribe as ReturnType<typeof vi.fn>).mockImplementation(
      (cb: () => void) => {
        storeListener = cb;
        return () => {};
      },
    );

    vi.mocked(isFeatureAvailable).mockReturnValue(false);
    const nav = mountNav();
    expect(navRoutes(nav)).not.toContain("ai");

    vi.mocked(isFeatureAvailable).mockReturnValue(true);
    storeListener?.();

    expect(navRoutes(nav)).toContain("ai");
  });
});
