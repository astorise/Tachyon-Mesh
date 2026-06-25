## Why

NVFP4 checkpoints now execute through a bounded dense fallback, but native packed-FP4 CUDA/CUTLASS matmul and per-request execution-device telemetry remain separate production capabilities.

## What Changes

- Implement native packed NVFP4 dequant/matmul execution.
- Add `executed_on` telemetry for ONNX and NVFP4 requests.
- Prove native and fallback selection on the CUDA hardware runner.

## Capabilities

### Modified Capabilities

- `modelopt-nvfp4-kernels`
- `ai-inference`
