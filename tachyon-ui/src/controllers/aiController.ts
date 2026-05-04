import gsap from "gsap";

type AiOrchestrationPayload = {
  api_version: "ai.tachyon.io/v1alpha1";
  kind: "AiOrchestration";
  metadata: {
    name: string;
    environment: string;
  };
  spec: {
    model_deployments: Array<{
      name: string;
      asset_ref: string;
      engine: {
        backend: "auto_detect";
        quantization: "q4_k_m";
        max_context_window: number;
        flash_attention: boolean;
      };
      hardware_strategy: {
        multi_gpu: boolean;
        distribution_mode: "tensor_parallelism";
      };
      sharing_strategy: {
        mode: "lora_multiplexing";
        base_model_memory_lock: boolean;
        adapters: Array<{
          name: string;
          asset_ref: string;
          routing_condition: {
            header_match: Record<string, string>;
          };
        }>;
      };
    }>;
  };
};

export class AiOrchestrationController {
  static init() {
    gsap.from(".ai-card", { opacity: 0, y: 20, stagger: 0.1, duration: 0.4, ease: "power2.out" });
    gsap.from(".lora-rule", {
      opacity: 0,
      x: 20,
      stagger: 0.1,
      duration: 0.4,
      ease: "power2.out",
      delay: 0.2,
    });

    const deployBtn = document.getElementById("btn-deploy-ai");
    if (!deployBtn) {
      return;
    }

    deployBtn.addEventListener("click", () => {
      gsap.to(deployBtn, { scale: 0.95, duration: 0.1, yoyo: true, repeat: 1 });
      const payload = this.buildPayload();
      console.log("Pushing to System FaaS Config API:", JSON.stringify(payload, null, 2));
    });
  }

  private static buildPayload(): AiOrchestrationPayload {
    const modelName = readText("[data-ai-model-name]", "llama-3-core");
    const modelAsset = readText("[data-ai-model-asset]", "llama-3-8b-instruct.gguf")
      .replace(/\.gguf$/i, "");
    const contextWindow = Number.parseInt(readText("[data-ai-model-context]", "8192"), 10) || 8192;
    const adapterName = readText("[data-ai-lora-name]", "Legal Assistant")
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-|-$/g, "");

    return {
      api_version: "ai.tachyon.io/v1alpha1",
      kind: "AiOrchestration",
      metadata: {
        name: "global-inference-fleet",
        environment: "production",
      },
      spec: {
        model_deployments: [
          {
            name: modelName,
            asset_ref: modelAsset,
            engine: {
              backend: "auto_detect",
              quantization: "q4_k_m",
              max_context_window: contextWindow,
              flash_attention: true,
            },
            hardware_strategy: {
              multi_gpu: true,
              distribution_mode: "tensor_parallelism",
            },
            sharing_strategy: {
              mode: "lora_multiplexing",
              base_model_memory_lock: true,
              adapters: [
                {
                  name: `${adapterName}-lora`,
                  asset_ref: "lora-legal-v1",
                  routing_condition: {
                    header_match: {
                      "X-Tenant-Domain": "legal",
                    },
                  },
                },
              ],
            },
          },
        ],
      },
    };
  }
}

function readText(selector: string, fallback: string): string {
  return document.querySelector(selector)?.textContent?.trim() || fallback;
}
