# Proposal: v1.1.x Audit Zero Warning Closure

## Context
This represents the absolute final pass of the `v1.1.x` audit remediation. All critical vulnerabilities, memory leaks, and core logic flaws have been resolved. This phase focuses on achieving 100% literal compliance with the AI audit report by extending integration tests to the remaining missing components, purging unused dependencies, and taking a definitive stance on the stubs.

## Objective
1. Scaffold integration test boundaries for the remaining 4 missing systems: `view-builder`, `sql-engine`, `vector-search`, and `media-server`.
2. Remove the phantom `biscuit-auth` dependency from `core-host` or feature-gate it properly if retained for future experimental use.
3. Decisively resolve the `constrained-decoding` module by stripping its unused WIT contract and code, rather than leaving it in an experimental limbo state, as per the auditor's strict recommendation.

## Scope
- `core-host/tests/`
- `core-host/Cargo.toml`
- `core-host/src/ai_inference/samplers.rs`
- `wit/ai/` (or equivalent location of the constrained decoding WIT)