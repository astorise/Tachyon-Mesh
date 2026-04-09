# Hardware WITs and QoS

## Purpose
Define how Tachyon exposes explicit accelerator imports for component guests and prioritizes inference work according to declarative QoS classes.

## Requirements
### Requirement: Accelerator access is exposed through explicit hardware WITs with QoS-aware scheduling
The platform SHALL expose hardware-specific accelerator capabilities through explicit WIT namespaces and SHALL schedule those workloads according to declared QoS classes.

#### Scenario: A model binding declares an accelerator class and QoS
- **WHEN** a sealed route binds a model alias to a specific device and QoS policy
- **THEN** the host resolves explicit accelerator imports through the matching hardware queue
- **AND** prioritizes execution according to the declared QoS class

#### Scenario: Unavailable accelerators are not linked into the component host
- **WHEN** the host runtime does not expose a specific accelerator class such as TPU
- **THEN** the corresponding accelerator interface is omitted from the Wasmtime linker
- **AND** components requiring that interface fail fast during instantiation instead of silently falling back

#### Scenario: Realtime inference preempts batch backlog on the same accelerator
- **WHEN** realtime and batch jobs compete for the same accelerator queue
- **THEN** the scheduler chooses the higher QoS request for the next batch
- **AND** waiting work is aged so lower-priority jobs eventually execute
