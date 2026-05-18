# Proposal: v1.1.x Audit Hygiene & Alpha Transition

## Context
Following the initial critical security pass, this second remediation pass addresses the semantic versioning integrity and codebase health concerns flagged in the technical audit:
1. **Semantic Versioning Breakage:** The hardcoded `1.1.0` version bump across manifests implies fully ready features, which contradicts the actual state of unwired stubs. Shifting to `1.1.0-alpha` correctly signals in-progress development.
2. **Silent Mutex Poisoning:** In `core-host/src/telemetry/mod.rs`, using `unwrap_or_else(|p| p.into_inner())` silently swallows lock panics, potentially leaving registries in an inconsistent state without diagnostic visibility.

## Objective
1. Transition all project manifests from `1.1.0` to `1.1.0-alpha`.
2. Replace blind mutex poisoning recovery with safe propagation or clean panic logging to prevent silent telemetry corruption.

## Scope
- Workspace `Cargo.toml` manifests (`core-host`, components, etc.)
- Root `package.json`
- `tachyon-ui/tauri.conf.json`
- `core-host/src/telemetry/mod.rs`