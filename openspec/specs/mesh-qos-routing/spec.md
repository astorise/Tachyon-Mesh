# Mesh QoS Routing

## Purpose
Define how Tachyon combines per-accelerator queue depth with QoS policy so real-time workloads can overflow to healthier peers sooner than batch workloads.

## Requirements
### Requirement: Telemetry exposes per-accelerator queue depth by QoS tier
The host SHALL publish queue depth for each accelerator and QoS tier so control-plane components can distinguish real-time backlog from standard and batch backlog.

#### Scenario: The gossip controller reads accelerator pressure
- **WHEN** the control plane reads host telemetry
- **THEN** it receives queue depth for CPU, GPU, NPU, and TPU split into real-time, standard, and batch tiers
- **AND** can include those counters in mesh routing decisions

### Requirement: Real-time hardware workloads overflow to healthier peers ahead of batch work
The mesh router SHALL use the requested workload QoS and target accelerator when deciding whether to keep work local or forward it to a healthier peer.

#### Scenario: A real-time GPU request arrives while the local GPU queue is backed up
- **WHEN** the local real-time GPU queue has backlog and a healthier peer advertises a lower real-time GPU queue depth
- **THEN** the router forwards the request to that peer before instantiating the local FaaS
- **AND** preserves the route path while using the mesh override entry for the QoS-specific target

### Requirement: Batch workloads avoid remote overflow until local pressure is critical
The mesh router SHALL prefer local execution for batch workloads and only divert them away from the local node when local buffering or saturation thresholds are reached.

#### Scenario: Batch GPU traffic builds backlog on the local node
- **WHEN** batch GPU requests accumulate but the local node has not reached the critical overflow threshold
- **THEN** the router keeps the requests local
- **AND** does not consume cluster network bandwidth by forwarding them to peers
