# Implementation Tasks

- [ ] **Task 1: Safetensors Header Parsing**
  - Implement a lightweight parser in `core-host` that extracts the 8-byte length prefix and the JSON header block from a `.safetensors` file stream without loading the rest of the file.

- [ ] **Task 2: Mmap Allocation Engine**
  - Create the `prepare_tensor_landing_zone` utility using `memmap2` and ensure it gracefully handles sparse file creation across OS targets (Linux `set_len`, Windows equivalents).

- [ ] **Task 3: QUIC Multiplexing Logic**
  - Update `core-host/src/server_h3.rs`.
  - Implement the logic to split a local `.safetensors` file into offset-tagged chunks and transmit them across concurrent QUIC streams.
  - Implement the receiver logic that reconstructs the file directly into the `MmapMut` slice based on the received offsets.

- [ ] **Task 4: Integrity Enforcement**
  - Integrate a fast hashing mechanism (e.g., BLAKE3) to verify that the chunks written to the `mmap` match the expected hash defined in the model's OCI manifest, protecting against network corruption or malicious injection before inference begins.
