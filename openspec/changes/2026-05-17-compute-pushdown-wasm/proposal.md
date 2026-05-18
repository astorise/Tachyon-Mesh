# Proposal: Compute Pushdown via Embedded Wasm Filters

## Why
In the current decoupled BaaS architecture, the Logical Plane (Wasm FaaS like `system-faas-sql-engine`) reads data from the Storage Plane (RedDB). When a user executes a query with a highly selective `WHERE` clause over an unindexed column, the FaaS must request a full range scan. RedDB sends millions of records over the mesh network to the FaaS, which discards 99% of them in memory.

1. **Network Saturation:** Moving gigabytes of discarded data across the mesh destroys network bandwidth.
2. **Memory Exhaustion:** FaaS nodes must buffer massive data streams, risking Out-Of-Memory (OOM) crashes.
3. **Latency:** The physical speed of the network becomes the absolute bottleneck for analytics and table scans.

## What Changes
Bring the compute to the data.
1. **Filter Contract:** Define a micro-WIT contract (`tachyon:storage/pushdown-filter`).
2. **Wasm Injection:** When a FaaS initiates a `scan` or `batch_get`, it can optionally attach a pre-compiled, tiny Wasm bytecode payload (the filter).
3. **Local Execution:** RedDB instances embed a constrained Wasmtime runner. For every K/V pair read from the NVMe disk, RedDB executes the Wasm filter locally.
4. **Network Pruning:** Only the K/V pairs that return `true` from the filter are serialized and transmitted over the network back to the requesting FaaS.

## Impact
- **Network I/O Reduction:** Network traffic for filtered scans is reduced by up to 99.9%.
- **Extreme Speed:** Data is filtered at the speed of the local NVMe bus.
- **Security:** Because the filter is pure Wasm, it remains perfectly sandboxed and cannot compromise the RedDB storage node.
