# Proposal: Tachyon BaaS Data Fabric & Dynamic AST Mutation

## Why
Tachyon-Mesh currently operates as a highly optimized L4/L7 networking, execution, and AI inference engine. To empower developers to build fully decentralized applications without external database dependencies, Tachyon must evolve into a comprehensive Backend-as-a-Service (BaaS). This requires integrating a robust data layer (RedDB) while maintaining the strict zero-overhead, isolated architecture of the core-host.

1. **Coupled Security:** Traditional Row-Level Security (RLS) requires heavy centralized database processing and monolithic parsers, which bloat core systems.
2. **Migration Downtime:** Standard DDL migrations lock tables and cause cascading latency failures in distributed meshes.
3. **Storage vs. Compute Contention:** Serving large media blobs or running heavy OLAP analytical queries directly on the primary K/V store starves the critical path of CPU and memory resources.

## What Changes
Implement a decentralized BaaS architecture using domain-isolated FaaS and SmolVM components:
1. **Decentralized IAM (Biscuit + SmolVM):** Utilize cryptographic Biscuit tokens for Datalog-based capabilities. Delegate heavy AST query parsing and Datalog injection to an isolated SmolVM, generating sanitized queries for the core-host.
2. **Write-on-Read Migrations:** Use ephemeral Wasm shims to transform deprecated schema structures in-memory during read operations, migrating data organically without DDL locks.
3. **Internal WIT Bypasses:** Internal business logic FaaS modules use strongly-typed WIT bindings to query data, completely bypassing AST parsing for microsecond latency.
4. **RustFS Media & OLAP FaaS:** Extend RustFS for smart object storage (bypassing zstd for pre-compressed media). Spawn strictly isolated, ephemeral FaaS instances for zero-copy HTTP Range streaming and heavy OLAP cold-storage aggregations.

## Impact
Transforms Tachyon-Mesh into a self-sufficient, highly scalable decentralized application backend with zero compromise on the `core-host`'s performance or memory safety.
