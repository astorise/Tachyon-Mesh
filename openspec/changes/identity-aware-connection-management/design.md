# Design: Identity-Aware Connection Management

## Overview
The connection pipeline is split into three layers:

1. `system-faas-auth` exports a dedicated `tachyon:identity/auth` interface for JWT verification and recovery-code primitives.
2. `core-host` exposes `/admin/*` routes protected by middleware that instantiates the auth component on demand.
3. `tachyon-client` and `tachyon-ui` keep a runtime connection profile and validate it against `/admin/status` before the dashboard becomes interactive.

## Host Boundary
- Administrative routes are isolated under `/admin/*`.
- The middleware extracts `Authorization: Bearer <token>`.
- Verification is delegated to the auth component rather than duplicated in host logic.

## Client Boundary
- The desktop client stores the current node URL, bearer token, and optional identity bundle in a process-local `RwLock`.
- If a connection exists, status reads go through the remote `/admin/status` endpoint; otherwise they fall back to local `integrity.lock` parsing.

## Runtime Packaging
- The new component is compiled in CI and copied into the container runtime so the host can resolve `system-faas-auth.wasm` from the same guest-module search path as the other system FaaS modules.
