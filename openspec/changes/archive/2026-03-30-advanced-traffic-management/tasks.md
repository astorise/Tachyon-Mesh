# Tasks: Change 024 Implementation

- [x] 1.1 Convert the change to the spec-driven layout with a `design.md` file
  plus a delta spec under `specs/http-routing/spec.md`.
- [x] 2.1 Extend `tachyon-cli` so sealed routes can declare explicit
  `targets` with module names, optional weights, and optional header matches.
- [x] 2.2 Update `core-host` manifest deserialization and integrity validation
  to accept route targets while preserving backward compatibility for routes
  without explicit targets.
- [x] 3.1 Implement request-time target selection in `core-host` so header
  matches win before weighted rollout is evaluated.
- [x] 3.2 Propagate cohort headers on host-managed outbound mesh requests so
  downstream routes preserve the caller's rollout bucket.
- [x] 4.1 Add or update automated tests covering CLI normalization, header-first
  routing, weighted fallback routing, and cohort propagation.
- [x] 5.1 Verify the change with `cargo fmt --all`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
  `cargo test --workspace`, and `openspec validate --all`.
