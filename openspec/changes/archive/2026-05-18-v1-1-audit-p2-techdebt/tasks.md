# Implementation Tasks

- [x] **Task 1: Secure File Range Piping**
  - Locate `pipe_range_from_file` (likely in `systems/system-faas-media-server/src/lib.rs` or `core-host` storage utilities).
  - Implement path canonicalization (`std::fs::canonicalize`) and ensure the resolved path starts with the allowed root directory boundary before reading.

- [x] **Task 2: Refactor Safetensors Memory Mapping**
  - In `core-host/src/ai_inference.rs`, remove the `unsafe` array coercion block in `LayerWiseMappedModel`.
  - Implement safe slice viewing or return an explicit `unimplemented!()` if the logic is strictly experimental.

- [x] **Task 3: Clean up Sampler Types**
  - In `core-host/src/ai_inference/samplers.rs`, fix the `token_id as u32` cast (handle potential truncation safely).
  - Remove `_sampler_marker` and its associated `PhantomData`.
  - Refactor `CompiledFsm::transition` to remove arbitrary `wrapping_add`/`wrapping_mul` logic that masquerades as a real FSM.

- [x] **Task 4: Fix Telemetry Any-Downcasting**
  - In `core-host/src/telemetry/mod.rs`, locate the `Any` downcasting block (around lines 120-251).
  - Refactor the registry storage to avoid `downcast_ref` by using typed enums or structs.
