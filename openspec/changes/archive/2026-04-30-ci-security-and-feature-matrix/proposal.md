# Proposal: Enterprise CI/CD Security & Feature Matrix

## Context
Tachyon Mesh positions itself as a secure, Zero-Trust, and "zero overhead" Edge router. However, the current GitHub Actions workflow (`ci.yml`) lacks the mandatory security guardrails expected of a foundational infrastructure component. Furthermore, features hidden behind compile-time flags (like `ai-inference`, `chaos`, and `canary`) are currently only checked for compilation but not actively tested, leaving blind spots in our coverage.

## Proposed Solution
We will aggressively harden the `.github/workflows/ci.yml` pipeline:
1. **Security & Supply Chain:** Integrate `cargo audit` (vulnerability scanning), `cargo deny` (license compliance and banned crates), and SBOM (Software Bill of Materials) generation on every release.
2. **Memory Safety Validation:** Run `cargo +nightly miri test` specifically on modules utilizing `unsafe` code (such as the newly secured `cwasm` cache) to mathematically prove the absence of Undefined Behavior.
3. **Mutation Testing:** Introduce `cargo-mutants` on critical paths (like `auth.rs` and `tls_runtime.rs`) to verify that our tests actually catch logic alterations, not just line coverage.
4. **Feature Matrix Testing:** Modify the test runner to explicitly loop through all combination matrices (`--all-features`, `--no-default-features`, and specific isolation flags).

## Objectives
- Guarantee that no vulnerable dependency or incompatible license enters the codebase.
- Ensure that every optional module (AI, Layer 4 routing, metrics) is fully validated.
- Establish enterprise-grade confidence in the Rust `unsafe` blocks.