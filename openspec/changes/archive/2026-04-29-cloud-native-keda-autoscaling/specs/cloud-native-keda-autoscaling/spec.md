## ADDED Requirements

### Requirement: system-faas-keda exports scaling signals via the KEDA External Scaler protocol
The Mesh SHALL provide a `system-faas-keda` module that implements the gRPC External Scaler interface defined by KEDA and exposes Tachyon Mesh scaling signals (such as buffer backlog depth and active FaaS counts) as KEDA metric values.

#### Scenario: KEDA polls the Tachyon external scaler
- **WHEN** the KEDA operator polls `system-faas-keda` over the External Scaler gRPC interface
- **THEN** the module reads the latest shared metrics state from `system-faas-buffer` and `core-host`
- **AND** returns metric values that reflect the current backlog and active worker counts
- **AND** does not push metrics outside of poll cycles, leaving the routing critical path untouched

### Requirement: Core host shares scaling signals with the KEDA adapter via low-overhead IPC
The `core-host` SHALL publish its worker-management signals to a shared metrics state that `system-faas-keda` can read without contending with request-path locks.

#### Scenario: Scaling signals do not slow down request handling
- **WHEN** the host is at peak request throughput
- **AND** `system-faas-keda` reads scaling signals concurrently for a KEDA poll
- **THEN** the host continues serving requests with no measurable latency increase attributable to the KEDA reader
- **AND** the metrics observed by `system-faas-keda` remain accurate within one polling interval
