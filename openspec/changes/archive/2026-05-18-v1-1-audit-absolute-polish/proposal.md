# Proposal: v1.1.x Audit Absolute Polish

## Context
This is the final micro-pass to achieve 100% literal compliance with the expert technical audit. Previous passes secured the runtime, fixed DoS vectors, aligned SemVer, and finished the core telemetry feature. This pass focuses on CI strictness and deeply embedded technical correctness in experimental modules.

## Objective
1. Prevent future AI-generated dead code by strictly enforcing `-D dead_code` in the GitHub Actions CI pipeline.
2. Align the experimental QUIC safetensors replication with the specification by replacing SHA-256 with BLAKE3.
3. Replace the mocked/unsafe Safetensors flat-array coercion with actual JSON header parsing for accurate stride and dtype extraction.
4. Expand the E2E integration test suite to include base loading tests for `cdc-broadcaster` and `olap-engine`.

## Scope
- `.github/workflows/ci.yml` (or equivalent main workflow)
- `core-host/src/server_h3.rs`
- `core-host/src/ai_inference.rs`
- `core-host/tests/` (Integration test expansion)