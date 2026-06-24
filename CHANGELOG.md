# Changelog

## Unreleased

- **Constrained/guided decoding now actually executes** (`core-host/src/ai_inference/samplers.rs`, `candle_llm_runtime.rs`, `wit/ai/inference.wit`): the previously merged-but-unimplemented spec requirement ("core-host MUST support native constrained decoding behind ai-inference", archived change `2026-05-16-ai-constrained-decoding`, 0/4 tasks ever coded) is now real. A hand-written JSON-Schema-to-FSM grammar compiler (`compile_schema`) handles flat top-level `object` schemas with scalar/string-enum properties, or a top-level scalar schema, rejecting anything else (nesting, arrays, `$ref`, `oneOf`/`anyOf`) at compile time with a typed error; compiled grammars are cached in a SHA-256-keyed LRU (`FsmCache`, reusing the already-declared `lru`/`sha2` dependencies — no new `llm-samplers` dependency was added, since the existing `candle_transformers::generation::LogitsProcessor` already does token sampling). `FsmLogitProcessor` masks every disallowed vocabulary id to `-inf` before each sample and advances grammar state after each emitted token, wired into the single shared `decode_loop` so it applies uniformly to dense, GGUF, and tensor/pipeline/expert-parallel checkpoints. `wit/ai/inference.wit`'s `layer-execution` interface gains `sample-constrained`. A new CI step in `ci.yml` fails the build if `FsmLogitProcessor` (in `core-host/src`) or `sample-constrained` (in the WIT contract) goes missing again. Verified by 7 new unit tests in `samplers.rs`, 2 new integration tests in `candle_llm_runtime.rs`, and the full `ai_inference::` suite (114 tests, 0 regressions). See `openspec/changes/2026-06-19-constrained-decoding-activation`.
- **Real NCCL all-reduce + NVML VRAM telemetry** (`core-host/src/ai_inference/parallel.rs`): tensor-parallel `RowParallelLinear::forward` now issues a real `cudarc::nccl::Comm::all_reduce` collective (via a shared `NcclShardGroup`, one set of communicators per `TensorParallelLlama::load` call) on `candle-cuda` builds with 2+ CUDA devices and contiguous `f32` partials; the existing host-staged manual sum remains the fallback everywhere else, byte-for-byte unchanged. `discover_cluster_topology`'s per-device free VRAM is now sourced from real NVML telemetry (`nvml-wrapper`) on `candle-cuda` builds instead of a hardcoded `0`, so `validate_parallel_topology`'s VRAM check can actually reject an oversized deployment on real hardware. A new `nccl_all_reduce_matches_cpu_staged_reference` test (two loopback NCCL ranks on one CUDA device) runs in the `cuda-quality` CI job on `arc-gpu-runners` as proof of real multi-rank collective execution, not just compilation. See `openspec/changes/2026-06-23-nccl-allreduce-nvml-telemetry-gpu-ci-proof`.
- **Model-parallel runtime dispatch** (`core-host/src/ai_inference/candle_llm_runtime.rs`): the tensor/pipeline/expert-parallel engines are now selected by the live model loader. `IntegrityModelBinding` carries a `hardware_strategy` (mirroring `wit/config-ai.wit`'s `hardware-strategy`), and `try_load` validates the requested plan against the discovered hardware topology before constructing the matching engine. Tensor-parallelism runs the full autoregressive decode loop; pipeline-parallelism is loaded and prefill-correct (its decode loop is a follow-up, so generation returns a typed error until then); expert-parallelism is validated and placed but rejected at load until a full MoE checkpoint loader exists. The candle CUDA backend (the existing `candle-cuda` feature → `candle-core/cuda`) is what the dispatch, multi-GPU enumeration, and tensor-parallel all-reduce gate on; on that build `discover_cluster_topology` enumerates real GPUs. The default build (and the standard feature matrix, which keeps `nvfp4-cuda` CPU-buildable) stays CPU-only and CI-reproducible; the CUDA path is exercised by the `cuda-quality` job. See `openspec/changes/2026-06-22-wire-model-parallel-runtime-dispatch`.
- Added the node registry and systems catalog surfaces: `control-plane-faas` can now import `kv-partition`, `system-faas-node-registry` persists enrolled nodes, and the UI exposes read-only Nodes and Systems views.
- Honest policy views: five write-only policy panels (resilience, identity-config, rbac, supply-chain, fleet) now display a "Policy form" badge; topology panel gains a View/Edit mode toggle (defaults to View) with session persistence.
- **FIPS + musl Alpine build** (`Dockerfile.fips`): new multi-stage Dockerfile using `rust:alpine` as the FIPS builder to compile `core-host --features fips` with musl libc, producing a static `FROM scratch` image (~32 MB). CI gains a dedicated `fips-tests` job and the Docker publish matrix gains a `-fips` variant. A `feature-matrix-tests` job now validates five distinct feature-flag combinations for `core-host`, each uploading a labelled release artifact.
- **AI inference: ORT → candle-onnx** (`core-host/src/ai_inference/candle_onnx_backend.rs`): replaced Microsoft ORT (native FFI, musl-incompatible) with Hugging Face `candle-onnx` (pure Rust). WASI-NN guest API is preserved — guests use the same `graph_load → init_execution_context → set_input → compute → get_output` flow. Models load from raw ONNX bytes decoded via `prost`. GPU inference now executes on the requested CUDA device via the forked candle's CUDA ONNX op support (`candle_device`/`ExecutionTarget::Gpu`, landed in #193) instead of being CPU-only; there is no per-operator GPU allow-list or `executed_on` telemetry field yet (tracked in `openspec/changes/2026-06-19-gpu-accelerated-inference-execution`). The `ai-inference` feature is now musl-compatible.
- **NVFP4 (ModelOpt) checkpoints now execute, not just load** (`core-host/src/ai_inference/{modelopt_nvfp4,candle_llm_runtime,ai_inference}.rs`): ModelOpt/NVFP4 Llama checkpoints (previously detected and load-time-validated, but unconditionally rejected with an "unsupported execution" error at run time) now dequantize every NVFP4-quantized linear to dense F32 at load time and run the same tested `Llama::load`/decode engine the plain-safetensors path uses; everything ModelOpt left unquantized (norms, embeddings) is read through as-is. This is the documented fallback execution path — native FP4-kernel matmul without eager dequantization remains a follow-up. Verified by a new equivalence test (`modelopt_nvfp4_dequantized_forward_matches_a_dense_reference`) that quantizes a fixture's `down_proj` with exact NVFP4 E2M1 levels and asserts the dequantized forward pass matches a plain-dense reference checkpoint within tolerance, plus the full `ai_inference::` suite (105 tests, 0 regressions). See `openspec/changes/2026-06-19-gpu-accelerated-inference-execution`.
- **Homelab K3S deployment**: provisioned two K3S instances (`tachyon-edge-1` and `tachyon-edge-2`) via WSL/MCP and deployed the latest `core-host` image from GHCR using hardened Kubernetes manifests with Pod Security Standards, NetworkPolicy, and GPU node-selector support.

## [v1.0.0] — General Availability (GA) · 2026-05-15

Tachyon-Mesh v1.0.0 marks the transition to a production-ready, Enterprise-grade FaaS and AI orchestration mesh. This major release signals the stabilisation of our core API and MCP contracts. Following a rigorous multi-stage usability and security audit, it delivers robust supply-chain security, a hardened LLM agent interface, and full WCAG AAA accessibility.

---

### 🛡️ Security & Supply Chain

- **Keyless Signing:** All Linux/macOS release artifacts are cryptographically signed using GitHub OIDC and Sigstore (`cosign sign-blob`). Rekor transparency-log bundles (`.bundle`) are attached to every release.
- **SBOM Generation:** SPDX 2.3 Software Bill of Materials are generated via `cargo sbom` and attached to releases for vulnerability scanning (Trivy, Grype, etc.).
- **Zero-Build Verification:** The `get-tachyon.sh` and `get-tachyon.ps1` installation scripts now enforce SHA-256 checksum validation before extraction, preventing MITM attacks.
- **XSS Immunity:** The entire UI has been rewritten to use native DOM APIs (`el()` / `frag()` helpers). All `innerHTML` interpolation of user data has been eliminated. A strict Content Security Policy is compatible with the new architecture.
- **GitHub Actions Hardening:** `publish-server-binaries` now requests `id-token: write` for keyless OIDC. Artifact names are resolved via `$GITHUB_OUTPUT` (step outputs) rather than `$GITHUB_ENV` for static linter correctness.

---

### 🤖 AI Agents & MCP (Claude Desktop / Cursor)

- **Pre-auth Parameter Validation:** `missing_required_args()` now runs before authentication and network calls, ensuring malformed requests return `-32602 Invalid Params` rather than `-32001 Cluster Unreachable`. Agents can self-correct without a live cluster.
- **Strict Rate Limiting:** All mutator tools are rate-limited before dispatch: `canary_split` at 2/min, `deploy/delete` at 5/min, KV mutators at 30/min. Rate limits fire even when the cluster is unreachable.
- **Error Taxonomy:** Standardised error codes across all 35 MCP tools: `-32602` (invalid params), `-32001` (cluster unreachable), `-32002` (rate limited with `retry_after_ms`), `-32603` (internal error).
- **E2E Test Coverage:** Full behavioural coverage for all lifecycle (`deploy_function`, `list_functions`, `function_logs`, `delete_function`) and KV (`kv_get`, `kv_put`, `kv_delete`) tools. All 12 tests pass without a live cluster.
- **`tools/list` Warnings:** When the manifest schema is not yet fetched, `tools/list` appends a `data.warnings` array instead of returning degraded results silently.
- **Rate-limit State Isolation:** Tests use unique per-process state paths to prevent inter-test contamination.

---

### ♿ UI & Accessibility (WCAG AAA)

- **Focus Restoration (WCAG 2.4.3):** `trapFocus()` now captures `previousFocus` on entry and restores it in the cleanup function. Keyboard users are no longer dropped to `<body>` after closing a modal.
- **Screen Reader Announcements:** `TachyonToastManager` container has `role="status" aria-live="polite" aria-atomic="false"`. All `sealAndApply` outcomes (success, conflict, error) are announced.
- **`<dialog>` Safety Docs:** `trapFocus()` carries a JSDoc warning that it must not be combined with native `<dialog>` elements to prevent duplicate focus trapping.
- **Global Apply Loader:** `aria-live="polite"`, `role="status"`, `.sr-only` descriptive text, and `aria-hidden` on the visual spinner. `aria-busy="true"` is set on `main-content` during apply.
- **Component Decomposition:** `TachyonAppShell` split into `TachyonAppShellNav` and `TachyonAppShellModalRoot`. `TachyonIAM` split into `TachyonAuthStepCredentials`.

---

### ☸️ Kubernetes & Infrastructure

- **Hardened Homelab Manifest:** `manifests/deploy-gpu-homelab-hardened.yaml` enforces Pod Security Standards (Restricted): dedicated `ServiceAccount`, `runAsNonRoot`, `readOnlyRootFilesystem`, `capabilities.drop: [ALL]`, `seccompProfile: RuntimeDefault`, `/tmp emptyDir` scratch volume.
- **Zero-Trust NetworkPolicy:** Default-deny ingress/egress with explicit allowances for port 8080 (MCP/API), Prometheus scraping from the `monitoring` namespace, DNS (UDP+TCP/53), and HTTPS (TCP/443).
- **Dynamic OpenAPI:** All 35 core-host routes are documented and served at `/admin/docs` (Swagger UI) and `/admin/schema/openapi.json`. The integrity.lock JSON Schema is served at `/admin/schema/integrity-lock`.
- **IDE Schema Integration:** VS Code `json.schemas` and YAML modeline configs for `integrity.lock` validation. See `docs/ide-integration.md`.
- **GPU Homelab:** `deploy-gpu-homelab.yaml` supports NVIDIA GPU nodeSelector, `tachyon-model-pvc` (50 Gi), `ServiceMonitor` for Prometheus, and `CUDA_VISIBLE_DEVICES` injection.

---

### 🔧 Developer Experience

- **Setup Scripts:** `scripts/setup.sh` and `scripts/setup.ps1` bootstrap the full development environment (WASM targets, binaries, guest artifacts, npm install) with `--skip-guests` and `--skip-ui` flags.
- **Zero-Build Installers:** `get-tachyon.sh` and `get-tachyon.ps1` download pre-built binaries, verify SHA-256, and print an MCP config banner.
- **TROUBLESHOOTING.md:** 15 documented failure modes covering build, runtime, UI, MCP, and Kubernetes/GPU domains.
- **`docs/ide-integration.md`:** VS Code, JetBrains, Neovim integration guides and offline schema snapshot procedure.

---

## [v0.x] — Pre-release development

All earlier development was pre-release. No public changelog is maintained for the `0.x` series.
