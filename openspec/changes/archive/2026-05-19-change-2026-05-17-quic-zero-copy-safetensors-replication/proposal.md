# Proposal: QUIC-based Zero-Copy Safetensors Replication

## Why
As Tachyon-Mesh scales its decentralized AI orchestration (Tier 3 nodes), the distribution of multi-gigabyte foundation models and dynamic LoRA adapters becomes a primary bottleneck. Distributing these `.safetensors` artifacts over standard TCP (or standard HTTP/1.1) causes head-of-line blocking, high memory overhead during transfer, and slow startup times for FaaS agents waiting for weights to load. TACHYON-RFC-003 correctly identifies the need for a UDP-based, memory-mapped replication protocol optimized for the `safetensors` structure.

1. **Inefficient Transfers:** Standard file transfers load data into RAM before flushing to NVMe, consuming valuable memory on edge nodes.
2. **Sequential Blocking:** Tensors at the end of a file cannot be loaded until the entire file arrives.
3. **Reinventing the Wheel:** Creating a custom Reliable UDP protocol (as originally proposed in RFC-003) ignores our existing, robust HTTP/3 (QUIC) infrastructure.

## What Changes
We will adapt the core concepts of RFC-003 to leverage our existing QUIC implementation, achieving the same zero-copy goals with enterprise-grade reliability and security.

1. **QUIC Safetensors Streams:** Utilize QUIC's multi-stream capabilities. The sender transmits the `.safetensors` JSON header on a primary, reliable control stream.
2. **Sparse mmap Allocation:** Upon receiving the header, the receiving `core-host` extracts the expected file size and tensor offsets, creates a sparse file on NVMe, and `mmap`s it into memory.
3. **Concurrent Chunking (Multiplexing):** The sender chunks the tensor data and transmits them across *multiple independent QUIC streams* concurrently. Because each chunk knows its absolute offset (derived from the header), the receiver uses `std::ptr::copy_nonoverlapping` to write the payload directly into the correct location in the `mmap` buffer.
4. **Security Boundary:** Before marking a tensor as "ready" for inference via the `tachyon:inference` WIT, the host verifies the chunk's integrity against the cryptographic hash provided in the OCI manifest.

## Impact
- **Near-Instant Readiness:** FaaS functions can begin `load-layer` operations on specific tensors *while* the rest of the file is still transferring, as long as the specific tensor's chunks have arrived.
- **Zero RAM Overhead:** Data flows directly from the NIC buffer to the NVMe-backed memory map.
