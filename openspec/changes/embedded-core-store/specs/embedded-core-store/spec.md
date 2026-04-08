## ADDED Requirements

### Requirement: The host persists internal runtime state in an embedded ACID key-value store
The host SHALL use an embedded persistent store for crash-resilient internal state such as compiled module cache entries, certificate material, and hibernation data.

#### Scenario: The host needs to persist internal control data
- **WHEN** the runtime stores compiled artifacts, certificate records, or suspended state
- **THEN** it writes them into the embedded core store with crash-safe persistence semantics
- **AND** can recover them after a restart
