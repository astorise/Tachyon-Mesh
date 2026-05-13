# Proposal: Dynamic OpenAPI Registry via System FaaS

## Context
The P2 audit highlighted the lack of an auto-generated OpenAPI/Swagger specification for the 34+ HTTP routes of `core-host`. Currently, `tachyon-client` hardcodes these routes and structures, leading to inevitable drift between the API, the client, the UI, and the MCP server.

## Problem
1. **API Drift:** The `validate_cross_layer.sh` script is a band-aid; we lack a Single Source of Truth for the API contract.
2. **Core Bloat Risk:** Generating dynamic OpenAPI specifications and serving documentation UIs (Swagger/Redoc) natively inside `core-host` violates our design principle of keeping the host orchestrator lean.

## Proposed Solution
1. **Zero-Cost Abstraction (`core-host`):** Use `utoipa` macros in `core-host` to statically generate the base OpenAPI JSON at compile time. Expose this raw string via an internal Host API.
2. **WASM Offloading (`system-faas-openapi`):** Create a new system FaaS module. It will be routed to `/admin/docs/*` and `/admin/schema/openapi.json`. It will fetch the base schema from the host, optionally merge it with user-deployed FaaS schemas, and serve both the JSON and the Swagger UI HTML.
3. **Client Codegen:** Refactor `tachyon-client` constants to be verifiable or auto-generated against this OpenAPI spec.

## Impact
- **Architecture:** `core-host` remains strictly an orchestrator; API documentation and dynamic registry logic are safely sandboxed in WebAssembly.
- **Developer Experience:** Any developer (or AI agent via MCP) can hit `/admin/docs` to interactively test the cluster API.