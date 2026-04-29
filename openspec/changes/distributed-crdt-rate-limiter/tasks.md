# Implementation Tasks

## Phase 1: Distributed Limiter FaaS
- [x] Bootstrap `systems/system-faas-dist-limiter`.
- [x] Implement a simplified CRDT counter (or use a crate like `rust-crdt`).
- [x] Integrate with `system-faas-gossip` IPC to send/receive state updates.

## Phase 2: Core Host Integration
- [x] Update `IntegrityRoute` struct to support the `distributed_rate_limit` boolean flag.
- [ ] In the HTTP/3 request pipeline, inject a call to the distributed limiter FaaS if the flag is enabled for the matched route.

## Phase 3: Resilience (Fail-Open)
- [ ] Implement a timeout and error handling for the IPC call. If `system-faas-dist-limiter` does not respond within 5ms, the `core-host` MUST ignore the distributed check and rely solely on the local limiter.

## Phase 4: Validation
- [ ] **Test Convergence:** Start 3 nodes. Send 50 requests to Node A and 60 requests to Node B. Verify that Node C (which received 0 requests) eventually reports 110 requests for that IP and starts blocking.
- [ ] **Test Overhead:** Verify that routes with `distributed_rate_limit: false` do not suffer any latency increase.
