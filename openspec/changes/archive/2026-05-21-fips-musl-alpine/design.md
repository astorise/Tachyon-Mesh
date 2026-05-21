## Context

`aws-lc-fips-sys` (which backs the `fips` feature in `core-host`) requires a native musl toolchain with cmake, nasm, go, perl, and linux-headers to compile its C/assembly bundled source. Ubuntu glibc builds fail to satisfy these constraints cleanly. Alpine Linux ships musl libc natively and the `rust:alpine` Docker image provides a clean base for static compilation.

The existing `Dockerfile` uses a glibc Ubuntu builder and produces a dynamically-linked binary; it cannot build `--features fips`. `Dockerfile.fips` is a separate multi-stage file that keeps the WASM build on Ubuntu (where `wasm-pack`/`tinygo`/`dotnet`/`java` toolchains are better supported) and moves the `core-host` FIPS compilation to Alpine.

## Goals / Non-Goals

**Goals:**
- Produce a `core-host` binary compiled with `--features fips` and statically linked via musl.
- Final Docker image under 35 MB using `FROM scratch`.
- Validate FIPS build in CI via a dedicated `fips-tests` job.
- Publish `ghcr.io/<owner>/tachyon-mesh:latest-fips` on every main push.

**Non-Goals:**
- Replacing the existing `Dockerfile` (glibc, non-FIPS builds are unchanged).
- Enabling `--features fips` in the standard `rust-ci` job (kept separate to avoid musl toolchain in the main job).
- FIPS certification of the runtime environment (only the cryptographic library is FIPS-validated).

## Decisions

### D1: Alpine as FIPS builder, Ubuntu as WASM builder

Alpine was chosen over Debian-slim/musl cross-compilation because:
- `rust:alpine` ships musl libc natively — no cross-toolchain complexity.
- `aws-lc-fips-sys` build-time deps (cmake, nasm, go, perl, linux-headers) are available in Alpine's package manager.
- Cross-compiling musl targets from Ubuntu requires `musl-tools` + target injection and has historically caused LLVM/linker mismatches with FIPS code.

Alternative considered: `cargo-zigbuild` with `x86_64-unknown-linux-musl` target on Ubuntu. Rejected because zigbuild wraps the linker but not the C/assembly build system for `aws-lc-fips-sys`.

### D2: Multi-stage — single `Dockerfile.fips`, not a multi-file approach

A single Dockerfile with named stages (`wasm-builder`, `fips-builder`, various polyglot builders, `runtime`) is easier to maintain and keeps the build graph explicit. Separate Dockerfiles would require coordinated artifact passing via shared volumes or registries.

### D3: `FROM scratch` final stage

The final image copies only the `core-host` binary and WASM modules. No shell, no package manager. This minimizes attack surface for a FIPS-compliant deployment and produces ~32 MB images (vs. ~120 MB for Ubuntu-based).

### D4: Dedicated `fips-tests` CI job

FIPS build dependencies (cmake, nasm, protobuf-compiler) are heavy. Isolating them in a separate job avoids bloating the main `rust-ci` job's setup time. The `fips-tests` job runs only `cargo test -p core-host --features fips` (no full workspace test).

## Risks / Trade-offs

- **Alpine glibc compatibility** → Not a risk: the binary is statically linked and the runtime is `FROM scratch`, so there is no glibc dependency in the final image.
- **FIPS + other features** → `fips` is mutually exclusive with `ring`-based features. The `fips-tests` job uses `--features fips` only, not `--all-features`. The CI matrix explicitly excludes combined fips+all-features.
- **Alpine package staleness** → `rust:alpine` uses Alpine's rolling packages. If `nasm` or `go` versions change, build may break. Mitigation: pin to specific Alpine package versions in a future maintenance pass.
- **Image size regression** → Adding WASM modules to the scratch image increases size. Mitigation: WASM modules are built separately (Ubuntu stage) and copied selectively.

## Migration Plan

1. Merge `Dockerfile.fips` (already implemented).
2. CI `fips-tests` job runs on every push to validate FIPS build.
3. Docker `publish-docker-images` job publishes `-fips` tagged image on main push.
4. Consumers wanting FIPS pull `ghcr.io/<owner>/tachyon-mesh:latest-fips` instead of `latest`.

No rollback needed — `Dockerfile.fips` is additive; the existing `Dockerfile` and `latest` tag are unchanged.

## Open Questions

- Should `fips-tests` run on PRs or only on `main`? Currently runs on all triggers (same as `rust-ci`).
- Alpine package pinning for reproducible FIPS builds: deferred to a future maintenance change.
