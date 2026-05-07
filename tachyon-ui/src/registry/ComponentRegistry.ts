export type ComponentRoute = {
  route: string;
  label: string;
  tagName: string;
};

const routes: ComponentRoute[] = [
  { route: "routing", label: "Routing", tagName: "tachyon-routing-panel" },
  { route: "resilience", label: "Resilience", tagName: "tachyon-resilience-panel" },
  { route: "ai", label: "AI Orchestration", tagName: "tachyon-ai-panel" },
  { route: "hardware", label: "Hardware", tagName: "tachyon-hardware-panel" },
];

export function listComponentRoutes(): ComponentRoute[] {
  return [...routes];
}

export function resolveComponentTag(route: string): string | null {
  return routes.find((entry) => entry.route === route)?.tagName ?? null;
}
