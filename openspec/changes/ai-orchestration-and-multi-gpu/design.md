# Design: AI Orchestration Data Model

## 1. The GitOps YAML Specification
This configuration orchestrates how the Candle engine manages weights in memory and processes inference.

    api_version: ai.tachyon.io/v1alpha1
    kind: AiOrchestration
    metadata:
      name: global-inference-fleet
      environment: production
      
    spec:
      model_deployments:
        - name: "llama-3-core"
          asset_ref: "llama-3-8b-instruct" # Links to Domain 8 (Air-Gapped Asset)
          
          # 1. CANDLE ENGINE CONFIG
          engine:
            backend: auto_detect # Falls back to Metal/CUDA based on Hardware config
            quantization: q4_k_m
            max_context_window: 8192
            flash_attention: true
            
          # 2. MULTI-GPU STRATEGY
          hardware_strategy:
            multi_gpu: true
            distribution_mode: tensor_parallelism # Splits layers across VRAM of multiple GPUs
            
          # 3. MODEL SHARING & LORA MULTIPLEXING
          sharing_strategy:
            mode: lora_multiplexing
            base_model_memory_lock: true # Base model stays pinned in VRAM
            adapters:
              - name: "legal-assistant-lora"
                asset_ref: "lora-legal-v1"
                routing_condition:
                  header_match: { "X-Tenant-Domain": "legal" } # Hot-swaps this LoRA when Legal queries
              - name: "code-copilot-lora"
                asset_ref: "lora-coding-v2"
                routing_condition:
                  header_match: { "X-Tenant-Domain": "engineering" }

## 2. The WIT Contract (`wit/config-ai.wit`)
The strict Wasm interface used by `system-faas-config-api` to validate AI orchestration rules.

    interface config-ai {
        enum engine-backend { cpu, cuda, metal, auto-detect }
        enum gpu-distribution { single, tensor-parallelism, pipeline-parallelism }
        enum sharing-mode { isolated, lora-multiplexing }

        record engine-config {
            backend: engine-backend,
            quantization: option<string>,
            max-context-window: u32,
            flash-attention: bool,
        }

        record hardware-strategy {
            multi-gpu: bool,
            distribution-mode: gpu-distribution,
        }

        record lora-adapter {
            name: string,
            asset-ref: string,
            routing-header-key: string,
            routing-header-value: string,
        }

        record sharing-strategy {
            mode: sharing-mode,
            base-model-memory-lock: bool,
            adapters: list<lora-adapter>,
        }

        record model-deployment {
            name: string,
            asset-ref: string,
            engine: engine-config,
            strategy: hardware-strategy,
            sharing: sharing-strategy,
        }

        record ai-configuration {
            deployments: list<model-deployment>,
        }

        /// Validation (Ensures strategies are compatible with assets)
        validate-ai-config: func(config: ai-configuration) -> result<_, string>;

        /// CRUD for Tachyon-UI
        get-ai-config: func() -> result<ai-configuration, string>;
        apply-model-deployment: func(deployment: model-deployment) -> result<_, string>;
    }