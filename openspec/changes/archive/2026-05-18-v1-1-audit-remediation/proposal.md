# Proposal: v1.1.x Audit Remediation

## Context
A comprehensive technical audit of the `v1.1.x` branch revealed critical discrepancies between the declared OpenSpec tasks and the actual codebase. Specifically:
1. **Critical Security Flaw:** `system-faas-cdc-broadcaster` contains a dummy authentication check allowing any non-empty token to bypass security.
2. **Denial of Service (DoS):** Unbounded `HashMap` in `SubspaceAccessTracker` and unbounded JSON deserialization in `olap-engine`.
3. **Ghost Features & Dead Code:** Five recent proposals (Constrained Decoding, VRAM Orchestration, QUIC zero-copy, BaaS Data Fabric, Business Canary) were marked as complete (`[x]`) but consist mostly of unwired stubs covered by `#[allow(dead_code)]`.
4. **Toolchain Missing:** Local builds fail due to missing `rust-toolchain.toml` requiring `rustc 1.95+`.

## Objective
This is a strict remediation pass. The AI Agent must **NOT** attempt to finish the incomplete features. The objectives are strictly limited to:
1. Fixing the authentication bypass in `cdc-broadcaster` (fail-closed implementation).
2. Fixing the memory exhaustion DoS vectors.
3. Adding a `rust-toolchain.toml` file.
4. Unchecking the false-positive tasks in the archived proposals.
5. Hiding all identified unwired stubs behind a `#[cfg(feature = "experimental")]` flag and removing `#[allow(dead_code)]`.

## Scope
- `systems/system-faas-cdc-broadcaster/src/lib.rs`
- `core-host/src/mesh/migration.rs`
- `systems/system-faas-olap-engine/src/lib.rs`
- Root `rust-toolchain.toml`
- OpenSpec archived `tasks.md` for the 5 identified ghost features.
- `core-host` files containing `#[allow(dead_code)]` introduced in recent commits.