# Proposal: Strict WIT Versioning

## Context
As we release official Polyglot SDKs, the `tachyon.wit` interfaces become our public API contract. If a developer renames a function or changes a struct field in the `wit` file, all user-deployed Wasm modules compiled against the older version will instantly fail to link at runtime.

## Proposed Solution
1. **Explicit Versioning:** All `wit` files must declare their package version (e.g., `package tachyon:mesh@1.0.0;`).
2. **CI Enforcement:** We will use `wit-bindgen --check-compat` (or `wasm-tools component check`) in a dedicated GitHub Action. It will compare the `wit` files in the current PR against the `main` branch. If a non-backward-compatible change is detected, the CI fails unless the major version number has been bumped.