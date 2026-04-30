# Proposal: eBPF Fast-Path Option

## Context
For Layer 4 routing (`tcp-layer4-routing`, `udp-layer4-routing`), bringing network packets all the way up to userspace (inside Tachyon Mesh) just to forward them to another IP/Port wastes CPU cycles and adds microsecond latency.

## Proposed Solution
We will implement an optional `--accel=ebpf` flag for Linux hosts.
1. **XDP Program:** We will write a tiny eBPF program (in restricted Rust using `aya-rs`) that attaches to the host's Network Interface Card (NIC) via XDP (eXpress Data Path).
2. **eBPF Maps:** Tachyon Mesh will populate an eBPF Map (a shared memory hash table in the kernel) with the Layer 4 routing rules defined in `integrity.lock`.
3. **Kernel-Bypass:** When a packet hits the NIC, the eBPF program rewrites the destination IP/Port in the packet header and immediately re-transmits it. The packet *never reaches userspace*.

## Objectives
- Achieve line-rate packet forwarding (millions of packets/sec) with negligible CPU usage.
- Match or exceed the performance of Cilium and hardware load balancers for pure L4 traffic.