# Tasks: Change 044 Implementation (Full Spectrum)

**Agent Instruction:** Implement the 4-way hardware router. Use a modular architecture where backends are compiled as optional features. Do not use nested code blocks in your outputs.

## [TASK-1] Modular Backend Trait
1. Define `trait WasiNnBackend: Send + Sync`.
2. Methods: `init(model_bytes)`, `execute(inputs) -> outputs`.
3. Implement `CpuBackend` (using `ort` crate), `GpuBackend` (using `candle`), `NpuBackend` (using `openvino`), and `TpuBackend` (using `tensorflow-sys` or `libtpu` bindings).

## [TASK-2] Universal Dispatcher & Queue Management
1. In the `core-host`, create 4 dedicated `tokio::mpsc` channels, one for each hardware type.
2. Spawn 4 dedicated OS threads (pinned to relevant cores if possible) to handle the hardware-specific blocking calls.
3. Implement the `wasi_nn::compute` function to act as a router: it looks up the model's assigned device and sends the data to the correct channel.

## [TASK-3] Memory Management & Buffering
1. Implement "Zero-Copy" where possible: use shared memory pointers between the WASM guest and the hardware backends.
2. For TPU/GPU, ensure tensors are stayed in VRAM/SRAM between calls to avoid PCIe bottleneck.

## Validation Step
1. Run a Tachyon node on a machine with a CPU, an integrated NPU (Intel/Apple), and a discrete GPU (Nvidia). 
2. (Optional) Connect a Coral USB TPU.
3. Deploy a multi-modal FaaS that calls all four devices in a loop.
4. Verify using `top`, `nvidia-smi`, and `intel_gpu_top` that all four accelerators are working in parallel.