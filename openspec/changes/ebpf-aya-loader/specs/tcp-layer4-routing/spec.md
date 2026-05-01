## MODIFIED Requirements

### Requirement: L4 fast-path acceleration is opt-in
The `core-host` SHALL expose an `--accel` startup option with `userspace` as the default and `ebpf` as an explicit opt-in mode for Layer 4 fast-path experiments.

#### Scenario: Host starts without acceleration flag
- **GIVEN** the operator starts `core-host` without `--accel`
- **WHEN** Layer 4 bindings are configured
- **THEN** the host keeps the existing userspace TCP and UDP routing path

#### Scenario: Host starts with eBPF acceleration on an unsupported platform
- **GIVEN** the operator starts `core-host --accel ebpf`
- **AND** the platform cannot load the eBPF fast-path
- **WHEN** the host initializes Layer 4 routing
- **THEN** startup continues with userspace routing
- **AND** the fallback is logged

#### Scenario: Host starts with eBPF acceleration and the Aya loader is available
- **GIVEN** the operator starts `core-host --accel ebpf`
- **AND** a compiled Tachyon eBPF artifact is available to the host build
- **WHEN** the host initializes Layer 4 routing
- **THEN** it loads the embedded eBPF object through the Aya loader
- **AND** startup continues with the eBPF fast-path initialized
