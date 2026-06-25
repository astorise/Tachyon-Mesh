# Design: Real GPU Execution Wiring

## 1. ONNX GPU execution

### Current state
`CandleOnnxBackend` decodes `ModelProto` and builds a `CandleOnnxGraph` over `candle_onnx::simple_eval`, which today only supports `Device::Cpu` reliably for the operator set Tachyon needs, per upstream candle issue #3491.

### Plan
- Add a device-capability probe at graph construction time: if the model binding declares a GPU device AND the installed candle version's `simple_eval` is confirmed CUDA-safe for the model's operator set (tracked via an explicit allow-list of validated ONNX op types, not a blanket "try CUDA and hope"), construct tensors on `Device::Cuda(ordinal)` instead of `Device::Cpu`.
- If the upstream fix is not yet available for a given operator, the graph falls back to CPU explicitly and the response/telemetry records `executed_on: cpu (fallback: unsupported_op_on_gpu)` rather than silently succeeding with no indication.
- This keeps the existing CPU path as a correctness fallback (default for unknown/unvalidated op sets), only opting into GPU for an allow-listed, tested subset.

```rust
enum OnnxExecutionDevice { Cpu, Gpu { ordinal: u32 } }

struct OnnxOpSupport {
    /// Operator types validated to execute correctly on Device::Cuda with the vendored candle-onnx.
    cuda_validated_ops: HashSet<&'static str>,
}
```

## 2. NVFP4 native forward pass

### Current state
`modelopt-nvfp4-kernels` already specifies: typed packed-FP4 representation, a correctness-first BF16/F32 fallback dequantizer, capability gating (`nvfp4-cuda` feature), and a CUDA/CUTLASS backend that compiles dequant/matmul kernels when that feature and toolchain are present. None of this is invoked from the actual inference call path — `ai-inference`'s existing requirement "AI inference bindings MUST classify ModelOpt/NVFP4 directories without mock execution" deliberately returns an unsupported-execution error for every NVFP4 alias today.

### Plan
- Implement `Nvfp4Linear::forward(&self, x: &Tensor) -> Result<Tensor>` that:
  1. Queries accelerator capability (existing "Native FP4 acceleration MUST be capability-gated" requirement) to decide kernel path.
  2. If native FP4 capability + kernels available: dispatches packed weights and the input activation directly to the compiled CUDA/CUTLASS matmul kernel (no eager dequantization).
  3. Else if BF16/F32 fallback fits within configured memory limits: dequantizes the shard via the existing fallback dequantizer, then runs a standard candle matmul on GPU.
  4. Else: returns the existing typed unsupported-execution error (today's only behavior becomes the last resort, not the only outcome).
- Wire this into the model's forward graph wherever a ModelOpt/NVFP4 component set was classified at load time, replacing the current unconditional error return in the inference call path.

```rust
enum Nvfp4ExecutionPath {
    NativeFp4,
    Fp32Fallback,
    UnsupportedExecution(UnsupportedExecutionError),
}

fn select_execution_path(caps: &AcceleratorCapabilities, mem_budget: &MemoryBudget) -> Nvfp4ExecutionPath { /* ... */ }
```

## 3. Execution telemetry
Both paths report which `OnnxExecutionDevice` / `Nvfp4ExecutionPath` actually ran, attached to the existing inference response/telemetry pipeline (`compute-observability`), so dashboards can show "GPU-native", "GPU-fallback", or "CPU" per request instead of requiring a source read to know.

## 4. Compatibility
- Existing CPU-only ONNX behavior remains the default and only fallback when an operator isn't allow-listed for CUDA — no regression for unsupported ops.
- Existing "unsupported-execution error" for NVFP4 remains reachable (now genuinely "unsupported" rather than "always"), satisfying the existing `ai-inference` requirement that NVFP4 aliases never return mock output.
