# Proposal: Change 070 - Local Asset Registry & Air-Gapped Deployments

## Context
Currently, the system assumes that container images (OCI) and WebAssembly modules (WASM) are fetched from external remote registries at deployment time. This violates strict Zero-Trust and Air-Gapped operational requirements. To ensure absolute supply chain security and offline capability, the Tachyon Mesh must operate its own embedded asset registry. External network calls for deployments must be strictly eliminated.

## Objective
1. Develop an embedded Local Asset Registry within the Tachyon Mesh.
2. Enable the `tachyon-ui` and `tachyon-mcp` to upload (Push) compiled `.wasm` binaries directly to the Mesh control plane.
3. Update the `core-host` execution engine to resolve and load modules strictly from this local, cryptographically verified storage rather than pulling from external URLs.

## Scope
- Create a `system-faas-registry` component.
- Add file upload capabilities to `tachyon-client` and expose them to Tauri/MCP.
- Modify the Wasmtime instantiation logic in `core-host` to read from the local registry.