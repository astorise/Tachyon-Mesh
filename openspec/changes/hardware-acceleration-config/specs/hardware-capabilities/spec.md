# hardware-capabilities Delta

## ADDED Requirements

### Requirement: Hardware accelerators MUST be declaratively allocatable
The control plane SHALL use the declarative GitOps schema to negotiate the allocation of TPUs, GPUs, and eBPF network maps with the host operating system.

#### Scenario: Configuring eBPF XDP in generic mode
- **WHEN** the `system-faas-config-api` receives a configuration setting `ebpf_xdp.mode` to `generic`
- **THEN** the `core-host` gracefully reloads the eBPF probes
- **AND** attaches them to the generic network stack (SKB) instead of the native driver queue, preventing compatibility panics on older hardware.
