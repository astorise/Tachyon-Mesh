## Why

The current k3d integration path only proves that a single WASM-backed FaaS service can be packaged and reached inside the cluster. Tachyon Mesh is supposed to bridge WASM functions with legacy-style services, so CI should validate both directions of that traffic before the topology can be treated as credible.

## What Changes

- Add a `legacy-mock` service that simulates a legacy container and can call the FaaS service from inside the cluster.
- Add a `guest-call-legacy` WASM guest plus a host-side `MESH_FETCH:` bridge so a guest can ask `core-host` to fetch a legacy service URL on its behalf.
- Extend the Docker build, Kubernetes manifests, and k3d GitHub Actions workflow so both services are deployed and all direct and bridged traffic paths are verified.
- Regenerate the sealed runtime configuration so `core-host` allows both `/api/guest-example` and `/api/guest-call-legacy`.

## Capabilities

### Modified Capabilities

- `http-routing`: accept both `GET` and `POST` traffic for guest functions and support host-mediated outbound mesh fetches requested by a guest.
- `k3d-integration-test`: package, deploy, and validate the hybrid WASM plus legacy topology inside k3d.

## Impact

- Adds new workspace members `guest-call-legacy` and `legacy-mock`.
- Updates the runtime container build, Kubernetes deployment manifests, and `.github/workflows/integration.yml`.
- Expands the sealed `integrity.lock` routes so the hybrid mesh path is reachable at runtime.
