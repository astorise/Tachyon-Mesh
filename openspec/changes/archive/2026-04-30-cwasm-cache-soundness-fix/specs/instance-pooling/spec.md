## ADDED Requirements

### Requirement: Cwasm cache entries are bound to Wasmtime compatibility
The host MUST include the active Wasmtime engine precompile compatibility hash in every serialized Cwasm cache key.

#### Scenario: Engine configuration changes
- **GIVEN** a Wasm module has already been cached as precompiled Cwasm
- **WHEN** the host restarts with a different Wasmtime version or engine configuration
- **THEN** the host computes a different compatibility hash
- **AND** it does not deserialize the stale Cwasm bytes with the new engine

### Requirement: Stale Cwasm cache is purged at boot
The host MUST compare the active engine compatibility hash with persisted cache metadata before loading any cached Wasm module.

#### Scenario: Stored compatibility hash differs from current engine
- **GIVEN** the cache metadata stores a previous engine compatibility hash
- **WHEN** the host boots with a different current compatibility hash
- **THEN** it clears the Cwasm cache bucket
- **AND** it stores the current compatibility hash before route prewarming can load cached Cwasm

### Requirement: Matching Cwasm cache is retained
The host MUST retain existing Cwasm cache entries when the persisted compatibility hash matches the current engine.

#### Scenario: Host restarts without engine changes
- **GIVEN** the cache metadata matches the active engine compatibility hash
- **WHEN** the host boots
- **THEN** it leaves Cwasm cache entries intact
- **AND** repeat module loads can reuse the cached precompiled bytes
