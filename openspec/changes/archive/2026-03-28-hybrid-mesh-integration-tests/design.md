## Overview

This change extends the existing k3d deployment from a single `core-host` service to a two-service topology:

- `tachyon-host` continues to serve WASM-backed FaaS endpoints.
- `legacy-mock` acts as a simple HTTP legacy application reachable inside the cluster.

The new guest `guest-call-legacy` does not perform networking itself. Instead, it emits a `MESH_FETCH:<URL>` command on stdout. After `core-host` captures and filters guest stdout, it detects that command, performs the outbound HTTP request with `reqwest`, and returns the fetched body to the original caller.

## Packaging

The existing root `Dockerfile` remains the single build context. It now compiles:

- `guest-example` for direct FaaS requests
- `guest-call-legacy` for the bridged FaaS-to-legacy path
- `legacy-mock` as a static Linux binary for its own runtime image

The builder stage also regenerates `integrity.lock` with both sealed routes before building `core-host`, so the host image embeds the correct runtime policy.

## Verification

The integration workflow deploys both services into k3d, port-forwards them locally, and validates four paths:

1. Direct legacy access
2. Direct FaaS access
3. Legacy-to-FaaS access
4. FaaS-to-legacy access through the host bridge
