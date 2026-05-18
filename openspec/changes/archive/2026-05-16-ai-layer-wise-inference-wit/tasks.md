# Implementation Tasks

- [x] **Task 1: WIT Definition**
  - Create the file `wit/ai/inference.wit` inside the standard project interface layout.
  - Recompile definitions with `wit-bindgen` or structural ecosystem equivalents.

- [x] **Task 2: Feature Splitting**
  - Update `core-host/Cargo.toml` to introduce the `ai-inference` feature gate.
  - Enforce that `candle-core` and structural dependencies are marked strictly optional.

- [x] **Task 3: Memory & GC Safety Infrastructure**
  - Create `core-host/src/ai_inference.rs`.
  - Wire the instance data mapping structure so it tracks state allocations natively per instance.
  - Implement the automatic cleanup hook inside the Wasmtime drop sequence to guarantee complete VRAM reclaim upon guest context termination.

- [x] **Task 4: Integration Smoke Test**
  - Write an explicit unit test `test_inference_stubs_no_feature` ensuring that when compiled without the feature, the binary overhead remains completely unaffected and gracefully returns the fallback error condition.
