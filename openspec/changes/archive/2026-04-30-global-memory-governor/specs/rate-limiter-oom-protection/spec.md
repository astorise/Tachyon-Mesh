## ADDED Requirements

### Requirement: Core host tracks global process memory pressure
The `core-host` SHALL maintain a global memory governor that samples process RSS, classifies pressure as normal, high, or critical against host-memory thresholds, and exposes the latest pressure state to request-path components.

#### Scenario: RSS crosses the soft threshold
- **GIVEN** the process RSS is at or above the configured soft threshold
- **WHEN** the memory governor samples RSS
- **THEN** it classifies global memory pressure as high

#### Scenario: RSS crosses the hard threshold
- **GIVEN** the process RSS is at or above the configured hard threshold
- **WHEN** the memory governor samples RSS
- **THEN** it classifies global memory pressure as critical

### Requirement: Memory-heavy host caches shed under pressure
The `core-host` SHALL evict warm in-memory Wasmtime module entries when global memory pressure is high or critical so requests can fall back to the persisted cwasm cache without keeping idle modules resident.

#### Scenario: Governor observes high pressure
- **GIVEN** the runtime instance pool contains warm module entries
- **WHEN** the memory governor reports high or critical pressure
- **THEN** the runtime invalidates warm instance-pool entries

### Requirement: Buffers reject new work during critical pressure
The request buffering path SHALL reject new buffered work with `503 Service Unavailable` when global memory pressure is critical.

#### Scenario: Route is saturated during critical pressure
- **GIVEN** a route has no available execution permit
- **AND** global memory pressure is critical
- **WHEN** a request would otherwise be queued for buffered execution
- **THEN** the host rejects the request with `503 Service Unavailable`
