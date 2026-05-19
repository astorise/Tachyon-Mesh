# Proposal: v1.1 GA Readiness — CI Regression Fix

## Context

The previous OpenSpec change `2026-05-18-v1-1-ga-readiness` correctly
introduced real `#[cfg(feature = "experimental")]` gating and an authentic
Wasmtime integration test. Its verification block reported clean local
builds:

```
cargo build -p core-host (default features): clean, zero warnings.
cargo clippy -p core-host --all-targets -- -D warnings -D clippy::unwrap_used: clean.
cargo clippy --workspace --all-targets -- -D warnings: clean.
```

However the audit on the resulting branch tip (commit `5408cdb`) caught
**two regressions blocking CI**:

1. **`cargo check -p core-host --features ai-inference`** (CI step at
   `.github/workflows/ci.yml:111`) fails with `error[E0433]: cannot find
   type 'KvPrecision' in this scope`. The enum is gated
   `#[cfg(feature = "experimental")]` (`ai_inference.rs:1233`) but its
   sole consumer `TurboQuantLayerDecision` (`ai_inference.rs:1259`) and
   `TurboQuantAttentionStack::layer_decision` (line 1267) are **not**
   gated. With `ai-inference` on and `experimental` off, the type
   disappears while its callers remain → compile break.

2. **`cargo clippy --workspace --all-targets --all-features -- -D warnings -D clippy::unwrap_used`**
   (CI step at `.github/workflows/ci.yml:84`) fails with 90 errors:
   - 89 × dead-code (`is never used`/`is never constructed`/`is never read`)
     on items gated `#[cfg(feature = "experimental")]`. When
     `--all-features` activates `experimental`, those items get compiled
     in but **stay unused** (they are intentional v1.2 scaffolding with
     no consumer yet). The CI's global `RUSTFLAGS="-D dead_code"` turns
     the warnings into errors.
   - 1 × `clippy::unnecessary_lazy_evaluations` at
     `ai_inference.rs:1518` — `SafetensorsHeader::parse(&mmap).unwrap_or_else(|_| SafetensorsHeader { ... })`
     uses `unwrap_or_else` with a constructor closure that doesn't
     reference the error, which clippy wants as `unwrap_or(...)`.

The previous design document already acknowledged option 2 in its
verification line *"cargo build -p core-host --features experimental:
builds (49 dead-code warnings on experimental items themselves; **CI
only checks default**)."* — but that assumption was wrong. CI line 84
runs `--all-features`, which includes `experimental`.

## Objective

Restore green CI on `v1.1.x` without regressing any of the architectural
honesty gains from `v1-1-ga-readiness`. The fix must be minimal,
documented, and survive future expansions of `--all-features`.

## Scope

- `core-host/src/main.rs` — crate-level lint attribute.
- `core-host/src/ai_inference.rs` — `KvPrecision` gating consistency
  + the `unwrap_or_else` clippy nit.

Out of scope: the eight backlog proposals restored to
`openspec/changes/` by `2026-05-18-v1-1-audit-backlog-restoration`.
Those remain genuinely unfinished work and are intentionally not
addressed here.

## Why this is the right shape

The v1.2 scaffolding behind `experimental` is intentionally unwired
today. Wiring it just to satisfy `dead_code` would be the exact "gaming"
behavior `v1-1-ga-readiness` was built to forbid. A crate-level
`#![cfg_attr(feature = "experimental", allow(dead_code))]` makes the
contract explicit: *when the experimental feature is on, we tolerate
dead code by design*. Default builds remain strict (`-D dead_code` still
fires), so regressions in production paths cannot hide.

The `KvPrecision` fix follows the existing 3-way classification: the
enum is consumed by code that ships in the default build, so per the
ga-readiness rule "items actually used in default builds → annotation
removed", the `#[cfg(feature = "experimental")]` is removed.
