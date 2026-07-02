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
  beforeEach(() => {
    document.body.innerHTML = "";
    setLanguage("en");
    vi.mocked(listen).mockClear();
    tauriInvokeMock.mockImplementation(async (command: string) => {
      if (command === "get_manifest_config") {
        return { routes: [], kv_caches: [] };
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
});
