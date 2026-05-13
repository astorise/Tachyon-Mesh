# Design: operator-zero-build-deployment

## Overview

Enables operators (who have no Rust/Node toolchain) to run Tachyon-Mesh in under a minute via a download script or a single `kubectl apply`, while preserving the contributor path that builds from source.

## Task 1 — `scripts/get-tachyon.sh`

A standalone installer script:
- Accepts `--version <tag>` (defaults to latest via GitHub Releases API) and `--dir <path>` (defaults to `./`).
- Detects OS (`uname -s`) and architecture (`uname -m`), normalising arm64 → aarch64.
- Constructs the tarball URL: `tachyon-mesh-VERSION-OS-ARCH.tar.gz`.
- Downloads with `curl -fsSL`, writes to a `mktemp` directory, checks HTTP status, and extracts into `TARGET_DIR`.
- Prints a coloured success banner with the absolute path to the binaries and a ready-to-paste MCP JSON snippet.
- Fails gracefully with a build-from-source hint when the HTTP download returns non-200 (e.g. no release exists yet).

## Task 2 — `release.yml` — `publish-server-binaries` job

A new matrix job runs **only on `v*` tags** and builds `core-host` + `tachyon-mcp` for four targets:

| OS | Arch | Rust target |
|---|---|---|
| ubuntu-22.04 | x86_64 | `x86_64-unknown-linux-gnu` |
| ubuntu-22.04 | aarch64 | `aarch64-unknown-linux-gnu` (cross-compiled via `gcc-aarch64-linux-gnu`) |
| macos-latest | x86_64 | `x86_64-apple-darwin` |
| macos-latest | aarch64 | `aarch64-apple-darwin` |

Each matrix leg packages the two binaries into `tachyon-mesh-VERSION-OS-ARCH.tar.gz` and uploads to the GitHub release using `softprops/action-gh-release@v2`. The Tauri desktop publish job is unchanged.

## Task 3 — README "Path A / Path B" split

Quick Start is restructured into two clearly labelled paths:
- **Path A — Operators**: `curl | bash` one-liner for the download script, plus a `kubectl apply` one-liner for Kubernetes. No prerequisites beyond curl.
- **Path B — Contributors**: `git clone` + `./scripts/setup.sh` (or `setup.ps1` on Windows). Setup script description with optional flags.

## Task 4 — `manifests/deploy.yaml`

`image: tachyon-mesh:test` → `image: ghcr.io/astorise/tachyon-mesh:latest`; `imagePullPolicy: Never` → `Always`. A `livenessProbe` is also added. The `legacy-deployment` section is left as-is (integration-test fixture, not an operator concern).
