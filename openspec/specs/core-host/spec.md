# core-host Specification

## Purpose
Core Tachyon Mesh host runtime — manifest schema exposure, routing, and admin API surface.
## Requirements
### Requirement: Manifest schema generation MUST be supported
The core-host SHALL add the `schemars` crate to `core-host/Cargo.toml`, derive `JsonSchema` on `Manifest` and all its composite types (`FunctionManifest`, `ResourceLimit`, etc.), expose `GET /admin/schema/manifest`, and actively route `POST /admin/manifest/dryrun` to the `system-faas-config-api` component rather than evaluating dry runs natively by the host.

```rust
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Debug)]
pub struct Manifest {
    pub api_version: String,
    pub kind: String,
    pub metadata: Metadata,
    pub spec: ManifestSpec,
}
```

```rust
// In router definition
let schema = schemars::schema_for!(Manifest);
serde_json::to_string(&schema)
```

#### Scenario: Manifest schema and dry-run routes are available
- **WHEN** an operator requests `GET /admin/schema/manifest`
- **THEN** core-host returns the generated manifest JSON Schema
- **AND** requests to `POST /admin/manifest/dryrun` are routed to `system-faas-config-api`

### Requirement: The host provides an incremental body-flush streaming transport

When a request carries `Accept: text/event-stream`, the host SHALL execute the
FaaS guest on a dedicated thread with a `tachyon:mesh/response-body` streaming
sink pre-installed in the execution context. The guest acquires the sink via
`get-streaming-response`, commits status and headers with `begin`, and flushes
body bytes with `write`; each write is forwarded to the client immediately via an
axum `Body::from_stream` backed by a bounded channel, so the client receives
headers and first bytes before generation completes. When the guest never calls
`begin`, the execution SHALL fall back to a buffered response whose headers and
body are forwarded through the same channel pair after `handle-request` returns,
so a non-streaming guest under a streaming request still responds correctly. The
transport SHALL acquire the route's volume leases and concurrency permit exactly
as the buffered path does, and SHALL wire the same scope-gated interfaces into
the streaming linker (including `kv-partition` and `graph` under the `kv` scope).

#### Scenario: Streaming response flushes chunks in real time

- **GIVEN** a guest that calls `get-streaming-response` and `begin`
- **WHEN** the guest writes body chunks via `write`
- **THEN** each chunk is forwarded to the connected HTTP client immediately
- **AND** the client receives the committed headers before any body bytes

#### Scenario: Buffered fallback under a streaming request

- **GIVEN** a request carrying `Accept: text/event-stream`
- **WHEN** the guest returns a buffered `handler::response` without calling `begin`
- **THEN** the host forwards the buffered status, headers, and body through the
  same channel pair
- **AND** the awaiting transport never blocks waiting for headers

#### Scenario: Streaming linker matches the buffered authorization model

- **WHEN** the streaming execution path builds its component linker
- **THEN** it wires the same interfaces as the buffered path, each gated on the
  route's deployment scope (e.g. `kv-partition` and `graph` only when the route
  grants `kv`)

## Requirements: OpenAPI Documentation Endpoints

### Requirement: core-host MUST expose an utoipa-generated OpenAPI 3.1 schema
The core-host admin API SHALL expose `GET /admin/schema/openapi.json` returning an OpenAPI 3.1 JSON document generated at compile time via the `utoipa` crate. An `ApiDoc` struct SHALL annotate the top 10 most critical admin routes and key response types.

#### Scenario: Schema endpoint returns valid OpenAPI JSON
- **WHEN** an authenticated client sends `GET /admin/schema/openapi.json`
- **THEN** the response status is 200 with `content-type: application/json`
- **AND** the body is a valid OpenAPI 3.1 document containing paths for `/admin/manifest`, `/admin/iam/users`, and `/admin/metrics`

### Requirement: core-host MUST serve an interactive API documentation page
The core-host admin API SHALL expose `GET /admin/docs` returning a Swagger UI HTML page embedded at compile time via `include_str!`. No filesystem access shall occur at runtime to serve this page.

#### Scenario: Docs page is self-contained
- **WHEN** an authenticated client sends `GET /admin/docs`
- **THEN** the response status is 200 with `content-type: text/html; charset=utf-8`
- **AND** the HTML references `/admin/schema/openapi.json` as the schema URL

### Requirement: Cross-layer validation MUST assert OpenAPI contract routes exist
The `validate_cross_layer.sh` script SHALL assert that the four core OpenAPI contract routes (`/admin/schema/openapi.json`, `/admin/docs`, `/admin/manifest`, `/admin/iam/users`) are registered in `app_runtime.rs`.

#### Scenario: Validation fails when a contract route is removed
- **WHEN** one of the checked routes is removed from the Axum router
- **THEN** `validate_cross_layer.sh` exits with a non-zero status and names the missing route
### Requirement: OpenAPI schema MUST cover all ~35 admin routes
The `ApiDoc` struct SHALL declare `#[utoipa::path]` stubs for all admin routes including KV-Partition V2, canary management, shadow diffs, chaos scenarios, enrollment, security (MFA/PAT/step-up), full IAM CRUD, KV-cache, and asset/model upload. At least 35 operations SHALL appear in the generated OpenAPI document.

#### Scenario: OpenAPI schema includes broad admin coverage
- **WHEN** the generated OpenAPI document is inspected
- **THEN** it contains at least 35 operations covering the core admin API surface

### Requirement: `GET /admin/schema/integrity-lock` MUST return a JSON Schema for the lock file
A new endpoint `GET /admin/schema/integrity-lock` SHALL return a JSON Schema (Draft-07) document describing the `integrity.lock` file format including route entries, `resourcePolicy` (with `vramMb`, `gpuAffinity`), and canary config sub-schemas.

#### Scenario: Agent fetches integrity lock schema
- **WHEN** an agent calls `GET /admin/schema/integrity-lock`
- **THEN** the response is JSON with `$schema`, `title: "IntegrityLock"`, and a `routes` array property

### Requirement: core-host MUST expose a zero-copy layer-wise inference WIT contract
The project SHALL define `wit/ai/inference.wit` in the existing `tachyon:mesh@1.1.0` WIT package and SHALL expose a `layer-execution` interface with opaque `tensor-handle` values so Wasm guests can sequence model layers without copying intermediate tensors through linear memory.

#### Scenario: Guest orchestrates layer-wise execution through tensor handles
- **WHEN** a guest calls `load-layer`, `forward-layer`, and `drop-tensor` through the `layer-execution` interface
- **THEN** the host owns tensor memory natively and the guest only receives opaque `tensor-handle` identifiers

### Requirement: AI inference dependencies MUST remain feature-gated
The `core-host` crate SHALL keep heavyweight AI dependencies behind the `ai-inference` feature and SHALL return a clear fallback error for AI guests when the feature is not compiled.

#### Scenario: AI guest runs without ai-inference feature
- **WHEN** `core-host` is built without `--features ai-inference`
- **AND** an AI guest such as `guest-ai` is selected for execution
- **THEN** execution fails gracefully with an error naming the missing `ai-inference` feature

### Requirement: core-host MUST support native constrained decoding behind ai-inference
The `core-host` crate SHALL keep constrained decoding dependencies optional under the `ai-inference` feature, extend `wit/ai/inference.wit` with `sample-constrained`, and provide a native logit processor that compiles JSON Schema strings into cached FSM state before masking invalid token logits. CI SHALL verify that this requirement is implemented in code whenever it is asserted in the spec, so the requirement cannot be merged as spec text without a corresponding implementation.

#### Scenario: Guest samples logits with an optional JSON Schema
- **WHEN** a guest calls `sample-constrained` with a logits tensor handle and a JSON Schema
- **THEN** core-host samples only tokens allowed by the compiled schema FSM
- **AND** repeated calls with the same schema reuse the cached FSM by schema hash

#### Scenario: Core host is built without constrained decoding dependencies
- **WHEN** `core-host` is built without `--features ai-inference`
- **THEN** `llm-samplers`, `lru`, and the constrained decoding sampler module are not linked into the binary

#### Scenario: CI fails if the requirement is specified but not implemented
- **WHEN** the CI workflow builds `core-host --features ai-inference`
- **THEN** it verifies that `sample-constrained` and `FsmLogitProcessor` symbols exist in the codebase
- **AND** the build fails if either symbol is absent, preventing a recurrence of a merged spec requirement with no matching implementation

### Requirement: Candle LLM dependencies MUST remain feature-gated
The `core-host` crate SHALL keep tokenizer and Candle text-generation dependencies optional under the existing `ai-inference` feature and SHALL keep the default host build free of those dependencies.

#### Scenario: Default host build excludes Candle LLM runtime
- **WHEN** a developer builds `core-host` without `--features ai-inference`
- **THEN** tokenizer and Candle LLM runtime dependencies are not linked
- **AND** the default release and container workflows remain unchanged

#### Scenario: AI inference build includes Candle LLM runtime
- **WHEN** a developer builds `core-host` with `--features ai-inference`
- **THEN** the Candle LLM runtime module, tokenizer support, and selected Candle text-generation dependency are compiled
- **AND** existing ONNX/WASI-NN AI inference support remains available

#### Scenario: AI inference build consumes the downstream Candle quantization fork
- **WHEN** a developer builds `core-host` with `--features ai-inference`
- **THEN** `candle-core`, `candle-nn`, `candle-onnx`, and `candle-transformers` resolve from the pinned `astorise/candle` fork revision that carries GPTQ/Marlin, AWQ, and block-wise FP8 weight-quantization kernels proposed upstream in `huggingface/candle#3650`
- **AND** the default `core-host` build remains free of those optional Candle dependencies

#### Scenario: AI guest runs without ai-inference feature
- **WHEN** `core-host` is built without `--features ai-inference`
- **AND** an AI guest or route requires a model binding
- **THEN** execution fails gracefully with an error naming the missing `ai-inference` feature
