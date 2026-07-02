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
  models?: ManifestModelBinding[];
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

export type GpuDistribution =
  | "single"
  | "tensor_parallelism"
  | "pipeline_parallelism"
  | "expert_parallelism";

export type HardwareStrategy = {
  distribution_mode: GpuDistribution;
  device_ids: number[];
  stage_layer_ranges: Array<[number, number]>;
  expert_device_map: Array<[number, number]>;
  pipeline_depth: number;
  paged_attention: boolean;
  cuda_graph_decode: boolean;
  flashinfer_attention: boolean;
  prefill_chunk_tokens?: number;
  speculative_draft_model_path: string;
  speculative_draft_tokens: number;
};

export type ManifestModelBinding = {
  alias?: string;
  path?: string;
  device?: string;
  hardware_strategy?: Partial<HardwareStrategy>;
  [key: string]: unknown;
};

export type ManifestModelHardwareBinding = {
  routePath: string;
  routeName: string;
  routeVersion: string;
  alias: string;
  path: string;
  device: string;
  hardwareStrategy: HardwareStrategy;
};

const defaultHardwareStrategy: HardwareStrategy = {
  distribution_mode: "single",
  device_ids: [],
  stage_layer_ranges: [],
  expert_device_map: [],
  pipeline_depth: 0,
  paged_attention: false,
  cuda_graph_decode: false,
  flashinfer_attention: false,
  speculative_draft_model_path: "",
  speculative_draft_tokens: 0,
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

export async function listManifestModelHardwareBindings(): Promise<ManifestModelHardwareBinding[]> {
  const config = await readManifestConfig();
  return (config.routes ?? []).flatMap((route) =>
    (route.models ?? [])
      .filter((model) => typeof model.alias === "string" && model.alias.trim().length > 0)
      .map((model) => ({
        routePath: route.path,
        routeName: route.name ?? "",
        routeVersion: route.version ?? "",
        alias: model.alias!.trim(),
        path: model.path ?? "",
        device: model.device ?? "cpu",
        hardwareStrategy: normalizeHardwareStrategy(model.hardware_strategy),
      })),
  );
}

export async function writeModelHardwareStrategy(
  routePath: string,
  modelAlias: string,
  strategy: HardwareStrategy,
): Promise<SealApplyOutcome> {
  const config = await readManifestConfig();
  const routes = config.routes ?? [];
  const routeIdx = routes.findIndex((route) => route.path === routePath);
  if (routeIdx === -1) {
    throw new Error(`Route '${routePath}' not found in manifest`);
  }

  const route = routes[routeIdx];
  const models = route.models ?? [];
  const modelIdx = models.findIndex((model) => model.alias === modelAlias);
  if (modelIdx === -1) {
    throw new Error(`Model '${modelAlias}' not found on route '${routePath}'`);
  }

  const normalized = normalizeHardwareStrategy(strategy);
  const updatedModel = { ...models[modelIdx] };
  if (isDefaultHardwareStrategy(normalized)) {
    delete updatedModel.hardware_strategy;
  } else {
    updatedModel.hardware_strategy = normalized;
  }

  const updatedRoute: ManifestRoute = {
    ...route,
    models: [...models.slice(0, modelIdx), updatedModel, ...models.slice(modelIdx + 1)],
  };

  return applyManifestConfig({
    ...config,
    routes: [...routes.slice(0, routeIdx), updatedRoute, ...routes.slice(routeIdx + 1)],
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

export function normalizeHardwareStrategy(strategy: Partial<HardwareStrategy> | undefined): HardwareStrategy {
  const distribution = strategy?.distribution_mode;
  return {
    distribution_mode:
      distribution === "tensor_parallelism" ||
      distribution === "pipeline_parallelism" ||
      distribution === "expert_parallelism"
        ? distribution
        : "single",
    device_ids: normalizeNumberList(strategy?.device_ids),
    stage_layer_ranges: normalizePairList(strategy?.stage_layer_ranges),
    expert_device_map: normalizePairList(strategy?.expert_device_map),
    pipeline_depth: normalizeNonNegativeInteger(strategy?.pipeline_depth),
    paged_attention: strategy?.paged_attention === true,
    cuda_graph_decode: strategy?.cuda_graph_decode === true,
    flashinfer_attention: strategy?.flashinfer_attention === true,
    prefill_chunk_tokens:
      strategy?.prefill_chunk_tokens === undefined
        ? undefined
        : normalizeNonNegativeInteger(strategy.prefill_chunk_tokens),
    speculative_draft_model_path: strategy?.speculative_draft_model_path ?? "",
    speculative_draft_tokens: normalizeNonNegativeInteger(strategy?.speculative_draft_tokens),
  };
}

function isDefaultHardwareStrategy(strategy: HardwareStrategy): boolean {
  return (
    strategy.distribution_mode === defaultHardwareStrategy.distribution_mode &&
    strategy.device_ids.length === 0 &&
    strategy.stage_layer_ranges.length === 0 &&
    strategy.expert_device_map.length === 0 &&
    strategy.pipeline_depth === 0 &&
    !strategy.paged_attention &&
    !strategy.cuda_graph_decode &&
    !strategy.flashinfer_attention &&
    strategy.prefill_chunk_tokens === undefined &&
    strategy.speculative_draft_model_path.trim() === "" &&
    strategy.speculative_draft_tokens === 0
  );
}

function normalizeNumberList(value: unknown): number[] {
  if (!Array.isArray(value)) {
    return [];
  }
  return value.map(normalizeNonNegativeInteger).filter((item, index, list) => list.indexOf(item) === index);
}

function normalizePairList(value: unknown): Array<[number, number]> {
  if (!Array.isArray(value)) {
    return [];
  }
  return value
    .filter((item): item is [unknown, unknown] => Array.isArray(item) && item.length >= 2)
    .map(([left, right]) => [normalizeNonNegativeInteger(left), normalizeNonNegativeInteger(right)] as [number, number]);
}

function normalizeNonNegativeInteger(value: unknown): number {
  const numeric = typeof value === "number" ? value : Number.parseInt(String(value ?? "0"), 10);
  if (!Number.isFinite(numeric) || numeric < 0) {
    return 0;
  }
  return Math.trunc(numeric);
}

async function readManifestConfig(): Promise<ManifestConfig> {
  return invoke<ManifestConfig>("get_manifest_config");
}

async function applyManifestConfig(config: ManifestConfig): Promise<SealApplyOutcome> {
  return invoke<SealApplyOutcome>("apply_manifest_config", { config });
}
