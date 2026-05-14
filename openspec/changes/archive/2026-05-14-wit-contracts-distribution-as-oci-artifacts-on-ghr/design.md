# Design: wit-contracts-distribution-as-oci-artifacts-on-ghr

## Tasks 1, 2, 3 — `publish-wit-oci` job in `publish-sdks.yml`

A new job `publish-wit-oci` is appended to `.github/workflows/publish-sdks.yml`. It runs on `v*` tags and `release: published` events.

**Steps:**
1. Checkout repository.
2. Install Rust toolchain (needed by `wkg`).
3. Rust dependency cache (Swatinem/rust-cache).
4. `cargo install wkg --locked` — Bytecode Alliance Wasm Package Tools CLI.
5. `docker/login-action@v4` — authenticates to `ghcr.io` using `GITHUB_TOKEN` (no secrets beyond the default needed).
6. `wkg publish` — strips the `v` prefix from `GITHUB_REF`, then publishes `./wit` as an OCI artifact to `ghcr.io/$OWNER/tachyon-mesh-wit:$VERSION`.
7. Verification echo.

**Permissions:** `packages: write` (required to push to GHCR) + `contents: read`.

## Task 4 — Documentation

**`README.md`**: New "🧩 Building FaaS Guests — WIT Contracts via OCI" section (placed before Troubleshooting) explaining:
- `[package.metadata.component.dependencies]` syntax for `cargo-component`.
- `wkg list` command to enumerate published versions.

**`faas-sdk/README.md`** (new file): Comprehensive guest developer guide covering both the SDK crate path and the direct WIT/cargo-component path, with a table of available interfaces and version-pinning instructions.
