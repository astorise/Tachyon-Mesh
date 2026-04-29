# Proposal: Event-Driven AuthZ Cache Invalidation

## Context
To ensure sub-millisecond latency, the `core-host` caches authorization decisions (RBAC) made by the `system-faas-authz` module. However, this introduces a classic caching vulnerability: stale state. If an administrator revokes a Personal Access Token (PAT) or changes a user's role via Tachyon Studio, the `core-host` will continue to grant access using the cached data until its Time-To-Live (TTL) expires. In a strict Zero-Trust environment, revocation must be instantaneous.

## Proposed Solution
We will leverage the Mesh's internal event bus (or data events) to implement **Active Cache Invalidation**:
1. When `system-faas-authz` processes a mutation (token revocation, role update, user ban), it emits a targeted `authz.cache.purge` event.
2. The `core-host` maintains a background subscriber listening to the `authz` event channel.
3. Upon receiving a purge event (containing a specific `token_hash` or `user_id`), the host immediately evicts the corresponding entries from its internal RBAC cache.

## Objectives
- Achieve near-instantaneous (sub-second) permission revocation across the Mesh.
- Maintain the high throughput of cached auth decisions without sacrificing security.
- Decouple the FaaS logic from the Host's internal cache mechanics using standard IPC events.