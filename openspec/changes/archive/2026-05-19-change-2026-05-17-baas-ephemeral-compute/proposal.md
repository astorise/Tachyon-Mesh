# Proposal: Ephemeral FaaS for BaaS OLAP and Media Streaming

## Why
Following the implementation of the Tachyon BaaS Data Fabric (RedDB & RustFS foundations), the mesh must now handle complex, resource-intensive operations: serving large, uncompressed media blobs (e.g., video streaming) and executing heavy analytical aggregations (OLAP) over cold storage snapshots.

1. **OLTP Starvation:** If the core-host or the primary RedDB Engine processes heavy `GROUP BY` aggregations over millions of rows, it will consume CPU cycles required for microsecond-latency transactional (OLTP) requests.
2. **Memory Exhaustion (OOM):** Streaming large video files by loading them entirely into memory will crash edge nodes with constrained RAM.
3. **Missing Range Support:** Modern web clients require `HTTP Range` requests to scrub through videos fluidly, logic which should not pollute the core L7 router.

## What Changes
Implement a strict "Ephemeral Compute" boundary utilizing specialized, short-lived Wasm FaaS modules:
1. **system-faas-olap-engine:** An ephemeral, memory-safe Wasm module spawned exclusively when an analytical query is detected. It uses new WIT bindings to stream `zstd` compressed chunks from RustFS, performs vectorized columnar aggregations locally in its Wasm linear memory, yields the final JSON/Arrow result, and immediately self-destructs.
2. **system-faas-media-server:** A lightweight Wasm proxy that handles `Accept-Ranges: bytes` HTTP headers. It uses WIT bindings to map a specific byte-range of an uncompressed RustFS object directly to the outbound network socket (zero-copy), buffering only the requested chunk.

## Impact
- **Absolute Isolation:** Heavy analytics and video streaming cannot impact the latency of standard database reads/writes.
- **Edge Efficiency:** Zero-copy media streaming allows even Tier-3 constrained nodes (like a Raspberry Pi) to serve 4K video over the mesh without RAM exhaustion.
