# network Specification Delta

## ADDED Requirements

### Requirement: Safetensors Header Parsing For QUIC Replication
The HTTP/3 QUIC replication path SHALL parse the `.safetensors` 8-byte little-endian header length prefix and JSON header before allocating tensor storage.

#### Scenario: Header announces tensor offsets
- **WHEN** a `.safetensors` stream begins with a valid length prefix and JSON header
- **THEN** the host extracts the header without reading the full tensor payload into memory
- **AND** computes the expected total file size from tensor `data_offsets`

### Requirement: Sparse Mmap Tensor Landing Zone
The host SHALL prepare an NVMe-backed sparse file and mutable memory map sized to the expected `.safetensors` artifact before chunk reconstruction.

#### Scenario: Receiver prepares target storage
- **WHEN** the parsed header reports the expected total artifact size
- **THEN** the receiver creates or truncates the target file to that size
- **AND** maps it as writable memory for offset-addressed chunk writes

### Requirement: Offset Tagged QUIC Tensor Chunks
The QUIC transfer implementation SHALL encode each tensor chunk with an absolute file offset and payload length so independent streams can reconstruct the file out of order.

#### Scenario: Chunk arrives out of order
- **WHEN** a chunk arrives with offset and length metadata
- **THEN** the receiver writes the payload into the matching mmap slice
- **AND** rejects chunks whose declared range exceeds the mapped file

### Requirement: Safetensors Integrity Verification Before Readiness
The replication path SHALL verify the reconstructed safetensors artifact against an expected cryptographic hash before exposing it to inference.

#### Scenario: Reconstructed artifact hash is checked
- **WHEN** all chunks for a safetensors artifact have been written
- **THEN** the host computes the artifact hash
- **AND** marks the artifact usable only when the hash matches the expected manifest value
