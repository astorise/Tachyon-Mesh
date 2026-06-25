# Design: NPU/TPU Real Device Execution

## 1. Why this sits outside candle's `Device` enum
`candle_core::Device` is `{ Cpu, Cuda(ordinal), Metal(ordinal) }`. NPU/TPU runtimes (OpenVINO, Edge TPU) are not exposed as candle backends upstream, and forking candle to add device variants is out of scope. Instead, this introduces a host-level `AcceleratorBackend` enum that sits *above* candle and *beside* it — for ops that go through candle (GPU/CPU), unchanged; for ops dispatched to NPU/TPU, the host calls the vendor SDK directly and never constructs a `candle::Tensor` on that path.

```rust
enum AcceleratorBackend {
    Candle(candle_core::Device),   // existing CPU/CUDA/Metal path, unchanged
    OpenVinoNpu(openvino::Device), // new
    EdgeTpu(edgetpu::Device),      // new
}
```

## 2. Capability reporting becomes execution-backed
Today, `heterogeneous-accelerator-orchestration`'s capability routing can mark `npu`/`tpu` as affinities without any backend able to execute them. This change ties capability *availability* to backend *presence*:

```rust
struct AcceleratorAvailability {
    backend: AcceleratorClass, // gpu | npu | tpu | cpu
    status: AvailabilityStatus, // Available | Unavailable { reason }
}
```
`status` is `Available` only if the corresponding `AcceleratorBackend` variant was successfully initialized against real hardware/driver at startup (e.g., OpenVINO plugin enumerates an NPU device; `libedgetpu` enumerates a USB Edge TPU). Otherwise `Unavailable { reason: "no_backend_wired" | "driver_not_found" | "device_not_detected" }`.

## 3. Minimal supported op set per class
- **NPU (OpenVINO)**: load a quantized INT8 model in OpenVINO IR format, run inference through the OpenVINO Inference Engine API, return output tensor — this is the minimal proof that the dispatch boundary works end-to-end, scoped to one model type rather than general op coverage.
- **TPU (Edge TPU)**: load a `.tflite` model compiled for Edge TPU, run inference via `libedgetpu`'s delegate, return output tensor — same minimal-proof scope.

Both paths convert host-side input tensors (already produced by the existing inference request pipeline) into the vendor SDK's native tensor format at the dispatch boundary, and convert results back, so the rest of the pipeline (batching, telemetry, response serialization) is unaffected.

## 4. Hardware validation plan (closes the deferred tasks)
Run, and record evidence for, the validation tasks already named in `heterogeneous-accelerator-orchestration`:
1. On a machine with CPU + NPU + GPU: confirm capability discovery correctly enumerates all three and that model dispatch honors declared affinity and fallback.
2. Connect a Coral USB TPU: confirm capability discovery detects it and that a model declared with `tpu` affinity executes via the Edge TPU backend.
3. Verify with `nvidia-smi`/`intel_gpu_top` (and the OpenVINO/Edge TPU equivalents) that the declared backend is the one actually doing the work, not a silent CPU fallback.

Evidence (command output, capability-detection JSON) is attached to the change's acceptance record rather than left as an open, indefinitely-deferred task.

## 5. Compatibility
- Existing GPU/CPU dispatch via `candle_core::Device` is untouched.
- Any target currently declaring `npu`/`tpu` affinity without a wired backend now correctly reports `unavailable` and falls back per existing fallback policy, instead of being silently routed to a label with no real backend.
