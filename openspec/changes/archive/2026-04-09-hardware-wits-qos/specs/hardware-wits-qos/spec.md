## ADDED Requirements

### Requirement: Accelerator access is exposed through explicit hardware WITs with QoS-aware scheduling
The platform SHALL expose hardware-specific accelerator capabilities through explicit WIT namespaces and SHALL schedule those workloads according to declared QoS classes.

#### Scenario: A target declares a device-specific accelerator and QoS class
- **WHEN** a route requests a specific accelerator interface and QoS policy
- **THEN** the host routes the work into the matching hardware queue
- **AND** prioritizes execution according to the declared QoS class
