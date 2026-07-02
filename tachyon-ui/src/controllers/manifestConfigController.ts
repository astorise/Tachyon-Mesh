import { invoke } from "@tauri-apps/api/core";

type SealApplyOutcome = {
  success: boolean;
  message: string;
  configVersion: number;
};

type ManifestRoute = {
  path: string;
  name?: string;
  version?: string;
  canary?: CanaryConfig | null;
  models?: Array<{ alias?: string; [key: string]: unknown }>;
  [key: string]: unknown;
};

type ManifestConfig = {
  routes?: ManifestRoute[];
  kv_caches?: Array<{ name: string; model_ref: string; [key: string]: unknown }>;
  [key: string]: unknown;
};

export type ManifestRouteOption = {
  path: string;
  name: string;
  version: string;
};

export type CanaryConfig = {
  next_version: string;
  step_weight: number;
  interval_secs: number;
  max_error_rate: number;
};

export async function listManifestRoutes(): Promise<ManifestRouteOption[]> {
  const config = await readManifestConfig();
  return (config.routes ?? []).map((route) => ({
    path: route.path,
    name: route.name ?? "",
    version: route.version ?? "",
  }));
}

export async function writeRouteCanary(
  routePath: string,
  canary: CanaryConfig | null,
): Promise<SealApplyOutcome> {
  const config = await readManifestConfig();
  const routes = config.routes ?? [];
  const idx = routes.findIndex((route) => route.path === routePath);
  if (idx === -1) {
    throw new Error(`Route '${routePath}' not found in manifest`);
  }

  const updated = { ...routes[idx] };
  if (canary) {
    updated.canary = canary;
  } else {
    delete updated.canary;
  }

  return applyManifestConfig({
    ...config,
    routes: [...routes.slice(0, idx), updated, ...routes.slice(idx + 1)],
  });
}

export async function writeAiKvCaches(modelAliases: string[]): Promise<SealApplyOutcome> {
  const aliases = [...new Set(modelAliases.map((alias) => alias.trim()).filter(Boolean))];
  if (aliases.length === 0) {
    throw new Error("No AI model aliases are available to configure.");
  }

  const config = await readManifestConfig();
  const existing = config.kv_caches ?? [];
  const byModelRef = new Map(existing.map((cache) => [cache.model_ref, cache]));
  for (const alias of aliases) {
    const previous = byModelRef.get(alias);
    byModelRef.set(alias, {
      ...previous,
      name: previous?.name ?? `cache-for-${alias}`,
      model_ref: alias,
      eviction_policy: previous?.eviction_policy ?? "lru",
      tenant_isolation: previous?.tenant_isolation ?? true,
    });
  }

  return applyManifestConfig({
    ...config,
    kv_caches: Array.from(byModelRef.values()).sort((left, right) =>
      left.name.localeCompare(right.name),
    ),
  });
}

async function readManifestConfig(): Promise<ManifestConfig> {
  return invoke<ManifestConfig>("get_manifest_config");
}

async function applyManifestConfig(config: ManifestConfig): Promise<SealApplyOutcome> {
  return invoke<SealApplyOutcome>("apply_manifest_config", { config });
}
