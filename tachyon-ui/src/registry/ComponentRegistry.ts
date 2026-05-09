export type ComponentRoute = {
  route: string;
  label: string;
  tagName: string;
};

const routes: ComponentRoute[] = [
  { route: "overview", label: "Overview", tagName: "tachyon-overview-panel" },
  { route: "users", label: "Users & Groups", tagName: "tachyon-users-panel" },
  { route: "routing", label: "Routing", tagName: "tachyon-routing-panel" },
  { route: "resilience", label: "Resilience", tagName: "tachyon-resilience-panel" },
  { route: "ai", label: "AI Orchestration", tagName: "tachyon-ai-panel" },
  { route: "hardware", label: "Hardware", tagName: "tachyon-hardware-panel" },
  { route: "identity-config", label: "Identity & Quotas", tagName: "tachyon-identity-panel" },
  { route: "rbac", label: "RBAC", tagName: "tachyon-rbac-panel" },
  { route: "workloads", label: "Workloads", tagName: "tachyon-workloads-panel" },
  { route: "observability", label: "Observability", tagName: "tachyon-observability-panel" },
  { route: "storage", label: "Storage", tagName: "tachyon-storage-panel" },
  { route: "fleet", label: "Fleet", tagName: "tachyon-fleet-panel" },
  { route: "supply-chain", label: "Supply Chain", tagName: "tachyon-supply-chain-panel" },
];

export function listComponentRoutes(): ComponentRoute[] {
  return [...routes];
}

export function resolveComponentTag(route: string): string | null {
  return routes.find((entry) => entry.route === route)?.tagName ?? null;
}
