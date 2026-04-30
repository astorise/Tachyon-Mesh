## ADDED Requirements

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

### Requirement: eBPF probe rules are represented independently
The workspace SHALL provide a dedicated `ebpf-probes` crate that models Layer 4 rewrite keys and targets independently from the userspace host so packet rewrite behavior can be unit-tested before kernel loading is enabled.

#### Scenario: Probe rule lookup matches protocol and port
- **GIVEN** a TCP rewrite rule for destination port 8080
- **WHEN** the probe lookup receives a TCP packet key for port 8080
- **THEN** it returns the configured rewrite target
- **AND** a UDP key for the same port does not match the TCP rule
