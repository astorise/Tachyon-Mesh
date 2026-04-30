# udp-layer4-routing Specification

## Purpose
Define UDP Layer 4 routing for raw datagram workloads such as VoIP, DNS, and game traffic.

## Requirements
### Requirement: UDP Layer 4 route dispatch
The host SHALL bind configured UDP Layer 4 listeners and dispatch datagrams to the sealed target route.

#### Scenario: UDP datagram reaches a bound listener
- **WHEN** a datagram arrives on a configured UDP Layer 4 port
- **THEN** the host resolves the target route and invokes the configured guest
- **AND** guest response datagrams are sent to their declared targets

### Requirement: eBPF fast-path fallback
The host SHALL fall back to userspace UDP routing when eBPF or XDP acceleration is unavailable.

#### Scenario: eBPF acceleration cannot initialize
- **WHEN** the host requests eBPF acceleration on an unsupported build or host
- **THEN** UDP routing continues through the userspace listener path
