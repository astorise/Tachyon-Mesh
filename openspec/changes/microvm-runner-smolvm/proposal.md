# Proposal: MicroVM Execution Engine via SmolVM

## Context
Tachyon Mesh relies on WebAssembly (Wasmtime) for sub-millisecond, highly secure FaaS execution. While Wasm is perfect for pure compute and routing, it struggles with languages relying on native C-extensions (like Python's `numpy` or `torch`) or AI Agents that require standard OS capabilities (shell access, complex file systems). To support these "Enterprise & Agentic" workloads at the Edge without sacrificing security, we need a lightweight virtualization layer.

## Proposed Solution
We will integrate **SmolVM** as a secondary, alternative execution backend managed by a dedicated system module: `system-faas-microvm-runner`.
1. **Schema Extension:** The `integrity.lock` will support a new runtime type. Developers can flag a function as `type: "microvm"` and provide a `.smolmachine` image.
2. **The Runner:** The `core-host` will route requests for these specific functions to `system-faas-microvm-runner`. This Rust module uses the SmolVM SDK to boot a tiny Alpine Linux Kernel via KVM/Firecracker in ~200ms.
3. **Transparent IPC:** The runner will proxy Tachyon's internal events and HTTP/3 requests into the MicroVM via a virtual serial port or vsock, making the MicroVM act exactly like a standard Wasm FaaS from the outside.
4. **Scale-to-Zero Synergy:** We will leverage SmolVM's native snapshot/restore capabilities to hibernate inactive AI Agents to disk, perfectly mirroring our Wasm RAM Hibernation strategy.

## Objectives
- Support 100% of programming languages and native binaries at the Edge.
- Provide secure, disposable sandbox environments for Autonomous AI Agents.
- Maintain strong isolation (Hardware-level Virtualization) without the bloat of standard Docker/Containerd runtimes.