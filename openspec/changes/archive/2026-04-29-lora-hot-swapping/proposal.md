# Proposal: LoRA Hot-Swapping (Direct-to-GPU Inference)

## Context
In a Multi-Tenant Edge FaaS environment, GPU/NPU VRAM is the most critical bottleneck. Hosting multiple specialized Large Language Models (LLMs) simultaneously for different tenants is impossible without triggering Out-Of-Memory (OOM) crashes. While the `Large Model Broker` (Change 071) handles the storage of massive binaries on disk, constantly loading full foundation models into VRAM destroys the targeted sub-millisecond latency. 
The "Bring-Your-Own-LoRA" approach solves this by keeping a single, immutable Foundation Model in shared RAM/VRAM, while applying only small, tenant-specific weight adapters (`.safetensors` files) on the fly during inference.

## Objective
1. Allow WebAssembly (Wasm) modules to request a specific LoRA adapter during an inference call without breaking the W3C FaaS standard.
2. Implement a "Hot-Swap" mechanism in the execution engine (Candle) to dynamically inject and remove adapter weights.
3. Protect overall system latency by limiting the hardware Context Switching overhead.

## Scope
- Update Wasm Component Model definitions (`wit/ai`) to accept an optional `adapter_id` parameter (Opt-In).
- Modify the `wasi-nn-candle` module to parse and apply `.safetensors` matrices onto the foundation model's execution graph.
- Integrate with `system-faas-model-broker` to retrieve locally stored adapters.