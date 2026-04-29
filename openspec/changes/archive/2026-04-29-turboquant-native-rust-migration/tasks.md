# Implementation Tasks

## Phase 1: Code Deletion
- [x] Removed `turboquant-sys/native/` (C++ shim) and
      `turboquant-sys/{build.rs, fixtures, tools}`.
- [x] Stripped `links =`, `build =`, and the `cc` build dep from
      `turboquant-sys/Cargo.toml`.
- [x] Kept the crate name `turboquant-sys` so callers don't need updates;
      the implementation is now pure Rust.

## Phase 2: Native Rust port
- [x] `turboquant-sys/src/lib.rs` is now a faithful port of the 2-bit /
      3-bit codebook quantizer with the same public surface
      (`packed_len`, `compress_values`, `decompress_values_sparse`,
      `TurboQuantError`).
- [x] Codebooks (`LEVELS_2_BIT`, `LEVELS_3_BIT`) reproduce the C++ tables
      byte-for-byte; the straddling 16-bit window unpack (used by codes
      that cross a byte boundary) is preserved.
- [x] No `unsafe` of any kind. The FFI boundary is gone.

## Phase 3: Caller updates
- [x] `core-host/src/ai_inference.rs` callers
      (`turboquant_sys::packed_len`, `compress_values`,
      `decompress_values_sparse`) compile unchanged; only the
      fixture-based test was replaced by a self-contained round-trip
      that exercises the full `apply_op2_no_bwd` integration.

## Phase 4: Validation
- [x] `cargo build -p core-host` no longer pulls a C compiler — the build
      output above shows no `cc`/`cl` invocations.
- [x] `cargo test -p turboquant-sys` — 7 passing (round-trip 2- and 3-bit,
      sparse-decode threshold, length-mismatch errors).
- [x] `cargo test -p core-host --features ai-inference --bin core-host
      turboquant` — 2 passing (the new round-trip test plus the existing
      `boundary_layers_bypass_turboquant_value_compression`).
