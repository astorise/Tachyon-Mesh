# Proposal: Reactive Materialized Views (CQRS via CDC)

## Why
Applications built on the Tachyon BaaS often require complex UI dashboards (e.g., a user profile, their 5 recent posts, and 3 notifications). In a normalized relational or graph model, generating this payload requires multiple `get` operations, `batch_get`, or heavy joins, consuming compute resources on every page load.

1. **Read Amplification:** Heavy read workloads executing complex joins at runtime will starve the `core-host` of CPU cycles.
2. **Coupled Scaling:** If reads and writes compete for the same execution paths, traffic spikes (e.g., a viral post) degrade the performance of background writes.
3. **Wasted Compute:** Computing the same dashboard 10,000 times for 10,000 visitors when the underlying data hasn't changed is structurally inefficient.

## What Changes
Separate the Write Path (Commands) from the Read Path (Queries) using the CQRS pattern.
1. **Event-Driven Materialization:** Leverage the existing `tachyon:storage/data-events` (CDC) WIT contract. Dedicated Wasm FaaS modules (`system-faas-view-builder`) subscribe to relevant data mutations.
2. **Background Computation:** When a mutation occurs (e.g., a new comment is added), the FaaS wakes up, fetches the necessary related data, and computes a static, ready-to-serve JSON document (the Materialized View).
3. **O(1) Reads:** The compiled JSON is saved to RedDB under a dedicated subspace (e.g., `V:dashboard:user123`). Client applications read this single key instantly, completely bypassing the SQL/Graph FaaS engines.

## Impact
- **Infinite Read Scalability:** Complex queries are reduced to $O(1)$ K/V lookups.
- **Compute Offloading:** CPU is only used exactly once when data changes, rather than every time data is read.
- **Edge Caching:** Pre-computed views can be trivially cached at the CDN or reverse-proxy layer.
