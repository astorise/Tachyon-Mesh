import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import {
  listRoutePolicies,
  writeRouteConcurrencyPolicy,
  writeRouteModelPolicy,
  writeRoutePolicy,
} from "./manifestConfigController";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const applyOk = { success: true, message: "applied", configVersion: 2 };

function mockManifest(config: unknown): void {
  vi.mocked(invoke).mockImplementation(async (command: string) => {
    if (command === "get_manifest_config") return config;
    if (command === "apply_manifest_config") return applyOk;
    throw new Error(`unexpected command ${command}`);
  });
}

function appliedConfig(): unknown {
  const call = vi.mocked(invoke).mock.calls.find(([command]) => command === "apply_manifest_config");
  return (call?.[1] as { config?: unknown } | undefined)?.config;
}

describe("manifestConfigController route policies", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("lists route policy fields from manifest routes", async () => {
    mockManifest({
      routes: [{
        path: "/chat",
        name: "chat",
        version: "v1",
        min_instances: 1,
        max_concurrency: 8,
        env: { TENANT: "a" },
        domains: ["chat"],
        distributed_rate_limit: { threshold: 100, window_seconds: 60, scope: "tenant" },
        resource_policy: { vram_mb: 4096, gpu_affinity: "gpu:0", admission_strategy: "queue" },
        adapter_id: "lora-tenant-a",
        shadow_target: "/chat-shadow",
        models: [{
          alias: "qwen",
          path: "/models/qwen",
          qos: "gold",
        }],
      }],
    });

    await expect(listRoutePolicies()).resolves.toMatchObject([{
      path: "/chat",
      distributedRateLimit: { threshold: 100, window_seconds: 60, scope: "tenant" },
      resourcePolicy: { vram_mb: 4096, gpu_affinity: "gpu:0", admission_strategy: "queue" },
      adapterId: "lora-tenant-a",
      shadowTarget: "/chat-shadow",
      models: [{
        alias: "qwen",
        qos: "gold",
        minInstances: 1,
        maxConcurrency: 8,
        env: { TENANT: "a" },
        domains: ["chat"],
      }],
    }]);
  });

  it("writes concurrency and volume consistency to the selected route", async () => {
    mockManifest({
      routes: [{
        path: "/etl",
        volumes: [{ type: "s3", host_path: "s3://bucket/data", guest_path: "/data" }],
      }],
    });

    await writeRouteConcurrencyPolicy(
      "/etl",
      { mode: "mesh-singleton", on_conflict: "queue" },
      { read_mode: "snapshot", write_mode: "pessimistic_lock" },
    );

    expect(appliedConfig()).toMatchObject({
      routes: [{
        path: "/etl",
        concurrency: { mode: "mesh-singleton", on_conflict: "queue" },
        volumes: [{
          guest_path: "/data",
          consistency: { read_mode: "snapshot", write_mode: "pessimistic_lock" },
        }],
      }],
    });
  });

  it("writes route policy fields and removes empty values", async () => {
    mockManifest({
      routes: [{
        path: "/chat",
        concurrency: { mode: "mesh-singleton", on_conflict: "queue" },
        adapter_id: "old-adapter",
        shadow_target: "/old-shadow",
      }],
    });

    await writeRoutePolicy("/chat", {
      distributedRateLimit: { threshold: 200, window_seconds: 30, scope: "user" },
      resourcePolicy: { vram_mb: 2048, gpu_affinity: "gpu:1", admission_strategy: "reject" },
      adapterId: "",
      shadowTarget: "/shadow",
    });

    expect(appliedConfig()).toMatchObject({
      routes: [{
        path: "/chat",
        concurrency: { mode: "mesh-singleton", on_conflict: "queue" },
        distributed_rate_limit: { threshold: 200, window_seconds: 30, scope: "user" },
        resource_policy: { vram_mb: 2048, gpu_affinity: "gpu:1", admission_strategy: "reject" },
        shadow_target: "/shadow",
      }],
    });
    expect((appliedConfig() as { routes: Array<Record<string, unknown>> }).routes[0]).not.toHaveProperty("adapter_id");
  });

  it("writes model qos on models and route scale, env, and domains on the route", async () => {
    mockManifest({
      routes: [{
        path: "/chat",
        models: [{
          alias: "qwen",
          path: "/models/qwen",
          min_instances: 1,
          max_concurrency: 8,
          env: { TENANT: "old" },
          domains: ["legacy"],
        }],
      }],
    });

    await writeRouteModelPolicy("/chat", "qwen", {
      qos: "gold",
      minInstances: 2,
      maxConcurrency: 16,
      env: { TENANT: "alpha" },
      domains: ["chat", "rag"],
    });

    expect(appliedConfig()).toMatchObject({
      routes: [{
        path: "/chat",
        min_instances: 2,
        max_concurrency: 16,
        env: { TENANT: "alpha" },
        domains: ["chat", "rag"],
        models: [{
          alias: "qwen",
          qos: "gold",
        }],
      }],
    });
    const route = (appliedConfig() as { routes: Array<{ models: Array<Record<string, unknown>> }> }).routes[0];
    const model = route.models[0];
    expect(model).not.toHaveProperty("min_instances");
    expect(model).not.toHaveProperty("max_concurrency");
    expect(model).not.toHaveProperty("env");
    expect(model).not.toHaveProperty("domains");
  });
});
