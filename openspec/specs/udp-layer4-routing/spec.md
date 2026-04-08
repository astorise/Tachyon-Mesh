# UDP Layer4 Routing

## Purpose
Define how Tachyon binds UDP ports to guest targets, exposes a packet-oriented guest contract, and protects the host with bounded asynchronous dispatch.

## Requirements
### Requirement: Hosts can bind UDP ports to packet-handling targets
The host manifest SHALL allow operators to map Layer 4 UDP ports to target names independently from TCP mappings.

#### Scenario: A UDP port is mapped to a target
- **WHEN** the host loads a UDP Layer 4 binding from the manifest
- **THEN** it starts a UDP socket listener for that port and associates inbound datagrams with the configured target

### Requirement: UDP targets expose a packet handler through WIT
UDP-capable targets SHALL implement a packet-handling WIT interface that receives the source address and payload and returns zero or more datagrams to send.

#### Scenario: A guest implements the UDP packet handler
- **WHEN** a UDP-bound guest is built for the runtime
- **THEN** it exports the packet handler with the agreed source address and payload contract

### Requirement: UDP datagrams are dispatched asynchronously with backpressure protection
The host SHALL dispatch inbound datagrams to guest instances asynchronously, send any returned datagrams back to the network, and drop excess traffic when a safe queue threshold is exceeded.

#### Scenario: A UDP datagram is processed successfully
- **WHEN** an inbound datagram is received for a UDP-bound target
- **THEN** the host invokes the guest packet handler with the datagram contents
- **AND** sends each returned response datagram back through the UDP socket

#### Scenario: The inbound UDP queue exceeds the safe threshold
- **WHEN** the runtime detects that queued UDP work exceeds its configured safety limit
- **THEN** it drops new datagrams rather than allowing unbounded memory growth
