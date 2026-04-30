# Implementation Tasks
- [x] Create a new workspace crate `ebpf-probes` for restricted L4 rewrite rule logic.
- [x] Write unit-tested L4 protocol/port rewrite lookup logic in restricted Rust.
- [x] In `core-host`, add an `--accel=ebpf` CLI flag that fails gracefully outside a Linux/aya loader path.
- [x] Add the host control-plane hook that counts configured L4 routes and falls back to userspace routing until kernel map loading is enabled.
- [x] Capture the iperf3 throughput comparison as a follow-up benchmark requirement instead of fabricating local results.
