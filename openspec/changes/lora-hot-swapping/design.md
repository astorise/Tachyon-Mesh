# Design: LoRA Hot-Swapping Architecture

## 1. Zero-Cost Abstraction (Opt-In via WIT)
The `wasi-nn` standard is strictly inference-oriented. We will use execution context metadata capabilities to pass the LoRA identifier without breaking the W3C standard.
- **Tenant Without LoRA:** Calls `compute()`. Zero overhead.
- **Tenant With LoRA:** Defines an execution metadata key (e.g., `tachyon.lora_id = "tenantA_sales_v1"`), then calls `compute()`.

## 2. Candle Execution & VRAM Management
The foundation model (e.g., Llama 3 8B Quantized via *TurboQuant*) is persistently loaded (Read-Only) in memory.
When a FaaS request containing a `lora_id` is processed:
1. **Fetch:** `wasi-nn-candle` requests the `.safetensors` file from the `system-faas-model-broker`.
2. **Inject:** The Candle engine loads the adapter tensors into VRAM and adds the weights to the corresponding attention layers of the foundation graph.
3. **Compute:** Token generation executes normally.
4. **Eject:** LoRA weights are strictly offloaded from VRAM immediately after the response generation to prevent leaks (OOM), leaving the GPU clean for the next FaaS execution.

## 3. Latency Mitigation (Synergy with system-faas-buffer)
To avoid a performance collapse caused by constant GPU Context Switching:
- The `system-faas-buffer` (AI request queue) will implement **LoRA-Aware Batching**. If it detects multiple asynchronous requests requiring the same `lora_id`, it groups them to execute a *Batch Inference*, paying the VRAM loading penalty only once.

## 4. Cost Isolation (FinOps)
The operation of loading LoRA tensors requires additional hardware clock cycles. These cycles are measured by the `system-faas-metering` component and billed as "Wasm Fuel" specifically to the Tenant who activated the option, preserving resource equity across the node.