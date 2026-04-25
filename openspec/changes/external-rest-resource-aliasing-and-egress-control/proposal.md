# Proposal: Unified Resource Mapping & Location Transparency

## Context
FaaS modules often depend on other services, whether they are other FaaS modules within the same Mesh (internal) or third-party APIs (external). Currently, these are handled via different protocols or domain conventions, forcing the developer to know the location of a resource at compile-time.

## Proposed Solution
We will implement **Location Transparency** by abstracting all service dependencies into a single `resources` map within the `integrity.lock`. 

A FaaS module will always call a resource via a logical URI (e.g., `http://mesh/billing-api`). The `core-host` will resolve this logical name using the signed manifest to decide if it should route the call to:
1. An **Internal** Wasm module (IPC).
2. An **External** HTTPS endpoint (Egress Proxy).

## Objectives
- **Agnostic FaaS:** Move a service from external (SaaS) to internal (Wasm) or vice-versa without changing or recompiling the dependent FaaS code.
- **Unified Security:** Every outbound call (internal or external) is governed by the same signed contract.
- **Simplified Development:** Developers use a consistent naming convention regardless of the underlying infrastructure.