# Proposal: Distributed CRDT Rate Limiter

## Context
Tachyon Mesh nodes currently protect themselves using local rate limiting. However, in a distributed deployment, an attacker can spread a DDoS attack across multiple ingress nodes, staying under the local threshold of each while overwhelming the global capacity of the service. A centralized rate limiter would introduce a Single Point of Failure (SPOF) and is incompatible with Air-Gapped/P2P constraints.

## Proposed Solution
We will implement a **Distributed Rate Limiting layer powered by CRDTs**:
1. **System FaaS:** Create `system-faas-dist-limiter` which maintains request counters using a G-Counter (Grow-only Counter) or a LWW-Map (Last-Write-Wins Map) per IP/Window.
2. **Gossip Integration:** This FaaS will use `system-faas-gossip` to broadcast and merge counter states asynchronously across all nodes in the Mesh.
3. **Opt-In Policy:** The `integrity.lock` will allow flagging specific routes with `distributed_rate_limit: true`.
4. **Non-Blocking Check:** The `core-host` will query this FaaS via IPC. If the global threshold is exceeded, it denies the request. If the FaaS is unavailable or the network is partitioned, the host fails-open to its local rate limiter to maintain availability.

## Objectives
- Detect and mitigate coordinated DDoS attacks across the entire Mesh.
- Avoid centralized dependencies (Redis-less architecture).
- Ensure zero overhead for standard routes by keeping the feature opt-in.