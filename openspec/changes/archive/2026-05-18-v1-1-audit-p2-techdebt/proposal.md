# Proposal: v1.1.x Audit Phase 3 - P2 Technical Debt

## Context
This is the final remediation pass for the `v1.1.x` branch based on the consolidated AI audit. While critical security (P0) and DoS/stability (P1) issues have been resolved, several P2 code hygiene and architectural soundness issues remain in the codebase, mostly centered around early WebAssembly integration prototypes.

## Objective
Address the remaining technical debt flagged in the audit to ensure the repository meets strict engineering standards, even for features currently flagged as experimental.

## Scope
- `core-host/src/ai_inference.rs` (Unsafe tensor parsing)
- `core-host/src/ai_inference/samplers.rs` (Integer truncations and dummy types)
- `core-host/src/telemetry/mod.rs` (Wasm Any-downcasting)
- Media/File access utilities containing `pipe_range_from_file` (Path canonicalization).