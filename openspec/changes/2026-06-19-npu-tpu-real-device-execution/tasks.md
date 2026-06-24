# Implementation Tasks

- [x] **Task 1: AcceleratorBackend abstraction** — done with a narrower shape than literally specified, see note
  - Implemented as `core-host/src/ai_inference/accelerator_backend.rs`: `AcceleratorAvailability { backend: AcceleratorKind, status: AvailabilityStatus }` where `AvailabilityStatus` is `Available | Unavailable { reason: &'static str }`. `probe(AcceleratorKind) -> AcceleratorAvailability` is the capability check; `resolve_with_fallback(preferred, fallback, probe_fn)` is the fallback-policy helper, parameterized over the probe function so it is unit-testable without hardware.
  - Deviation: there is no `AcceleratorBackend` enum (`Candle(Device) | OpenVinoNpu | EdgeTpu`) as originally proposed. `Cpu`/`Gpu` dispatch already runs through the existing `candle::Device`-based path (`AiInferenceRuntime::load_component_model`, `candle_llm_runtime.rs`) and is **completely unchanged** by this work — per this change's own non-goal ("does not change the existing GPU/CPU dispatch paths"), and to avoid risk to the only two tests that exercise that path (`component_accelerator_runtime_rejects_mismatched_devices`, `heterogeneous_runtime_routes_models_to_dedicated_accelerators`). `probe`/`AcceleratorAvailability` is a new, additive capability-reporting layer sitting alongside (not replacing) `AcceleratorKind`/`supports_accelerator`, which remains the separate "valid scheduling/QoS lane" concept it always was.

- [ ] **Task 2: OpenVINO NPU backend** — not done, deferred
  - Not implemented. Requires the OpenVINO SDK (a native vendor dependency) and, per Task 5, real NPU hardware to validate against — neither is available in this sandboxed environment. `probe()` honestly reports `Npu` as `Unavailable { reason: "no_backend_wired" }` rather than fabricating a backend that cannot be exercised. Tracked as follow-up work; see module docs in `accelerator_backend.rs`.

- [ ] **Task 3: Edge TPU backend** — not done, deferred
  - Not implemented, for the same reason as Task 2: requires the `libedgetpu` SDK and a physical Coral USB TPU, unavailable here. `probe()` reports `Tpu` as `Unavailable { reason: "no_backend_wired" }`.

- [x] **Task 4: Capability reporting tied to real backend presence**
  - `AiInferenceRuntime::load_component_model` (`core-host/src/ai_inference.rs`) now calls `accelerator_backend::probe(accelerator)` when, and only when, `accelerator` is `Npu` or `Tpu`, and returns a typed error (`"{kind} accelerator is unavailable on this host: {reason}"`) instead of proceeding to load a model against a label with no real execution path. `Cpu`/`Gpu` dispatch in this function is untouched.
  - This is scoped narrower than "update capability discovery" generally: `supports_accelerator`/`AcceleratorKind::ALL` (the pre-existing scheduling-lane/QoS-queue-sizing concept) is deliberately **not** changed — it continues to report `true` for all four kinds, since that's a distinct, already-tested concept (per-class queue provisioning), not a hardware-availability claim. The honesty fix lives at the model-load boundary, where a real load attempt actually happens.
  - Fallback-policy routing (actually redirecting a dispatch from `Npu`→`Cpu`/`Gpu` when unavailable, end-to-end through the WIT guest-facing surface) is implemented as the testable, injectable `resolve_with_fallback` helper, but is not yet wired into the live `load_accelerator_model`/`compute_accelerator_prompt` WIT dispatch path in `component_hosts.rs` — that wiring, and the corresponding guest-visible fallback behavior, remains a follow-up.

- [ ] **Task 5: Hardware validation (manual / labeled hardware runner)** — not done, deferred
  - Not run. No physical CPU+NPU+GPU machine or Coral USB TPU is available in this environment. This is consistent with Tasks 2/3 being deferred: there's no real backend yet to validate.

- [x] **Task 6: Tests** — partially done, scoped to what's actually implemented
  - Unit tests for `AcceleratorAvailability` status transitions: `cpu_is_always_available`, `npu_and_tpu_report_unavailable_with_no_backend_wired`, `gpu_reports_unavailable_on_a_cpu_only_build` (`accelerator_backend.rs`).
  - CI-runnable dispatch-logic tests using an injected probe function (standing in for "mocked `AcceleratorBackend`"): `resolve_with_fallback_picks_preferred_when_available`, `resolve_with_fallback_falls_back_when_preferred_is_unavailable`, `resolve_with_fallback_honors_an_injected_probe_for_testability`.
  - Integration test for the new `load_component_model` gate: `load_component_model_honestly_rejects_npu_and_tpu_as_unavailable` (`ai_inference.rs`), confirming the existing `component_accelerator_runtime_rejects_mismatched_devices` (which asserts `Gpu` dispatch still succeeds in CPU-only CI) is unaffected.
  - Hardware-gated integration tests for OpenVINO/Edge TPU: not added, since there is no backend implementation (Tasks 2/3) for them to exercise.
  - Full regression: `cargo test -p core-host --features ai-inference` — 121/121 passed (114 pre-existing + 7 new), 0 regressions. `cargo clippy -p core-host --features ai-inference --all-targets -- -D warnings -D clippy::unwrap_used` — clean.

- [ ] **Task 7: Docs** — not done, deferred
  - Not written. There is no supported NPU/TPU vendor or op set to document yet (Tasks 2/3 deferred). The `accelerator_backend.rs` module doc comment states, in code, which classes have a real backend (`Cpu`, `Gpu` via CUDA) vs. capability-routing-only (`Npu`, `Tpu`) — but this has not been mirrored into `heterogeneous-accelerator-orchestration`'s operator-facing docs outside this module.
