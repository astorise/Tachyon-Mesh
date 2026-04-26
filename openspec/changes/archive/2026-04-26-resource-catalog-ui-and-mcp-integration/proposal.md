# Proposal: Resource Catalog UI & MCP Integration

## Context
Following the implementation of "Location Transparency" and the Unified Resource Mapping in the `core-host` (which resolves logical aliases to internal Wasm IPCs or external HTTPS proxies), the platform requires administration interfaces. Currently, modifying the `resources` map relies entirely on the CLI.

## Proposed Solution
We will implement a two-pronged administration approach:
1. **Tachyon UI (Tachyon Studio):** A dedicated "Resource Catalog" view allowing administrators to visualize, add, edit, and remove logical resources. It will provide clear visual distinction between Internal and External resources.
2. **Tachyon MCP Server:** Expose the resource catalog to AI agents via the Model Context Protocol. This enables connected AIs (like Cursor or Claude) to query the available network topology and register new resources when generating FaaS modules that require external dependencies.

## Objectives
- Bridge the UX gap for Location Transparency in Tachyon Studio.
- Empower AI agents to interact with the Mesh's egress/IPC routing rules autonomously.
- Ensure the UI seamlessly triggers the re-sealing of the `integrity.lock` when resources are modified.