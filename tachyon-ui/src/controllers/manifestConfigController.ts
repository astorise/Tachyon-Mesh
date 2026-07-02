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
  min_instances?: number;
  max_concurrency?: number;
  env?: Record<string, string>;
  domains?: string[];
  canary?: CanaryConfig | null;
  resiliency?: RouteResiliencyConfig | null;
  concurrency?: RouteConcurrencyConfig | null;
  distributed_rate_limit?: DistributedRateLimitConfig | null;
  resource_policy?: ResourcePolicyConfig | null;
  adapter_id?: string;
  shadow_target?: string;
  requires_tee?: boolean;
  volumes?: ManifestVolume[];
  models?: ManifestModelBinding[];
  [key: string]: unknown;
};

type ManifestConfig = {
  routes?: ManifestRoute[];
  kv_caches?: Array<{ name: string; model_ref: string; [key: string]: unknown }>;
  layer4?: Layer4Config;
  enrollment?: EnrollmentConfig;
  tee_backend?: TeeBackendConfig;
  telemetry_sample_rate?: number;
  instance_pool_max_memory_bytes?: number;
  cloud_sync_endpoint?: string;
  batch_targets?: Array<{ name: string; module: string; [key: string]: unknown }>;
  require_scopes?: boolean;
  trusted_signers?: string[];
  [key: string]: unknown;
};

export type ManifestRouteOption = {
  path: string;
  name: string;
  version: string;
  requiresTee: boolean;
};

export type Layer4Binding = {
  port: number;
  target: string;
};

export type Layer4Config = {
  tcp?: Layer4Binding[];
  udp?: Layer4Binding[];
};

export type EnrollmentConfig = {
  mode: "pin" | "zero-touch" | "both";
  oidc_issuer?: string;
  oidc_audience?: string;
  auto_approve_tags?: string[];
};

export type TeeBackendConfig =
  | { kind: "local-enclave" }
  | { kind: "enarx"; keep_endpoint: string };

export type ManifestOperatorConfig = {
  layer4: Layer4Config;
  teeBackend: TeeBackendConfig | null;
  telemetrySampleRate: number;
  instancePoolMaxMemoryBytes: number | null;
  cloudSyncEndpoint: string;
  batchTargets: string[];
  requireScopes: boolean;
};

export type ManifestVolume = {
  type?: "host" | "ram" | "s3";
  host_path: string;
  guest_path: string;
  readonly?: boolean;
  ttl_seconds?: number;
  idle_timeout?: string;
  eviction_policy?: "hibernate";
  encrypted?: boolean;
  backup_schedule?: string | {
    cron: string;
    coordination?: "per_node" | "mesh_leader" | "manual_only";
    write_isolation?: "none" | "drain" | "copy_on_write";
  };
  consistency?: {
    read_mode?: "snapshot" | "live";
    write_mode?: "last_write_wins" | "optimistic_etag" | "pessimistic_lock" | "none";
  };
  [key: string]: unknown;
};

export type CanaryConfig = {
  next_version: string;
  step_weight: number;
  interval_secs: number;
  max_error_rate: number;
};

export type RouteResiliencyConfig = {
  timeout_ms?: number;
  retry_policy?: {
    max_retries: number;
    retry_on: number[];
  };
};

export type RouteConcurrencyConfig = {
  mode: string;
  on_conflict: string;
  lock_ttl_ms?: number;
};

export type DistributedRateLimitConfig = {
  threshold?: number;
  window_seconds?: number;
  scope?: string;
};

export type ResourcePolicyConfig = {
  vram_mb?: number;
  gpu_affinity?: string;
  admission_strategy?: string;
};

export type ManifestRouteResiliency = ManifestRouteOption & {
  resiliency: RouteResiliencyConfig | null;
};

export type ManifestRoutePolicy = ManifestRouteOption & {
  concurrency: RouteConcurrencyConfig | null;
  distributedRateLimit: DistributedRateLimitConfig | null;
  resourcePolicy: ResourcePolicyConfig | null;
  adapterId: string;
  shadowTarget: string;
  models: ManifestModelPolicyBinding[];
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
  qos?: string;
  hardware_strategy?: Partial<HardwareStrategy>;
  [key: string]: unknown;
};

export type ManifestModelPolicyBinding = {
  alias: string;
  path: string;
  qos: string;
  minInstances: number | null;
  maxConcurrency: number | null;
  env: Record<string, string>;
  domains: string[];
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
    requiresTee: route.requires_tee === true,
  }));
}

export async function listRouteResiliencyPolicies(): Promise<ManifestRouteResiliency[]> {
  const config = await readManifestConfig();
  return (config.routes ?? []).map((route) => ({
    path: route.path,
    name: route.name ?? "",
    version: route.version ?? "",
    requiresTee: route.requires_tee === true,
    resiliency: normalizeRouteResiliency(route.resiliency),
  }));
}

export async function listRoutePolicies(): Promise<ManifestRoutePolicy[]> {
  const config = await readManifestConfig();
  return (config.routes ?? []).map((route) => ({
    path: route.path,
    name: route.name ?? "",
    version: route.version ?? "",
    requiresTee: route.requires_tee === true,
    concurrency: normalizeRouteConcurrency(route.concurrency),
    distributedRateLimit: normalizeDistributedRateLimit(route.distributed_rate_limit),
    resourcePolicy: normalizeResourcePolicy(route.resource_policy),
    adapterId: typeof route.adapter_id === "string" ? route.adapter_id : "",
    shadowTarget: typeof route.shadow_target === "string" ? route.shadow_target : "",
    models: (route.models ?? []).map((model) => normalizeModelPolicyBinding(model, route)),
  }));
}

export async function writeRouteResiliency(
  routePath: string,
  resiliency: RouteResiliencyConfig | null,
): Promise<SealApplyOutcome> {
  const config = await readManifestConfig();
  const routes = config.routes ?? [];
  const idx = routes.findIndex((route) => route.path === routePath);
  if (idx === -1) {
    throw new Error(`Route '${routePath}' not found in manifest`);
  }

  const updated = { ...routes[idx] };
  const normalized = normalizeRouteResiliency(resiliency);
  if (normalized) {
    updated.resiliency = normalized;
  } else {
    delete updated.resiliency;
  }

  return applyManifestConfig({
    ...config,
    routes: [...routes.slice(0, idx), updated, ...routes.slice(idx + 1)],
  });
}

export async function writeRouteField(
  routePath: string,
  field: "concurrency" | "distributed_rate_limit" | "resource_policy" | "adapter_id" | "shadow_target",
  value: unknown,
): Promise<SealApplyOutcome> {
  const config = await readManifestConfig();
  const routes = config.routes ?? [];
  const idx = routes.findIndex((route) => route.path === routePath);
  if (idx === -1) {
    throw new Error(`Route '${routePath}' not found in manifest`);
  }

  const updated = { ...routes[idx] };
  const normalized = normalizeRouteField(field, value);
  if (normalized === null || normalized === "" || normalized === undefined) {
    delete updated[field];
  } else {
    (updated as Record<string, unknown>)[field] = normalized;
  }

  return applyManifestConfig({
    ...config,
    routes: [...routes.slice(0, idx), updated, ...routes.slice(idx + 1)],
  });
}

export async function writeRoutePolicy(
  routePath: string,
  policy: {
    concurrency?: RouteConcurrencyConfig | null;
    distributedRateLimit?: DistributedRateLimitConfig | null;
    resourcePolicy?: ResourcePolicyConfig | null;
    adapterId?: string;
    shadowTarget?: string;
  },
): Promise<SealApplyOutcome> {
  const config = await readManifestConfig();
  const routes = config.routes ?? [];
  const idx = routes.findIndex((route) => route.path === routePath);
  if (idx === -1) {
    throw new Error(`Route '${routePath}' not found in manifest`);
  }

  const updated = { ...routes[idx] };
  if (hasOwn(policy, "concurrency")) {
    setRouteField(updated, "concurrency", normalizeRouteConcurrency(policy.concurrency));
  }
  if (hasOwn(policy, "distributedRateLimit")) {
    setRouteField(updated, "distributed_rate_limit", normalizeDistributedRateLimit(policy.distributedRateLimit));
  }
  if (hasOwn(policy, "resourcePolicy")) {
    setRouteField(updated, "resource_policy", normalizeResourcePolicy(policy.resourcePolicy));
  }
  if (hasOwn(policy, "adapterId")) {
    setRouteField(updated, "adapter_id", normalizeOptionalString(policy.adapterId));
  }
  if (hasOwn(policy, "shadowTarget")) {
    setRouteField(updated, "shadow_target", normalizeOptionalString(policy.shadowTarget));
  }

  return applyManifestConfig({
    ...config,
    routes: [...routes.slice(0, idx), updated, ...routes.slice(idx + 1)],
  });
}

export async function writeRouteConcurrencyPolicy(
  routePath: string,
  concurrency: RouteConcurrencyConfig,
  consistency?: ManifestVolume["consistency"] | null,
): Promise<SealApplyOutcome> {
  const config = await readManifestConfig();
  const routes = config.routes ?? [];
  const idx = routes.findIndex((route) => route.path === routePath);
  if (idx === -1) {
    throw new Error(`Route '${routePath}' not found in manifest`);
  }

  const updated: ManifestRoute = {
    ...routes[idx],
    concurrency: normalizeRouteConcurrency(concurrency) ?? undefined,
  };

  const normalizedConsistency = normalizeVolumeConsistency(consistency);
  if (normalizedConsistency && (updated.volumes?.length ?? 0) > 0) {
    updated.volumes = updated.volumes!.map((volume) => ({
      ...volume,
      consistency: normalizedConsistency,
    }));
  }

  return applyManifestConfig({
    ...config,
    routes: [...routes.slice(0, idx), updated, ...routes.slice(idx + 1)],
  });
}

export async function writeRouteModelPolicy(
  routePath: string,
  modelAlias: string,
  policy: Pick<ManifestModelPolicyBinding, "qos" | "minInstances" | "maxConcurrency" | "env" | "domains">,
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

  const updatedModel = { ...models[modelIdx] };
  setModelField(updatedModel, "qos", normalizeOptionalString(policy.qos));
  delete updatedModel.min_instances;
  delete updatedModel.max_concurrency;
  delete updatedModel.env;
  delete updatedModel.domains;

  const updatedRoute: ManifestRoute = {
    ...route,
    models: [...models.slice(0, modelIdx), updatedModel, ...models.slice(modelIdx + 1)],
  };
  setRouteField(updatedRoute, "min_instances", normalizeOptionalNumber(policy.minInstances));
  setRouteField(updatedRoute, "max_concurrency", normalizeOptionalNumber(policy.maxConcurrency));
  setRouteField(updatedRoute, "env", normalizeEnv(policy.env));
  setRouteField(updatedRoute, "domains", normalizeStringList(policy.domains));

  return applyManifestConfig({
    ...config,
    routes: [...routes.slice(0, routeIdx), updatedRoute, ...routes.slice(routeIdx + 1)],
  });
}

export async function getEnrollmentConfig(): Promise<EnrollmentConfig> {
  const config = await readManifestConfig();
  return normalizeEnrollment(config.enrollment);
}

export async function writeEnrollmentConfig(enrollment: EnrollmentConfig): Promise<SealApplyOutcome> {
  const normalized = normalizeEnrollment(enrollment);
  if (normalized.mode !== "pin" && !normalized.oidc_issuer) {
    throw new Error("OIDC issuer is required when zero-touch enrollment is enabled.");
  }
  const invalidTag = (normalized.auto_approve_tags ?? []).find((tag) => !/^[^=]+=[^=]+$/.test(tag));
  if (invalidTag) {
    throw new Error(`Auto-approve tag '${invalidTag}' must use key=value syntax.`);
  }

  const config = await readManifestConfig();
  const next = { ...config };
  if (
    normalized.mode === "pin" &&
    !normalized.oidc_issuer &&
    !normalized.oidc_audience &&
    (normalized.auto_approve_tags ?? []).length === 0
  ) {
    delete next.enrollment;
  } else {
    next.enrollment = normalized;
  }
  return applyManifestConfig(next);
}

export async function getTrustedSigners(): Promise<string[]> {
  const config = await readManifestConfig();
  return Array.isArray(config.trusted_signers)
    ? config.trusted_signers.filter((signer): signer is string => typeof signer === "string")
    : [];
}

export async function writeTrustedSigners(signers: string[]): Promise<SealApplyOutcome> {
  const normalized = normalizeStringList(signers);
  const invalid = normalized.find((signer) => !/^[0-9a-fA-F]{64}$/.test(signer));
  if (invalid) {
    throw new Error(`Trusted signer '${invalid}' must be a 64-character hex Ed25519 public key.`);
  }
  const config = await readManifestConfig();
  const next: ManifestConfig = { ...config, trusted_signers: normalized };
  if (normalized.length === 0) {
    delete next.trusted_signers;
  }
  return applyManifestConfig(next);
}

export async function getManifestOperatorConfig(): Promise<ManifestOperatorConfig> {
  const config = await readManifestConfig();
  return {
    layer4: normalizeLayer4(config.layer4),
    teeBackend: normalizeTeeBackend(config.tee_backend),
    telemetrySampleRate: normalizeRate(config.telemetry_sample_rate),
    instancePoolMaxMemoryBytes: normalizeOptionalNumber(config.instance_pool_max_memory_bytes),
    cloudSyncEndpoint: typeof config.cloud_sync_endpoint === "string" ? config.cloud_sync_endpoint : "",
    batchTargets: (config.batch_targets ?? [])
      .map((target) => target.name)
      .filter((name): name is string => typeof name === "string" && name.trim().length > 0),
    requireScopes: config.require_scopes === true,
  };
}

export async function writeManifestOperatorConfig(options: ManifestOperatorConfig): Promise<SealApplyOutcome> {
  const config = await readManifestConfig();
  const next: ManifestConfig = { ...config };
  const layer4 = normalizeLayer4(options.layer4);
  if ((layer4.tcp?.length ?? 0) + (layer4.udp?.length ?? 0) > 0) {
    next.layer4 = layer4;
  } else {
    delete next.layer4;
  }
  if (options.teeBackend) {
    next.tee_backend = options.teeBackend;
  } else {
    delete next.tee_backend;
  }
  next.telemetry_sample_rate = normalizeRate(options.telemetrySampleRate);
  const memoryBytes = normalizeOptionalNumber(options.instancePoolMaxMemoryBytes);
  if (memoryBytes && memoryBytes > 0) {
    next.instance_pool_max_memory_bytes = memoryBytes;
  } else {
    delete next.instance_pool_max_memory_bytes;
  }
  const endpoint = options.cloudSyncEndpoint.trim();
  if (endpoint) {
    next.cloud_sync_endpoint = endpoint;
  } else {
    delete next.cloud_sync_endpoint;
  }
  const batchTargetNames = normalizeStringList(options.batchTargets);
  const existingBatchTargets = new Map((config.batch_targets ?? []).map((target) => [target.name, target]));
  next.batch_targets = batchTargetNames.map((name) => existingBatchTargets.get(name) ?? { name, module: name });
  if (next.batch_targets.length === 0) {
    delete next.batch_targets;
  }
  next.require_scopes = options.requireScopes;
  if (!next.require_scopes) {
    delete next.require_scopes;
  }
  return applyManifestConfig(next);
}

export async function writeRouteRequiresTee(routePath: string, requiresTee: boolean): Promise<SealApplyOutcome> {
  const config = await readManifestConfig();
  const routes = config.routes ?? [];
  const idx = routes.findIndex((route) => route.path === routePath);
  if (idx === -1) {
    throw new Error(`Route '${routePath}' not found in manifest`);
  }
  const updated = { ...routes[idx] };
  if (requiresTee) {
    updated.requires_tee = true;
  } else {
    delete updated.requires_tee;
  }
  return applyManifestConfig({
    ...config,
    routes: [...routes.slice(0, idx), updated, ...routes.slice(idx + 1)],
  });
}

export async function listRouteVolumes(routePath: string): Promise<ManifestVolume[]> {
  const route = await findRoute(routePath);
  return route.volumes ?? [];
}

export async function writeRouteVolume(routePath: string, originalGuestPath: string, volume: ManifestVolume): Promise<SealApplyOutcome> {
  const config = await readManifestConfig();
  const routes = config.routes ?? [];
  const routeIdx = routes.findIndex((route) => route.path === routePath);
  if (routeIdx === -1) {
    throw new Error(`Route '${routePath}' not found in manifest`);
  }
  const route = routes[routeIdx];
  const volumes = route.volumes ?? [];
  const volumeIdx = volumes.findIndex((item) => item.guest_path === originalGuestPath);
  if (volumeIdx === -1) {
    throw new Error(`Volume '${originalGuestPath}' not found on route '${routePath}'`);
  }
  const normalized = normalizeVolume(volume);
  const updatedRoute = {
    ...route,
    volumes: [...volumes.slice(0, volumeIdx), normalized, ...volumes.slice(volumeIdx + 1)],
  };
  return applyManifestConfig({
    ...config,
    routes: [...routes.slice(0, routeIdx), updatedRoute, ...routes.slice(routeIdx + 1)],
  });
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

async function findRoute(routePath: string): Promise<ManifestRoute> {
  const config = await readManifestConfig();
  const route = (config.routes ?? []).find((item) => item.path === routePath);
  if (!route) {
    throw new Error(`Route '${routePath}' not found in manifest`);
  }
  return route;
}

function normalizeEnrollment(value: unknown): EnrollmentConfig {
  const input = typeof value === "object" && value !== null ? value as Partial<EnrollmentConfig> : {};
  const mode = input.mode === "zero-touch" || input.mode === "both" ? input.mode : "pin";
  return {
    mode,
    oidc_issuer: typeof input.oidc_issuer === "string" ? input.oidc_issuer.trim() || undefined : undefined,
    oidc_audience: typeof input.oidc_audience === "string" ? input.oidc_audience.trim() || undefined : undefined,
    auto_approve_tags: normalizeStringList(input.auto_approve_tags ?? []),
  };
}

function normalizeLayer4(value: unknown): Layer4Config {
  const input = typeof value === "object" && value !== null ? value as Layer4Config : {};
  return {
    tcp: normalizeLayer4List(input.tcp),
    udp: normalizeLayer4List(input.udp),
  };
}

function normalizeLayer4List(value: unknown): Layer4Binding[] {
  if (!Array.isArray(value)) {
    return [];
  }
  return value
    .map((item) => {
      const binding = typeof item === "object" && item !== null ? item as Partial<Layer4Binding> : {};
      return { port: normalizeNonNegativeInteger(binding.port), target: String(binding.target ?? "").trim() };
    })
    .filter((binding) => binding.port > 0 && binding.port <= 65535 && binding.target.length > 0);
}

function normalizeTeeBackend(value: unknown): TeeBackendConfig | null {
  if (typeof value !== "object" || value === null) {
    return null;
  }
  const backend = value as Partial<TeeBackendConfig>;
  if (backend.kind === "local-enclave") {
    return { kind: "local-enclave" };
  }
  if (backend.kind === "enarx" && typeof backend.keep_endpoint === "string" && backend.keep_endpoint.trim()) {
    return { kind: "enarx", keep_endpoint: backend.keep_endpoint.trim() };
  }
  return null;
}

function normalizeRate(value: unknown): number {
  const rate = typeof value === "number" ? value : Number.parseFloat(String(value ?? "1"));
  if (!Number.isFinite(rate)) {
    return 1;
  }
  return Math.min(1, Math.max(0, rate));
}

function normalizeOptionalNumber(value: unknown): number | null {
  if (value === null || value === undefined || value === "") {
    return null;
  }
  const parsed = normalizeNonNegativeInteger(value);
  return parsed > 0 ? parsed : null;
}

function normalizeStringList(value: unknown): string[] {
  const items = Array.isArray(value) ? value : String(value ?? "").split(/[\n,]/);
  return [...new Set(items.map((item) => String(item).trim()).filter(Boolean))];
}

function normalizeRouteResiliency(value: unknown): RouteResiliencyConfig | null {
  const input = typeof value === "object" && value !== null ? value as RouteResiliencyConfig : {};
  const timeout = normalizeOptionalNumber(input.timeout_ms);
  const maxRetries = normalizeNonNegativeInteger(input.retry_policy?.max_retries);
  const retryOn = normalizeNumberList(input.retry_policy?.retry_on)
    .filter((status) => status >= 100 && status <= 599);
  const normalized: RouteResiliencyConfig = {};
  if (timeout && timeout > 0) {
    normalized.timeout_ms = timeout;
  }
  if (maxRetries > 0) {
    normalized.retry_policy = {
      max_retries: maxRetries,
      retry_on: retryOn.length > 0 ? retryOn : [502, 503, 504],
    };
  }
  return normalized.timeout_ms || normalized.retry_policy ? normalized : null;
}

function normalizeRouteConcurrency(value: unknown): RouteConcurrencyConfig | null {
  const input = typeof value === "object" && value !== null ? value as Partial<RouteConcurrencyConfig> : {};
  const mode = normalizeOptionalString(input.mode);
  if (!mode) return null;
  const onConflict = normalizeOptionalString(input.on_conflict) || "queue";
  const lockTtlMs = normalizeOptionalNumber(input.lock_ttl_ms);
  return {
    mode,
    on_conflict: onConflict,
    ...(lockTtlMs ? { lock_ttl_ms: lockTtlMs } : {}),
  };
}

function normalizeDistributedRateLimit(value: unknown): DistributedRateLimitConfig | null {
  const input = typeof value === "object" && value !== null ? value as Partial<DistributedRateLimitConfig> : {};
  const threshold = normalizeOptionalNumber(input.threshold);
  const windowSeconds = normalizeOptionalNumber(input.window_seconds);
  const scope = normalizeOptionalString(input.scope);
  const normalized: DistributedRateLimitConfig = {};
  if (threshold) normalized.threshold = threshold;
  if (windowSeconds) normalized.window_seconds = windowSeconds;
  if (scope) normalized.scope = scope;
  return Object.keys(normalized).length > 0 ? normalized : null;
}

function normalizeResourcePolicy(value: unknown): ResourcePolicyConfig | null {
  const input = typeof value === "object" && value !== null ? value as Partial<ResourcePolicyConfig> : {};
  const vramMb = normalizeOptionalNumber(input.vram_mb);
  const gpuAffinity = normalizeOptionalString(input.gpu_affinity);
  const admissionStrategy = normalizeOptionalString(input.admission_strategy);
  const normalized: ResourcePolicyConfig = {};
  if (vramMb) normalized.vram_mb = vramMb;
  if (gpuAffinity) normalized.gpu_affinity = gpuAffinity;
  if (admissionStrategy) normalized.admission_strategy = admissionStrategy;
  return Object.keys(normalized).length > 0 ? normalized : null;
}

function normalizeModelPolicyBinding(value: ManifestModelBinding, route: ManifestRoute): ManifestModelPolicyBinding {
  return {
    alias: value.alias ?? "",
    path: value.path ?? "",
    qos: typeof value.qos === "string" ? value.qos : "",
    minInstances: normalizeOptionalNumber(route.min_instances),
    maxConcurrency: normalizeOptionalNumber(route.max_concurrency),
    env: normalizeEnv(route.env),
    domains: normalizeStringList(route.domains),
  };
}

function normalizeEnv(value: unknown): Record<string, string> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return {};
  }
  return Object.fromEntries(
    Object.entries(value)
      .map(([key, val]) => [key.trim(), String(val).trim()])
      .filter(([key, val]) => key.length > 0 && val.length > 0),
  );
}

function normalizeRouteField(field: Parameters<typeof writeRouteField>[1], value: unknown): unknown {
  if (field === "concurrency") return normalizeRouteConcurrency(value);
  if (field === "distributed_rate_limit") return normalizeDistributedRateLimit(value);
  if (field === "resource_policy") return normalizeResourcePolicy(value);
  return normalizeOptionalString(value);
}

function normalizeVolumeConsistency(value: unknown): ManifestVolume["consistency"] | null {
  if (typeof value !== "object" || value === null) return null;
  const input = value as ManifestVolume["consistency"];
  const readMode = input?.read_mode === "live" ? "live" : "snapshot";
  const writeMode = ["optimistic_etag", "pessimistic_lock", "none"].includes(input?.write_mode ?? "")
    ? input?.write_mode
    : "last_write_wins";
  return { read_mode: readMode, write_mode: writeMode };
}

function normalizeOptionalString(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function hasOwn<T extends object>(value: T, key: PropertyKey): boolean {
  return Object.prototype.hasOwnProperty.call(value, key);
}

function setRouteField<T extends keyof ManifestRoute>(route: ManifestRoute, field: T, value: ManifestRoute[T] | null | undefined): void {
  if (value === null || value === undefined || value === "" || (Array.isArray(value) && value.length === 0)) {
    delete route[field];
  } else {
    route[field] = value;
  }
}

function setModelField<T extends keyof ManifestModelBinding>(model: ManifestModelBinding, field: T, value: ManifestModelBinding[T] | null | undefined): void {
  if (
    value === null ||
    value === undefined ||
    value === "" ||
    (Array.isArray(value) && value.length === 0) ||
    (typeof value === "object" && !Array.isArray(value) && Object.keys(value).length === 0)
  ) {
    delete model[field];
  } else {
    model[field] = value;
  }
}

function normalizeVolume(volume: ManifestVolume): ManifestVolume {
  const normalized: ManifestVolume = {
    type: volume.type === "ram" || volume.type === "s3" ? volume.type : "host",
    host_path: volume.host_path.trim(),
    guest_path: volume.guest_path.trim(),
    readonly: volume.readonly === true,
  };
  if (!normalized.guest_path.startsWith("/")) {
    throw new Error("Guest path must be absolute.");
  }
  if (normalized.type !== "ram" && normalized.host_path.length === 0) {
    throw new Error("Host path is required for host and S3 volumes.");
  }
  const ttl = normalizeOptionalNumber(volume.ttl_seconds);
  if (ttl) normalized.ttl_seconds = ttl;
  const idleTimeout = volume.idle_timeout?.trim();
  if (idleTimeout) normalized.idle_timeout = idleTimeout;
  if (volume.eviction_policy === "hibernate") normalized.eviction_policy = "hibernate";
  if (volume.encrypted === true) normalized.encrypted = true;
  if (volume.backup_schedule !== undefined) {
    normalized.backup_schedule = normalizeBackupSchedule(volume.backup_schedule);
  }
  const readMode = volume.consistency?.read_mode === "live" ? "live" : "snapshot";
  const writeMode = ["optimistic_etag", "pessimistic_lock", "none"].includes(volume.consistency?.write_mode ?? "")
    ? volume.consistency?.write_mode
    : "last_write_wins";
  if (readMode !== "snapshot" || writeMode !== "last_write_wins") {
    normalized.consistency = { read_mode: readMode, write_mode: writeMode };
  }
  return normalized;
}

function normalizeBackupSchedule(schedule: ManifestVolume["backup_schedule"]): ManifestVolume["backup_schedule"] {
  if (typeof schedule === "string") {
    return schedule.trim() || undefined;
  }
  if (!schedule || typeof schedule !== "object") {
    return undefined;
  }
  const cron = schedule.cron.trim();
  if (!cron) {
    return undefined;
  }
  return {
    cron,
    coordination: schedule.coordination ?? "per_node",
    write_isolation: schedule.write_isolation ?? "none",
  };
}
