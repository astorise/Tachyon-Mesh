# Tasks: Change 044 Implementation (Full Spectrum)

**Agent Instruction:** Implement the 4-way hardware router. Use a modular architecture where backends are compiled as optional features. Do not use nested code blocks in your outputs.

## [TASK-1] Modular Backend Trait
- [x] Define `trait WasiNnBackend: Send + Sync`.
- [x] Methods: `init(model_bytes)`, `execute(inputs) -> outputs`.
- [x] Implement `CpuBackend` (using `ort` crate), `GpuBackend` (using `candle`), `NpuBackend` (using `openvino`), and `TpuBackend` (using `tensorflow-sys` or `libtpu` bindings).

## [TASK-2] Universal Dispatcher & Queue Management
- [x] In the `core-host`, create 4 dedicated `tokio::mpsc` channels, one for each hardware type.
- [x] Spawn 4 dedicated OS threads (pinned to relevant cores if possible) to handle the hardware-specific blocking calls.
- [x] Implement the `wasi_nn::compute` function to act as a router: it looks up the model's assigned device and sends the data to the correct channel.

## [TASK-3] Memory Management & Buffering
- [x] Implement "Zero-Copy" where possible: use shared memory pointers between the WASM guest and the hardware backends.
- [x] For TPU/GPU, ensure tensors are stayed in VRAM/SRAM between calls to avoid PCIe bottleneck.

## Validation Step
- [ ] Run a Tachyon node on a machine with a CPU, an integrated NPU (Intel/Apple), and a discrete GPU (Nvidia).
- [ ] (Optional) Connect a Coral USB TPU.
- [ ] Deploy a multi-modal FaaS that calls all four devices in a loop.
- [ ] Verify using `top`, `nvidia-smi`, and `intel_gpu_top` that all four accelerators are working in parallel.
