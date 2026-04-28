## ADDED Requirements

### Requirement: Core host watches integrity.lock and atomically swaps the runtime
The `core-host` SHALL spawn a background file watcher (using the `notify` crate) that monitors the active `integrity.lock` file, validates any new manifest, and replaces the live `IntegrityRuntime` via an `ArcSwap` store without dropping in-flight requests.

#### Scenario: Configuration change is applied without dropping active streams
- **WHEN** a long-running HTTP/3 download or WebSocket session is active against the host
- **AND** the `integrity.lock` file is modified or atomically replaced on disk
- **THEN** the watcher loads and validates the new manifest
- **AND** the host calls `state.runtime.store(new_runtime)` to swap the pointer
- **AND** the in-flight request continues to completion using the previously loaded runtime
- **AND** subsequent new requests are served by the freshly loaded runtime without a process restart

### Requirement: Hot-reload tolerates corrupted or invalid manifests
If a reload candidate fails validation, the `core-host` SHALL retain the last known good `IntegrityRuntime` rather than crashing.

#### Scenario: Corrupted integrity.lock is rejected
- **WHEN** the watcher observes a write to `integrity.lock`
- **AND** the file fails JSON parsing or signature/integrity validation
- **THEN** the host logs a structured error
- **AND** the host continues serving traffic using the previously active runtime
- **AND** the host process remains alive
