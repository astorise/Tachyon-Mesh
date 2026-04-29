# Proposal: Native Rust TurboQuant Migration

## Context
Tachyon Mesh currently handles AI tensor quantization (compressing LLM KV caches) through a C++ library (`turboquant.cpp`) exposed to the Rust `core-host` via an FFI bridge (`turboquant-sys`). Maintaining a C++ bridge introduces significant risks: memory unsafety (segfaults), complex cross-compilation toolchains (`build.rs` requiring a C++ compiler), and FFI overhead. The open-source community has recently provided pure Rust implementations of Google's TurboQuant algorithm.

## Proposed Solution
We will perform a complete replacement of the C++ codebase with a native Rust dependency.
1. Rip out the entire `turboquant-sys` crate and its C++ fixtures.
2. Integrate a native Rust crate (such as `turboquant`) to handle PolarQuant and QJL quantization algorithms.
3. Refactor the `core-host/src/ai_inference.rs` (or relevant system FaaS) to call the safe Rust API instead of unsafe C ABI bindings.

## Objectives
- Achieve 100% memory safety in the AI quantization pipeline.
- Eliminate C++ build dependencies to streamline Edge cross-compilation.
- Reduce inference latency by avoiding FFI boundary crossings.