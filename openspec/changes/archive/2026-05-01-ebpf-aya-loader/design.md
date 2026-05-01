# Design: Aya eBPF Loader

## Technical Implementation

1. **Target Artifact**: The eBPF program is compiled to a BPF target. The loader must embed the artifact located at `../../../target/bpfel-unknown-none/release/tachyon-ebpf` (relative to the workspace root/module).
2. **Library**: We use `aya` for eBPF management.
3. **Error Handling**: The function must return `anyhow::Result<aya::Bpf>` (or the crate's custom `CoreError` type if applicable) to ensure the Zero-Panic Policy is maintained.

## Code Blueprint
```rust
use aya::{Bpf, include_bytes_aligned};
use anyhow::Result;

pub(crate) fn load_ebpf_fast_path() -> Result<Bpf> {
    // Statically include the BPF ELF object
    let bpf_data = include_bytes_aligned!("../../../target/bpfel-unknown-none/release/tachyon-ebpf");

    // Load the BPF program
    let bpf = Bpf::load(bpf_data)?;
    Ok(bpf)
}
```

Note to agent: Adjust the relative path to the artifact based on the exact location of the file where this function is implemented (e.g., ebpf-probes/src/lib.rs or core-host/src/network/mod.rs).
