import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { resilientInvoke } from "../../utils/network";
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { setLanguage } from "../../utils/i18n";

vi.mock("../../utils/network", () => ({
  resilientInvoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(vi.fn()),
}));

import "./TachyonAIPanel";

const invokeMock = vi.mocked(resilientInvoke);
const tauriInvokeMock = vi.mocked(tauriInvoke);

function mountPanel(): HTMLElement {
  const el = document.createElement("tachyon-ai-panel");
  document.body.appendChild(el);
  return el;
}

describe("TachyonAIPanel", () => {
  let manifestConfig: Record<string, unknown>;

  beforeEach(() => {
    document.body.innerHTML = "";
    setLanguage("en");
    manifestConfig = { routes: [], kv_caches: [] };
    vi.mocked(listen).mockClear();
    tauriInvokeMock.mockImplementation(async (command: string) => {
      if (command === "get_manifest_config") {
        return manifestConfig;
      }
      if (command === "apply_manifest_config") {
        return { success: true, message: "applied", configVersion: 2 };
      }
      return null;
    });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_metrics") {
        return {
          source: "live",
          errorRate: 0,
          p50LatencyMs: 10,
          p99LatencyMs: 30,
          queueDepth: 0,
          vramUtilizationPct: 12,
          ramOffloadActive: false,
        };
      }
      if (command === "list_available_models") {
        return [
          { id: "gguf/llama-3", alias: "llama-3", engine: "gguf" },
          { id: "safetensors/qwen", alias: "qwen", engine: "safetensors" },
        ];
      }
      if (command === "get_cluster_hardware_summary") {
        return { source: "mock", enrolledCount: 1, onlineCount: 1, staleCount: 0, totalRamMb: 65536, gpuCount: 2 };
      }
      return null;
    });
  });

  afterEach(() => {
    document.body.innerHTML = "";
    vi.clearAllMocks();
  });

  it("renders available models and writes kv-caches through the runtime manifest", async () => {
    const panel = mountPanel();
    await new Promise((resolve) => setTimeout(resolve, 0));

    const root = panel.shadowRoot;
    expect(root?.textContent).toContain("llama-3");
    expect(root?.textContent).toContain("qwen");

    const llamaMode = root?.querySelector<HTMLSelectElement>('[data-model-alias="llama-3"]');
    expect(llamaMode).toBeTruthy();
    llamaMode!.value = "layer-batch";
    llamaMode!.dispatchEvent(new Event("change"));

    root?.querySelector("form")?.dispatchEvent(new Event("submit", { cancelable: true }));
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(tauriInvokeMock).toHaveBeenCalledWith("apply_manifest_config", {
      config: expect.objectContaining({
        kv_caches: [
          expect.objectContaining({ name: "cache-for-llama-3", model_ref: "llama-3" }),
          expect.objectContaining({ name: "cache-for-qwen", model_ref: "qwen" }),
        ],
      }),
    });
    expect(tauriInvokeMock).not.toHaveBeenCalledWith(
      "apply_configuration",
      expect.objectContaining({ domain: "config-ai" }),
    );
  });

  it("writes hardware_strategy on the selected manifest model binding", async () => {
    manifestConfig = {
      routes: [
        {
          path: "/chat",
          name: "chat",
          version: "1.0.0",
          models: [{ alias: "qwen", path: "/models/qwen", device: "cuda" }],
        },
      ],
      kv_caches: [],
    };

    const panel = mountPanel();
    await new Promise((resolve) => setTimeout(resolve, 0));

    const root = panel.shadowRoot;
    expect(root?.textContent).toContain("Model Hardware Strategy");

    const distribution = root?.querySelector<HTMLSelectElement>('[data-hw-field="distribution_mode"]');
    const devices = root?.querySelector<HTMLInputElement>('[data-hw-field="device_ids"]');
    const paged = root?.querySelector<HTMLInputElement>('[data-hw-field="paged_attention"]');
    const prefill = root?.querySelector<HTMLInputElement>('[data-hw-field="prefill_chunk_tokens"]');
    const draftPath = root?.querySelector<HTMLInputElement>('[data-hw-field="speculative_draft_model_path"]');
    const draftTokens = root?.querySelector<HTMLInputElement>('[data-hw-field="speculative_draft_tokens"]');

    distribution!.value = "tensor_parallelism";
    devices!.value = "0,1";
    paged!.checked = true;
    prefill!.value = "4096";
    draftPath!.value = "/models/qwen-draft";
    draftTokens!.value = "6";

    root?.querySelector<HTMLButtonElement>("[data-hardware-save]")?.click();
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(tauriInvokeMock).toHaveBeenCalledWith("apply_manifest_config", {
      config: expect.objectContaining({
        routes: [
          expect.objectContaining({
            path: "/chat",
            models: [
              expect.objectContaining({
                alias: "qwen",
                hardware_strategy: expect.objectContaining({
                  distribution_mode: "tensor_parallelism",
                  device_ids: [0, 1],
                  paged_attention: true,
                  prefill_chunk_tokens: 4096,
                  speculative_draft_model_path: "/models/qwen-draft",
                  speculative_draft_tokens: 6,
                }),
              }),
            ],
          }),
        ],
      }),
    });
    expect(tauriInvokeMock).not.toHaveBeenCalledWith(
      "apply_configuration",
      expect.objectContaining({ domain: "config-ai" }),
    );
  });
});
