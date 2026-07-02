import type { ClusterFeatureSet } from "../types/clusterFeatures";

export type ClusterFeature = keyof ClusterFeatureSet;

export type ComponentRoute = {
  route: string;
  label: string;
  tagName: string;
  requires?: ClusterFeature;
};

const routes: ComponentRoute[] = [
  { route: "overview", label: "Overview", tagName: "tachyon-overview-panel" },
  { route: "topology", label: "Topology", tagName: "tachyon-topology-panel", requires: "hasEnrolledNodes" },
  { route: "users", label: "Users & Groups", tagName: "tachyon-users-panel", requires: "hasIdentity" },
  { route: "routing", label: "Routing", tagName: "tachyon-routing-panel", requires: "hasRouting" },
  { route: "resilience", label: "Resilience", tagName: "tachyon-resilience-panel", requires: "hasResilience" },
  { route: "ai", label: "AI Orchestration", tagName: "tachyon-ai-panel", requires: "hasAi" },
  { route: "chat", label: "Chat", tagName: "tachyon-chat-panel" },
  { route: "hardware", label: "Hardware", tagName: "tachyon-hardware-panel", requires: "hasEnrolledNodes" },
  { route: "nodes", label: "Nodes", tagName: "tachyon-nodes-panel", requires: "hasEnrolledNodes" },
  { route: "systems", label: "Systems", tagName: "tachyon-systems-panel", requires: "hasEnrolledNodes" },
  { route: "identity-config", label: "Identity & Quotas", tagName: "tachyon-identity-panel", requires: "hasIdentity" },
  { route: "workloads", label: "Workloads", tagName: "tachyon-workloads-panel", requires: "hasEnrolledNodes" },
  { route: "observability", label: "Observability", tagName: "tachyon-observability-panel", requires: "hasObservability" },
  { route: "storage", label: "Storage", tagName: "tachyon-storage-panel", requires: "hasStorage" },
];

export function listComponentRoutes(): ComponentRoute[] {
  return [...routes];
}

export function resolveComponentTag(route: string): string | null {
  return routes.find((entry) => entry.route === route)?.tagName ?? null;
}

export function resolveComponentRoute(route: string): ComponentRoute | null {
  return routes.find((entry) => entry.route === route) ?? null;
}
