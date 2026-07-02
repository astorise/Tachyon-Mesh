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
    requiresTee: route.requires_tee === true,
  }));
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
