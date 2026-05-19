# Spec: v1.1 GA Readiness — CI Regression Fix

## Requirements

1. **R1 — Default build is strict.** With default features and
   `RUSTFLAGS="-D dead_code"`, `cargo check -p core-host` MUST succeed
   with zero warnings. No production-path item may hide behind
   `#[allow(dead_code)]` without an inline justification comment
   referencing a specific tool-chain constraint.

2. **R2 — `ai-inference` build is consistent.** With
   `--features ai-inference` and `RUSTFLAGS="-D dead_code"`,
   `cargo check -p core-host` MUST succeed. Items consumed by code in
   the `ai-inference` build (whether the consumer is gated or not) MUST
   themselves be reachable in that build.

3. **R3 — `--all-features` build is clean.** With `--all-features` and
   `RUSTFLAGS="-D dead_code"`,
   `cargo clippy -p core-host --all-targets -- -D warnings -D clippy::unwrap_used`
   MUST succeed. Intentional v1.2 scaffolding behind the `experimental`
   and `ai-inference` features is permitted to remain unused via a
   single, documented crate-level `cfg_attr` allow.

4. **R4 — Real Wasmtime integration test stays green.**
   `cargo test -p core-host --test real_wasm_integration_test` MUST
   pass 2/2 on the pinned toolchain.

5. **R5 — Verification scope is non-negotiable.** A task that claims to
   fix CI MUST record the literal exit codes of every CI step touching
   the changed crate. Substituting a narrower scope (no `--all-features`,
   no `RUSTFLAGS`, no `-D` flags) for the documented CI commands is
   forbidden by this spec.

## Acceptance Tests

The five commands listed in `design.md` Verification table are the
acceptance suite. All five MUST exit 0.

## Out of Scope

- Wiring any of the unfinished v1.2 scaffolding (predictive VRAM,
  layer-wise inference, BaaS components, QUIC replication, geo-pinning,
  CQRS views, pushdown filters). Those have dedicated active OpenSpec
  changes.
- The workspace-wide clippy step (`ci.yml:84` workspace variant) which
  depends on `gdk-sys` system headers and cannot be exercised in
  air-gapped audit containers. CI itself runs this step; this change
  only verifies the `core-host`-scoped equivalent.
