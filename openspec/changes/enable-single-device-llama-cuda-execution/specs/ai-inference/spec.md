## MODIFIED Requirements

### Requirement: GPU execution MUST be served when the candle CUDA backend is compiled in, and refused with a typed error otherwise
The runtime SHALL accept a GPU `device` request only on a build where the candle CUDA backend is compiled in. On a build without the CUDA backend, a GPU request SHALL continue to return the existing typed unsupported-execution error, and parallel engines SHALL run on CPU device stand-ins. On a build with the CUDA backend compiled in, a GPU request on the `single` path SHALL construct and execute on a real CUDA device for a Llama-family checkpoint; every other architecture on the `single` path SHALL continue to return the existing typed unsupported-execution error until it receives the same treatment.

#### Scenario: GPU request on a CUDA-less build is refused unchanged
- **GIVEN** a build without the `candle-cuda` feature
- **WHEN** a binding requests a non-`cpu` device on the `single` path
- **THEN** `try_load` returns the existing `UnsupportedModel` error verbatim ("the Candle LLM runtime supports `cpu` execution only")

#### Scenario: GPU request for a non-Llama architecture on the single path is still refused on a CUDA build
- **GIVEN** a build with the `candle-cuda` feature
- **WHEN** a binding whose checkpoint architecture is not Llama requests a non-`cpu` device on the `single` path
- **THEN** `try_load` returns the existing `UnsupportedModel` error naming the CPU-only restriction
- **AND** no model weights are allocated on a GPU device

#### Scenario: A Llama binding executes on a real CUDA device on a CUDA build
- **GIVEN** a build with the `candle-cuda` feature on a host with a CUDA device
- **WHEN** a Llama-family model binding on the `single` path requests a non-`cpu` device
- **THEN** `try_load` succeeds and constructs the model's weights, KV cache, and generation tensors on a real `Device::Cuda` handle
- **AND** `generate(...)` runs a real autoregressive decode on that device and returns non-mocked output
- **AND** a build with the feature compiled in but no physical CUDA device present falls back to `Device::Cpu` the same way the existing tensor/pipeline/expert-parallel engines already do, rather than erroring

#### Scenario: Multi-GPU topology is enumerated on a CUDA build
- **GIVEN** a build with the `candle-cuda` feature on a host with more than one CUDA device
- **WHEN** `discover_cluster_topology()` runs
- **THEN** it enumerates every available CUDA device (the enumeration loop is live once the candle CUDA backend is compiled in)
- **AND** per-device free-VRAM telemetry (NVML) and the NCCL all-reduce are validated on the CUDA CI lane as hardware-gated follow-ups (see `tasks.md` Tasks 5–6); the CPU-staged summation remains the numerically-equivalent reduction on every non-CUDA build
