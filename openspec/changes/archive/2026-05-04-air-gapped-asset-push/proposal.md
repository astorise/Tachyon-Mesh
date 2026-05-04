# Proposal: Air-Gapped Asset PUSH Configuration

## Context
Standard Service Meshes and orchestrators rely on nodes pulling artifacts (images, Wasm modules) from external OCI registries. This "PULL" model forces Edge nodes to have outbound internet access, exposing the infrastructure to Supply Chain attacks and preventing deployments in strict Air-Gapped environments (military, industrial).

## Problem
If Tachyon nodes pull artifacts themselves, they violate the Zero-Trust network boundary. We need a mechanism where artifacts are pushed into the mesh via a trusted intermediary (Tachyon-UI or MCP), verifying cryptographic signatures before the edge node ever attempts to load them.

## Solution
Implement a "PUSH-First" Asset Management schema (`config-assets.wit`). 
1. The Control Plane (UI/MCP) performs the PULL from external sources, or builds the asset locally.
2. The UI pushes the raw bytes to the `system-faas-storage-broker`.
3. The configuration YAML only defines the expected SHA256 hash and cryptographic signature of the asset. 
4. The `core-host` guarantees execution integrity by verifying the hash without making any external network requests.