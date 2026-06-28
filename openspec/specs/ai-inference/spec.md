# ai-inference Specification

## Purpose
TBD - created by archiving change ai-inference-wasinn. Update Purpose after archive.
## Requirements
### Requirement: Host optionally exposes WASI-NN imports to legacy guests
The `core-host` runtime SHALL define an `ai-inference` Cargo feature that links the `wasi_ephemeral_nn` preview1 host functions for legacy WASI guests without changing the default host build. The feature SHALL use `candle-onnx` (pure Rust) as the ONNX inference backend, making `--features ai-inference` compatible with musl libc targets.

#### Scenario: Default host builds without AI inference
- **WHEN** a developer builds `core-host` without enabling `ai-inference`
- **THEN** the host compiles successfully without `wasmtime-wasi-nn` or `candle-onnx`
- **AND** the default release and container workflows remain unchanged

#### Scenario: AI inference build links WASI-NN via candle-onnx backend
- **WHEN** a developer builds `core-host` with `--features ai-inference`
- **THEN** the legacy preview1 linker registers the `wasi_ephemeral_nn` imports
- **AND** legacy guests can resolve the `wasi-nn` host functions at instantiation time
- **AND** the build succeeds on musl libc targets (Alpine) without native library dependencies

#### Scenario: ONNX model loaded from raw bytes via CandleOnnxBackend
- **WHEN** a legacy guest calls `graph_load` with raw ONNX model bytes and encoding `onnx`
- **THEN** the host decodes the bytes into a `ModelProto` via `prost`
- **AND** constructs a `CandleOnnxGraph` backed by candle-onnx's `simple_eval`
- **AND** returns a graph handle to the guest without touching the filesystem

### Requirement: AI guest reads sealed ONNX models and returns JSON inference output
The workspace SHALL include a `guest-ai` legacy guest that reads a JSON tensor request, loads an ONNX model from a sealed read-only `/models` directory, runs inference via `wasi-nn` using the candle-onnx backend, and returns the output tensor as JSON. Inference executes on CPU; GPU execution is deferred pending upstream candle fix (issue #3491).

#### Scenario: Valid request loads a sealed model and computes inference
- **WHEN** `/api/guest-ai` is sealed with a read-only volume mounted at `/models`
- **AND** the client sends a JSON request containing `shape`, `values`, and `output_len`
- **THEN** `guest-ai` loads the requested ONNX model from `/models`
- **AND** it calls `set_input`, `compute`, and `get_output` via WASI-NN witx
- **AND** the candle-onnx backend executes the model on CPU
- **AND** it returns a JSON response containing the output tensor values

#### Scenario: Invalid request body returns a JSON error payload
- **WHEN** the client sends malformed JSON or tensor dimensions that do not match the input values
- **THEN** `guest-ai` does not attempt inference
- **AND** it returns a JSON payload describing the validation error

### Requirement: Host configuration can bind named preloaded models for AI targets
The integrity manifest SHALL allow AI-capable targets to declare model aliases, storage paths, and
target devices so the host can preload model bindings before serving inference.

#### Scenario: A target declares a GPU-backed model binding
- **WHEN** a target configuration defines a model alias, model path, and device
- **THEN** the host loads that model binding into its runtime configuration for startup initialization

### Requirement: Inference requests are continuously batched by the host
The host SHALL run a continuous batching scheduler that admits compatible inference sequences into
an active set, advances each sequence through explicit `prefill` and `decode` phases, and chooses
the next compatible step without waiting for a fixed time window.

#### Scenario: Compatible inference requests are active together
- **WHEN** several compatible inference requests are active on the same accelerator
- **THEN** the scheduler groups their next matching phase into a shared prefill or decode step
- **AND** routes each generated response back to the correct caller

#### Scenario: New work is admitted while decode is in flight
- **WHEN** a new higher-QoS inference request arrives while another sequence is already active
- **THEN** the scheduler may admit the new request into the active set before the existing sequence completes
- **AND** the next eligible step is selected by QoS and compatibility rather than by the original arrival batch

### Requirement: WASI-NN calls are bridged through the batching scheduler
The Wasmtime host SHALL intercept `wasi-nn` compute calls, enqueue them with response channels,
and resume the guest only after the scheduler returns inference output.

#### Scenario: A guest invokes `wasi-nn` compute against a preloaded alias
- **WHEN** a guest module issues a `wasi-nn` compute request for a preloaded model alias
- **THEN** the host packages the inputs into an inference request
- **AND** submits it to the batching scheduler
- **AND** writes the resulting output back into guest memory before resuming execution

### Requirement: CI validates the optional AI inference build path
The repository SHALL build the `guest-ai` artifact in CI and validate that the optional
`core-host --features ai-inference` path still compiles.

#### Scenario: GitHub Actions checks the optional AI feature
- **WHEN** the main CI workflow runs on GitHub Actions
- **THEN** it builds `guest-ai` for `wasm32-wasip1`
- **AND** it runs `cargo check -p core-host --features ai-inference`
- **AND** it still builds the default `core-host` release artifact without the feature

### Requirement: Downstream Candle quantization kernels MUST be consumed through the pinned fork
The AI inference build SHALL consume Candle from the pinned `astorise/candle`
revision that includes the downstream GPTQ/Marlin, AWQ, and block-wise FP8
weight-quantization kernel work proposed upstream in `huggingface/candle#3650`.
This integration SHALL be represented as dependency selection in Tachyon rather
than by duplicating those Candle kernels in `core-host`.

#### Scenario: AI inference resolves the forked Candle crates
- **WHEN** the optional AI inference dependency graph is resolved
- **THEN** `candle-core`, `candle-nn`, `candle-onnx`, and `candle-transformers` come from `https://github.com/astorise/candle`
- **AND** Cargo.lock pins them to a single fork revision containing the downstream quantization work

#### Scenario: Default builds do not link Candle quantization code
- **WHEN** `core-host` is built without `--features ai-inference`
- **THEN** the optional Candle crates remain unlinked
- **AND** the default host build does not acquire the fork's quantization kernels

### Requirement: Wasm guests may request a LoRA adapter for an inference call
The Mesh SHALL extend the `wit/ai` Wasm Component Model definitions so that an inference call accepts an optional `adapter_id` parameter, allowing a guest to request that a tenant-specific LoRA adapter be applied to the shared foundation model for that single call.

#### Scenario: Guest requests an adapter that is locally available
- **WHEN** a Wasm guest invokes the inference interface with an `adapter_id`
- **AND** the corresponding `.safetensors` adapter exists in `system-faas-model-broker`
- **THEN** the host loads the adapter weights and applies them to the foundation model's execution graph
- **AND** the inference output reflects the adapter's behaviour
- **AND** guests that omit `adapter_id` continue to run against the unmodified foundation model

### Requirement: Candle engine hot-swaps adapter weights and bounds context-switching overhead
The `wasi-nn-candle` execution engine SHALL dynamically inject and remove `.safetensors` adapter matrices during inference and SHALL bound the rate of adapter context-switching so that the cost of switching between adapters cannot dominate end-to-end latency.

#### Scenario: Concurrent tenants alternate adapters without runaway switching
- **WHEN** multiple tenants issue back-to-back inference calls with different `adapter_id` values
- **THEN** the engine swaps adapter weights on the shared foundation model between calls
- **AND** the swap operation occurs without reloading the foundation model into VRAM
- **AND** the engine enforces the configured maximum adapter-switch rate to keep aggregate latency within target SLOs

### Requirement: Inference workloads MUST support declarative LoRA Multiplexing
The `system-faas-model-broker` SHALL allow the sharing of a single base model in VRAM across multiple tenants by dynamically loading LoRA (Low-Rank Adaptation) weights based on Layer 7 routing conditions defined in the GitOps configuration.

#### Scenario: Routing to a tenant-specific LoRA
- **GIVEN** a base model pinned in VRAM and a configured LoRA adapter for the "legal" domain
- **WHEN** an inference request arrives with the header `X-Tenant-Domain: legal`
- **THEN** the Candle engine hot-swaps the "legal" LoRA adapter into the computation graph
- **AND** processes the prompt without reloading the base model weights, achieving zero-overhead multi-tenancy.

### Requirement: Large Models MUST support declarative Tensor Parallelism
The orchestration configuration SHALL allow operators to define a `tensor_parallelism` strategy, forcing the underlying `wasi-nn` backend to partition model layers across multiple available GPUs to prevent OOM errors on large models.

#### Scenario: Partitioning a model across GPUs
- **GIVEN** an AI deployment configured with `tensor_parallelism`
- **WHEN** the model broker loads a model that exceeds a single GPU's available VRAM
- **THEN** the runtime partitions model layers across the configured GPU set
- **AND** rejects startup with a typed configuration error if the requested GPU topology is unavailable.

### Requirement: AI inference bindings MUST classify ModelOpt/NVFP4 directories without mock execution

The AI inference runtime SHALL load supported ModelOpt/NVFP4 model bindings as
typed component sets and SHALL NOT return mock inference output for those
aliases. When a registered architecture backend matches the checkpoint, the
runtime SHALL execute real inference; otherwise it SHALL return an actionable
unsupported-architecture error.

#### Scenario: Detected NVFP4 alias executes with a supported architecture

- **WHEN** a preloaded model alias is classified as ModelOpt/NVFP4
- **AND** a registered architecture backend validates its metadata and tensors
- **THEN** inference executes through that architecture backend
- **AND** the response is not `MOCK_LLM_RESPONSE`

#### Scenario: Detected NVFP4 alias refuses an unsupported architecture

- **WHEN** a preloaded model alias is classified as ModelOpt/NVFP4
- **AND** no registered architecture backend accepts it
- **THEN** the runtime returns an actionable unsupported-architecture error
- **AND** the response is not `MOCK_LLM_RESPONSE`

#### Scenario: Existing ONNX guest path remains available

- **WHEN** a legacy guest loads an ONNX model through WASI-NN
- **THEN** the host continues to use the candle-onnx backend
- **AND** ModelOpt/NVFP4 loading does not change the ONNX graph encoding contract

### Requirement: Unsupported quantized model bindings MUST fail with actionable errors
The AI inference runtime SHALL reject unsupported quantized model bindings with typed errors before returning mock inference output.

#### Scenario: Unsupported ModelOpt layout is configured
- **WHEN** a model binding points at a ModelOpt/NVFP4 checkpoint whose tensor names, scale layout, or architecture are not supported
- **THEN** model initialization fails with a typed error containing the model alias, model path, and unsupported layout detail
- **AND** inference for that alias is not registered

#### Scenario: Non-NVFP4 model remains outside the NVFP4 loader
- **WHEN** a model binding points at a safetensors directory without NVFP4 metadata or NVFP4 scale tensors
- **THEN** the ModelOpt/NVFP4 loader does not claim the binding
- **AND** the host either routes the model to another supported backend or returns an unsupported-model error

### Requirement: Candle LLM bindings MUST generate real model output
The AI inference runtime SHALL execute supported local Candle text-generation model bindings by loading their tokenizer, config, and safetensors weights, and SHALL return generated UTF-8 text bytes instead of mock inference output.

#### Scenario: Supported Candle LLM binding returns generated text
- **WHEN** a model binding points at a supported local Candle LLM directory
- **AND** a guest or host caller submits a UTF-8 prompt as the first `U8` input tensor
- **THEN** the runtime loads the model tokenizer, config, and safetensors weights
- **AND** executes bounded text generation through Candle
- **AND** returns UTF-8 generated text bytes that are not `MOCK_LLM_RESPONSE`

#### Scenario: Supported Candle LLM binding accepts a bounded JSON request
- **WHEN** a model binding points at a supported local Candle LLM directory
- **AND** the first `U8` input tensor is a JSON generation request with `prompt` and optional generation parameters
- **THEN** the runtime validates the request against configured prompt and generation limits
- **AND** returns UTF-8 generated text bytes through the existing inference response path

### Requirement: Non-mock model bindings MUST NOT fall back to mock output
The AI inference runtime SHALL classify model bindings as explicit mock, ModelOpt/NVFP4, supported Candle LLM, ONNX/WASI-NN, or unsupported, and SHALL NOT return `MOCK_LLM_RESPONSE` for any non-mock binding.

#### Scenario: Unsupported safetensors directory fails before registration
- **WHEN** a model binding points at a safetensors directory that is neither ModelOpt/NVFP4 nor a supported Candle LLM
- **THEN** model initialization fails with a typed unsupported-model error containing the alias, path, and unsupported reason
- **AND** inference for that alias is not registered

#### Scenario: Runtime load failure does not use mock output
- **WHEN** a supported Candle LLM binding has invalid tokenizer, config, or weight files
- **THEN** model initialization fails with a typed load error containing the alias, path, and invalid component
- **AND** the runtime does not register a mock model for the alias

#### Scenario: Explicit mock binding preserves test behavior
- **WHEN** a test or fixture configures an explicit mock model binding
- **THEN** the runtime may return `MOCK_LLM_RESPONSE`
- **AND** the mock path remains distinguishable from supported Candle LLM bindings

### Requirement: Candle LLM generation MUST be bounded and deterministic by default
The Candle LLM runtime SHALL enforce prompt length, max-new-token, batch size, and sampling limits, and SHALL use deterministic generation defaults suitable for repeatable tests.

#### Scenario: Prompt exceeds configured limit
- **WHEN** a caller submits a prompt that exceeds the configured prompt token or byte limit
- **THEN** the runtime rejects the request with a typed validation error
- **AND** no generation work is executed

#### Scenario: Generation request omits sampling parameters
- **WHEN** a caller submits a plain UTF-8 prompt or a JSON request without sampling parameters
- **THEN** the runtime uses deterministic defaults for token selection
- **AND** repeated runs against the deterministic fixture produce the expected non-mock output

#### Scenario: Requested generation limit exceeds host cap
- **WHEN** a JSON generation request asks for more new tokens than the configured host cap
- **THEN** the runtime rejects or clamps the request according to the configured policy
- **AND** the behavior is reported in the response or error path

### Requirement: Existing ONNX and NVFP4 boundaries MUST remain unchanged

Adding a new ModelOpt/NVFP4 architecture runtime SHALL NOT change legacy Candle
ONNX/WASI-NN graph loading. NVFP4 checkpoints SHALL execute only when an
explicit architecture backend validates their metadata and tensor contract;
all other NVFP4 checkpoints SHALL preserve the non-mock unsupported boundary.

#### Scenario: Legacy ONNX guest still uses candle-onnx

- **WHEN** a legacy guest loads an ONNX model through WASI-NN
- **THEN** the host continues to use the candle-onnx backend
- **AND** architecture backend selection does not change the ONNX graph
  encoding contract

#### Scenario: Supported ModelOpt/NVFP4 alias generates text

- **WHEN** a preloaded ModelOpt/NVFP4 alias matches a registered text-generation
  architecture backend
- **THEN** buffered and streaming inference execute real model generation
- **AND** the response is not `MOCK_LLM_RESPONSE`

#### Scenario: Unsupported ModelOpt/NVFP4 alias remains non-mock

- **WHEN** a preloaded model alias is classified as ModelOpt/NVFP4
- **AND** no architecture backend is configured for that alias
- **THEN** inference returns an actionable unsupported-execution error
- **AND** the response is not `MOCK_LLM_RESPONSE`

### Requirement: Real Candle LLM validation MUST run without network downloads
The repository SHALL include deterministic tests for real Candle LLM loading and generation that do not download external model artifacts during CI.

#### Scenario: CI validates real Candle generation
- **WHEN** the CI workflow runs the optional `core-host --features ai-inference` checks
- **THEN** it executes a deterministic real Candle LLM fixture test
- **AND** the fixture output is generated by Candle rather than by a mock backend
- **AND** the test does not require network access or Hugging Face downloads

#### Scenario: Optional real checkpoint probe is gated
- **WHEN** a developer sets an environment variable pointing at a local supported checkpoint directory
- **THEN** the test suite may run an additional real-checkpoint load and generation probe
- **AND** CI remains independent of that local checkpoint

### Requirement: Candle LLM generation supports configurable sampling
The Candle LLM runtime SHALL select tokens according to per-request sampling
parameters — `temperature`, `top_p` (nucleus), and `seed` — while remaining
deterministic by default. A `temperature` that is absent or `<= 0` SHALL produce
deterministic greedy decoding independent of the seed. A sampled request that
omits `seed` SHALL fall back to a fixed default seed so it stays reproducible.

#### Scenario: Greedy by default
- **WHEN** a generation request omits `temperature` or sets it to `0`
- **THEN** the runtime decodes greedily (argmax)
- **AND** two runs of the same prompt produce identical output

#### Scenario: Seeded sampling is reproducible
- **WHEN** a generation request sets `temperature > 0` and a fixed `seed`
- **THEN** the runtime samples from the temperature-scaled distribution
- **AND** two runs of the same prompt and seed produce identical output

#### Scenario: Nucleus filtering is bounded
- **WHEN** a sampled request sets `top_p` inside the open interval `(0, 1)`
- **THEN** the runtime restricts sampling to the smallest set of tokens whose
  cumulative probability reaches `top_p`
- **AND** a `top_p` of `1.0` (or absent) disables nucleus filtering

### Requirement: Chat requests render the model's own chat template
When a generation request carries structured `messages`, the runtime SHALL
render them into the prompt using the checkpoint's own `chat_template` loaded
from `tokenizer_config.json`, including the special tokens it references and an
appended generation prompt. When the checkpoint ships no template, the runtime
SHALL fall back to a generic rendering that ends on an open assistant turn. A
request MAY instead supply a raw `prompt`, which is used verbatim.

#### Scenario: Model template drives rendering
- **GIVEN** a checkpoint whose `tokenizer_config.json` declares a `chat_template`
- **WHEN** a request supplies structured `messages`
- **THEN** the runtime renders the conversation with that template and its
  special tokens, ready for the assistant to continue

#### Scenario: Generic fallback without a template
- **GIVEN** a checkpoint with no `chat_template`
- **WHEN** a request supplies structured `messages`
- **THEN** the runtime renders a generic `role: content` prompt ending on an
  open assistant turn
- **AND** generation still runs

#### Scenario: Raw prompt bypasses templating
- **WHEN** a request supplies a raw `prompt` and no `messages`
- **THEN** the runtime tokenizes the prompt verbatim

### Requirement: Generation honours stop sequences
The runtime SHALL accept a bounded list of `stop` strings and halt generation as
soon as any of them appears in the decoded text, returning the text up to (and
excluding) the earliest match. An empty or oversized stop entry SHALL be ignored.

#### Scenario: Output is trimmed at the earliest stop
- **WHEN** a request sets one or more `stop` strings
- **AND** the decoded text reaches a stop sequence
- **THEN** generation halts
- **AND** the returned text excludes the stop sequence and anything after it

#### Scenario: Stop list is bounded
- **WHEN** a request supplies empty, oversized, or excessively many stop strings
- **THEN** the runtime filters and caps the list before decoding

### Requirement: The runtime streams decoded fragments incrementally

The runtime SHALL provide a streaming generation path that emits each newly
decoded text fragment as it is produced, such that the concatenation of all
fragments equals the buffered generation output for the same request. While
streaming with stop sequences, the runtime SHALL hold back the trailing text
that could begin a stop match until a further token confirms it is safe to emit.

#### Scenario: Streamed fragments reconstruct the buffered output

- **WHEN** the same request is run buffered and streamed
- **THEN** the streamed path emits one or more fragments
- **AND** their concatenation equals the buffered output byte-for-byte

#### Scenario: Non-generative backends fall back to a single fragment

- **WHEN** a streaming request targets a backend that cannot decode
  incrementally, such as an explicit mock backend
- **THEN** the runtime emits the entire output as one fragment

#### Scenario: Supported NVFP4 architecture streams tokens

- **WHEN** a streaming request targets a ModelOpt/NVFP4 checkpoint with a
  registered autoregressive architecture backend
- **THEN** decoded text fragments are emitted incrementally as tokens are
  generated

### Requirement: The accelerator exposes a streaming compute primitive
The `tachyon:accelerator/cpu` interface SHALL provide a `compute-stream`
function returning a `token-stream` resource whose `next` yields decoded text
fragments until it returns `none` (generation complete). The streaming path
SHALL enforce the same sealed-alias scope and accelerator-handle checks as the
buffered `compute`, and the new function SHALL be additive so existing guests
are unaffected.

#### Scenario: Streaming a sealed model
- **GIVEN** a guest holding an accelerator handle for a sealed model alias
- **WHEN** it calls `compute-stream` and pulls with `next`
- **THEN** the host yields decoded fragments as they are produced
- **AND** `next` returns `none` once generation completes

#### Scenario: Streaming respects the scope gate
- **WHEN** a guest calls `compute-stream` for a handle it does not hold, or for
  an alias not sealed for its route
- **THEN** the host rejects the call with an error, exactly as `compute` does

### Requirement: The runtime MUST execute tensor-parallel inference across multiple GPUs
When a model deployment is configured with `hardware_strategy.distribution_mode: tensor_parallelism` and `multi_gpu: true`, the inference runtime SHALL shard transformer layer weights across the configured GPU set and SHALL synchronize partial results between shards on every layer that requires it using a real collective-communication primitive on CUDA hardware, falling back to a host-staged reduction only when no CUDA backend with multiple participating GPUs is available.

#### Scenario: A model exceeding single-GPU VRAM is sharded across GPUs
- **GIVEN** a model deployment configured with `distribution_mode: tensor_parallelism` and a GPU set whose combined VRAM, but not any single member's VRAM, can hold the model
- **WHEN** the model broker loads the model
- **THEN** the runtime partitions attention and MLP weights across the configured GPUs
- **AND** synchronizes partial activations across shards via an all-reduce/all-gather step per transformer block
- **AND** produces output numerically equivalent (within floating-point tolerance) to a single-GPU reference run of the same model on hardware where that reference fits

#### Scenario: Single-GPU deployments are unaffected
- **WHEN** a model deployment is configured with `distribution_mode: single` or `multi_gpu: false`
- **THEN** the runtime executes the existing single-device path unchanged
- **AND** no tensor-parallel synchronization code path is invoked

#### Scenario: A real NCCL collective performs the all-reduce on CUDA hardware
- **GIVEN** the runtime is built with the `candle-cuda` feature and a tensor-parallel shard group spans 2 or more CUDA devices
- **WHEN** `RowParallelLinear::forward` synchronizes partial outputs across the shard group
- **THEN** the runtime issues a real NCCL `AllReduce` collective across the participating devices' communicators
- **AND** the reduced result matches the existing host-staged-sum reference within `1e-4` tolerance

#### Scenario: The host-staged fallback remains correct when no multi-GPU CUDA group is available
- **GIVEN** the runtime is built without the `candle-cuda` feature, or a shard group has fewer than 2 CUDA devices, or the shard group runs on `Device::Cpu`
- **WHEN** `RowParallelLinear::forward` synchronizes partial outputs
- **THEN** the runtime performs the existing host-staged manual sum across devices
- **AND** the result is unchanged from the pre-existing behavior, with no regression in any CPU-only test

### Requirement: The runtime MUST execute pipeline-parallel inference across multiple nodes
When a model deployment is configured with `hardware_strategy.distribution_mode: pipeline_parallelism`, the runtime SHALL assign contiguous layer ranges to distinct nodes/GPUs, SHALL stream activations between pipeline stages over a point-to-point transport implementing `StageTransport`, and SHALL support full autoregressive generation (prefill followed by an arbitrary number of decode steps), not prefill alone.

#### Scenario: Layers are split across pipeline stages
- **GIVEN** a model deployment configured with `distribution_mode: pipeline_parallelism` across N nodes
- **WHEN** the model broker loads the model
- **THEN** each node is assigned a contiguous, non-overlapping range of layers
- **AND** each node executes its layer range with a real transformer-block forward pass

#### Scenario: A pipeline-parallel deployment generates more than one token
- **GIVEN** a model deployment configured with `distribution_mode: pipeline_parallelism` and successfully loaded
- **WHEN** a generation request is submitted with `max_tokens > 1`
- **THEN** the runtime completes an initial prefill pass across all stages
- **AND** completes a decode pass for each subsequent token, each stage reusing a persistent per-stage KV cache rather than rebuilding it from scratch
- **AND** the final output is numerically equivalent (within floating-point tolerance) to a dense single-device reference run of the same model and prompt for the same number of tokens

#### Scenario: Pipeline depth bounds in-flight micro-batches
- **GIVEN** a pipeline-parallel deployment with a configured pipeline depth
- **WHEN** multiple inference requests are in flight concurrently
- **THEN** the scheduler admits at most the configured number of micro-batches into the pipeline at once
- **AND** additional requests queue rather than unboundedly growing per-stage memory usage

### Requirement: The runtime MUST execute expert-parallel inference for Mixture-of-Experts checkpoints
For checkpoints declaring expert tensors (e.g. Mixtral-style `model_type: mixtral` checkpoints), the runtime SHALL load the checkpoint, partition experts across the configured GPU/node set, and SHALL route each token only to the device(s) hosting its selected expert, rather than rejecting expert-parallel deployments outright or replicating all experts on every device.

#### Scenario: An MoE checkpoint is loaded and partitioned across devices
- **GIVEN** a model deployment configured with `distribution_mode: expert_parallelism` and a checkpoint whose `config.json` declares `model_type: mixtral`
- **WHEN** the model broker loads the model
- **THEN** the runtime parses the checkpoint's MoE-specific config fields (`num_local_experts`, `num_experts_per_tok`)
- **AND** partitions experts across the configured device set per the deployment's `hardware_strategy.expert_device_map`, falling back to an even round-robin placement for any expert the map does not explicitly pin
- **AND** does not load a full replica of every expert onto every device

#### Scenario: Mixed dense and MoE layers in the same checkpoint load correctly
- **GIVEN** a checkpoint where some transformer layers declare expert tensors and others do not
- **WHEN** the model broker loads the model
- **THEN** layers without expert tensors execute the existing dense MLP path unchanged
- **AND** layers with expert tensors execute the expert-parallel routed path
- **AND** both layer kinds share the same attention and KV-cache machinery within one forward pass

#### Scenario: Tokens are routed only to their selected expert's device
- **WHEN** the gate layer selects the top-1 expert for a token
- **THEN** the runtime forwards that token's hidden state only to the device hosting the selected expert
- **AND** non-MoE checkpoints continue to execute the existing dense path unchanged

#### Scenario: An MoE deployment generates more than one token
- **GIVEN** a successfully loaded expert-parallel deployment
- **WHEN** a generation request is submitted with `max_tokens > 1`
- **THEN** the runtime completes an initial prefill pass followed by per-token decode steps
- **AND** the KV cache persists correctly across decode steps for both dense and MoE layers

#### Scenario: Top-k greater than one is rejected at load time
- **GIVEN** a checkpoint whose config declares `num_experts_per_tok > 1`
- **WHEN** the model broker attempts to load it under `distribution_mode: expert_parallelism`
- **THEN** the runtime rejects the deployment with a typed `UnsupportedModel` error
- **AND** does not silently truncate routing to top-1

### Requirement: Parallel execution plans MUST be validated against discovered hardware topology before deployment
The runtime SHALL reject, with a typed topology error, any `tensor_parallelism`, `pipeline_parallelism`, or `expert_parallelism` deployment whose GPU/node count, interconnect class, or per-shard VRAM requirement cannot be satisfied by the cluster's discovered hardware topology. On CUDA builds, per-device free VRAM SHALL be sourced from real NVML telemetry rather than a hardcoded placeholder value, so the VRAM check can actually reject an oversized deployment in production.

#### Scenario: Insufficient GPU count is rejected at deploy time
- **WHEN** a deployment requests `tensor_parallelism` across more GPUs than are available on the target node
- **THEN** `apply-model-deployment` fails with a typed `InsufficientDeviceCount` error
- **AND** no partial model load is attempted

#### Scenario: Incompatible interconnect is rejected at deploy time
- **WHEN** a deployment requests `tensor_parallelism` across GPUs that lack the required high-bandwidth interconnect
- **THEN** `apply-model-deployment` fails with a typed `IncompatibleInterconnect` error

#### Scenario: Per-shard VRAM overrun is rejected at deploy time using real telemetry
- **GIVEN** the runtime is built with the `candle-cuda` feature and NVML successfully reports each CUDA device's free VRAM
- **WHEN** a deployment's computed per-shard VRAM requirement exceeds any target GPU's NVML-reported free VRAM
- **THEN** `apply-model-deployment` fails with a typed `VramPerShardExceeded` error
- **AND** the runtime does not silently downgrade to a single-GPU execution plan

#### Scenario: VRAM telemetry degrades gracefully when NVML is unavailable
- **GIVEN** NVML initialization fails (no NVIDIA driver, insufficient permissions, or a non-NVIDIA host) or the `candle-cuda` feature is not compiled in
- **WHEN** the runtime discovers cluster topology
- **THEN** every device reports `free_vram_bytes: 0` ("unknown"), matching the existing pre-NVML behavior
- **AND** `validate_parallel_topology` never rejects a deployment on VRAM grounds for a device reporting `0`

### Requirement: CUDA CI MUST prove multi-GPU collective execution, not just compilation
The `cuda-quality` CI job (or an equivalent job on the same GPU-equipped self-hosted runner) SHALL execute a test that exercises a real NCCL all-reduce on real CUDA hardware and asserts its numeric result against a known-correct reference, in addition to the existing `cargo check`/`cargo clippy --features candle-cuda` compilation/lint steps.

#### Scenario: GPU CI runs and passes a real NCCL all-reduce test
- **GIVEN** the `cuda-quality` job runs on the `arc-gpu-runners` self-hosted runner with a real GPU detected via `nvidia-smi`
- **WHEN** the job executes its test step
- **THEN** a test exercising `ncclAllReduce` across multiple ranks runs to completion
- **AND** its result matches the existing CPU-staged-sum reference within `1e-4` tolerance
- **AND** the job's overall conclusion is `success` only if that test passes, not merely if `cargo clippy` finds no lint errors

#### Scenario: The NCCL test runs correctly on a single-physical-GPU runner
- **GIVEN** the runner exposes exactly one physical CUDA device (the verified case for the current `arc-gpu-runners` configuration)
- **WHEN** the NCCL all-reduce test runs
- **THEN** it uses multiple NCCL ranks on that single device (loopback communicator initialization) rather than requiring a second physical GPU
- **AND** the test is skipped, not failed, on a `candle-cuda` build executed on a host reporting zero CUDA devices

### Requirement: The `nvfp4-cuda` and `candle-cuda` Cargo features MUST be documented as independent
Inline documentation describing the relationship between the `nvfp4-cuda` and `candle-cuda` Cargo features SHALL accurately reflect that they are separate, sibling features — enabling one does not enable the other — matching `core-host/Cargo.toml`'s actual feature graph.

#### Scenario: The topology-discovery comment accurately describes feature independence
- **GIVEN** a reader inspects the comment above `discover_cluster_topology`'s CUDA-enumeration loop in `core-host/src/ai_inference/parallel.rs`
- **WHEN** they read the comment to understand what enables multi-GPU enumeration
- **THEN** the comment states that the `candle-cuda` feature, not `nvfp4-cuda`, is required
- **AND** the comment does not claim `nvfp4-cuda` pulls in or implies `candle-cuda`

### Requirement: Route-scoped dynamic model bindings

The integrity manifest SHALL support dynamic model bindings whose model content
is resolved from the managed broker model directory at runtime. A dynamic
binding SHALL authorize only the route that declares it and SHALL NOT require a
static model path.

#### Scenario: Dynamic binding omits static path

- **WHEN** integrity configuration normalization receives a model binding with
  `dynamic: true` and an empty path
- **THEN** normalization SHALL preserve the dynamic flag and accept the binding

#### Scenario: Static binding still requires path

- **WHEN** integrity configuration normalization receives a static model
  binding with an empty path
- **THEN** normalization SHALL reject the binding

#### Scenario: Dynamic authorization remains route-scoped

- **GIVEN** a dynamic alias is bound to the OpenAI chat route
- **WHEN** another route attempts to load the same alias without declaring it
- **THEN** the host SHALL reject that route's request as not sealed

### Requirement: A model deployment's `hardware_strategy` MUST select the parallel execution engine at load time
When a model deployment declares `hardware_strategy.distribution_mode` other than `single`, the runtime SHALL carry that strategy from configuration into model loading and SHALL construct the corresponding parallel engine (tensor-, pipeline-, or expert-parallel) instead of the dense single-device path. A `single` (or absent) strategy SHALL load the existing single-device path with no behavioural change.

#### Scenario: A tensor-parallel deployment is dispatched to the tensor-parallel engine
- **GIVEN** a model binding whose `hardware_strategy.distribution_mode` is `tensor_parallelism` with two device IDs
- **WHEN** the runtime loads the model
- **THEN** the binding's strategy is threaded into `try_load`
- **AND** the model is loaded as a tensor-parallel engine across the configured devices
- **AND** generation produces output numerically equivalent (within floating-point tolerance) to the dense single-device path on the same checkpoint

#### Scenario: A pipeline-parallel deployment is dispatched to the pipeline engine
- **GIVEN** a model binding whose `distribution_mode` is `pipeline_parallelism` with contiguous `stage_layer_ranges`
- **WHEN** the runtime loads the model
- **THEN** the model is loaded as a pipeline-parallel engine with the configured stage ranges
- **AND** a prefill request returns prompt logits equivalent to the dense reference
- **AND** a token-streaming (decode) request returns a typed "decode not yet supported for pipeline parallelism" error rather than incorrect output

#### Scenario: An expert-parallel deployment is validated but refused until a MoE loader exists
- **GIVEN** a model binding whose `distribution_mode` is `expert_parallelism`
- **WHEN** the runtime loads the model
- **THEN** the plan is validated against the discovered hardware topology
- **AND** the load returns a typed error indicating that a full MoE checkpoint loader is not yet implemented (only the numerically-verified per-expert `ExpertParallelMlp` primitive exists), rather than constructing a non-existent full MoE model or silently downgrading to the dense path

#### Scenario: Single-device deployments are byte-for-byte unaffected
- **WHEN** a model binding declares `distribution_mode: single` or carries no `hardware_strategy`
- **THEN** the existing `Safetensors`/`Gguf` single-device load path executes unchanged
- **AND** no parallel dispatch, topology discovery, or strategy plumbing is invoked

### Requirement: PagedAttention MUST require an explicit block-table runtime path
When a model deployment sets `hardware_strategy.paged_attention: true`, the runtime SHALL NOT silently fall back to the existing contiguous per-request KV cache. Tachyon SHALL enable this mode only after its core-host runtime owns a block allocator, a per-sequence block table, and a Candle paged flash-attn call using `flash_attn_varlen_paged_windowed` or a compatible successor API.

#### Scenario: PagedAttention request is rejected before Tachyon block tables are wired
- **GIVEN** a model binding sets `hardware_strategy.paged_attention: true`
- **WHEN** the Candle LLM runtime loads the binding
- **THEN** loading fails with a typed `UnsupportedModel` error naming the missing block allocator and block-table integration
- **AND** the runtime does not execute the contiguous KV-cache path as a fallback

#### Scenario: PagedAttention can be enabled only after block-paged KV integration
- **GIVEN** the runtime has a CUDA block pool, per-sequence block tables, context lengths, and a paged K/V tensor layout compatible with Candle's paged flash-attn API
- **WHEN** a deployment enables `hardware_strategy.paged_attention`
- **THEN** decode uses the block-paged attention path
- **AND** sequence admission and eviction operate at block granularity rather than reallocating a contiguous KV cache per request

### Requirement: The runtime MUST validate a parallel plan against discovered hardware before loading weights
Before constructing any parallel engine, the runtime SHALL validate the requested plan against the cluster's discovered hardware topology (device count, interconnect class, per-shard VRAM) and SHALL abort the load with a typed topology error — loading no weights — when the plan cannot be satisfied. This hardware-aware check is in addition to the structural plan validation already performed by the config API.

#### Scenario: A plan requesting more devices than exist is rejected before any load
- **GIVEN** a binding requesting a parallel plan across more devices than `discover_cluster_topology()` reports
- **WHEN** the runtime attempts to load the model
- **THEN** `try_load` fails with a typed topology error mapped from `TopologyError::InsufficientDeviceCount`
- **AND** no model weights are allocated

### Requirement: GPU execution MUST be served when the candle CUDA backend is compiled in, and refused with a typed error otherwise
The runtime SHALL accept a GPU `device` request only on a build where the candle CUDA backend is compiled in. On a build without the CUDA backend, a GPU request SHALL continue to return the existing typed unsupported-execution error, and parallel engines SHALL run on CPU device stand-ins.

#### Scenario: GPU request on a CUDA-less build is refused unchanged
- **GIVEN** a build without the `candle-cuda` feature
- **WHEN** a binding requests a non-`cpu` device on the `single` path
- **THEN** `try_load` returns the existing `UnsupportedModel` error verbatim ("the Candle LLM runtime supports `cpu` execution only")

#### Scenario: Multi-GPU topology is enumerated on a CUDA build
- **GIVEN** a build with the `candle-cuda` feature on a host with more than one CUDA device
- **WHEN** `discover_cluster_topology()` runs
- **THEN** it enumerates every available CUDA device (the enumeration loop is live once the candle CUDA backend is compiled in)
- **AND** per-device free-VRAM telemetry (NVML) and the NCCL all-reduce are validated on the CUDA CI lane as hardware-gated follow-ups (see `tasks.md` Tasks 5–6); the CPU-staged summation remains the numerically-equivalent reduction on every non-CUDA build

### Requirement: Native Candle inference MUST dispatch supported non-Llama architectures

The native Candle text-generation path SHALL dispatch supported non-Llama
safetensors and GGUF checkpoints through registered architecture-specific
backends while preserving the existing sealed-alias authorization, request
schema, scheduler, sampling, constrained decoding, stop, buffered, and streaming
behavior. Existing Llama, Mixtral, Qwen 3.5 MoE, ModelOpt/NVFP4, ONNX, and mock
backend boundaries SHALL remain unchanged.

#### Scenario: Supported non-Llama alias uses native Candle execution

- **WHEN** a route-authorized model alias resolves to a checkpoint whose
  architecture and format have a registered backend
- **THEN** the native Candle runtime executes that backend
- **AND** the response is not `MOCK_LLM_RESPONSE`

#### Scenario: Unsupported architecture remains actionable and non-mock

- **WHEN** a route-authorized alias resolves to an architecture or format
  combination without a registered backend
- **THEN** the runtime returns an actionable typed unsupported-model error
- **AND** does not return mock inference output

#### Scenario: Existing Llama behavior is unchanged

- **WHEN** a Llama safetensors or supported Llama GGUF checkpoint is loaded
- **THEN** its existing single-device generation path and outputs remain
  compatible with the pre-registry behavior

#### Scenario: Existing specialized runtimes remain independently dispatched

- **WHEN** a checkpoint matches the Qwen 3.5 MoE ModelOpt/NVFP4 contract, the
  Mixtral expert-parallel contract, or the legacy ONNX contract
- **THEN** the existing specialized dispatcher handles it
- **AND** the generic architecture registry does not reinterpret it as a dense
  Qwen, Gemma, Phi, or DeepSeek checkpoint

