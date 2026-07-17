import type { ScopesMap } from "../types/scopes";
import { loggedInvoke as invoke, logUiError } from "../utils/appLogger";

type ManifestConfig = {
  routes?: Array<{
    path: string;
    scopes?: ScopesMap | "allow-all";
    [key: string]: unknown;
  }>;
  [key: string]: unknown;
};

type RuntimeMetrics = {
  scopeDenialTotal?: number;
  [key: string]: unknown;
};

type SealApplyOutcome = {
  success: boolean;
  message: string;
  configVersion: number;
};

export async function readRouteScopes(routePath: string): Promise<{
  scopes: ScopesMap;
  allowAll: boolean;
}> {
  const config = await invoke<ManifestConfig>("get_manifest_config");
  const route = config.routes?.find((r) => r.path === routePath);
  if (!route) {
    throw logMissingRoute(routePath);
  }
  const rawScopes = route.scopes;
  if (!rawScopes || rawScopes === "allow-all") {
    return { scopes: {}, allowAll: true };
  }
  return { scopes: rawScopes as ScopesMap, allowAll: false };
}

export async function writeRouteScopes(
  routePath: string,
  scopes: ScopesMap | "allow-all",
): Promise<SealApplyOutcome> {
  const config = await invoke<ManifestConfig>("get_manifest_config");
  const routes = config.routes ?? [];
  const idx = routes.findIndex((r) => r.path === routePath);
  if (idx === -1) {
    throw logMissingRoute(routePath);
  }
  const updated = { ...routes[idx], scopes };
  const newConfig: ManifestConfig = {
    ...config,
    routes: [...routes.slice(0, idx), updated, ...routes.slice(idx + 1)],
  };
  return invoke<SealApplyOutcome>("apply_manifest_config", { config: newConfig });
}

export async function readScopeMetrics(): Promise<{ scopeDenialTotal: number }> {
  const metrics = await invoke<RuntimeMetrics>("get_metrics");
  return { scopeDenialTotal: metrics.scopeDenialTotal ?? 0 };
}

function logMissingRoute(routePath: string): Error {
  const error = new Error(`Route '${routePath}' not found in manifest`);
  logUiError("scopes.manifest", error);
  return error;
}
