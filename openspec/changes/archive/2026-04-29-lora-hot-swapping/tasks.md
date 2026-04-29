# Tasks: LoRA Hot-Swapping Implementation

**Agent Instruction:** Implement LoRA adapter support within the Wasm inference stack. Ensure the foundation model is never duplicated in memory during the hot-swapping process.

- [x] **Interface Definition:** Extend the Wasm graph configuration logic (in `wasi-nn`) to accept and store an optional `adapter_id` via `context_metadata`.
- [x] **Asset Resolution:** Connect the inference backend to the `system-faas-model-broker` to stream/read local `.safetensors` files from the Edge node's disk.
- [x] **Candle Injection:** Modify `core-host/src/ai_inference.rs` (or the underlying `wasi-nn-candle` implementation) to apply the LoRA weights to the tensor graph before executing `forward()`.
- [x] **VRAM Cleanup:** Implement a strict deallocation (`Drop`) routine for LoRA tensors immediately after token generation to enforce the `Rate Limiter OOM Protection`.
- [x] **FinOps Metering:** Add a new "Fuel" consumption event inside `system-faas-metering` triggered specifically by the overhead time taken to load an adapter into memory.
