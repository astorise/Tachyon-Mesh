## ADDED Requirements

### Requirement: Core host tracks last_accessed timestamps for warm Wasm instances
The `core-host` SHALL track a `last_accessed` timestamp for every warm Wasm instance held in its pool, updating it on every invocation.

#### Scenario: Invocation refreshes the activity timestamp
- **WHEN** a warm Wasm instance handles a request
- **THEN** the host updates the instance's `last_accessed` timestamp to the current time
- **AND** the value is observable by the Hibernation Manager

### Requirement: Hibernation Manager freezes idle instances to disk
A background Hibernation Manager SHALL scan the warm pool for instances whose `last_accessed` is older than the configured idle threshold (default `5 minutes`), serialize their Wasm linear memory to persistent storage, and drop the instance from RAM.

#### Scenario: Idle instance is hibernated
- **WHEN** the Hibernation Manager finds a warm instance whose `last_accessed` is older than the idle threshold
- **THEN** it serialises the instance's linear memory to a snapshot on persistent storage
- **AND** drops the in-memory instance, freeing its RAM
- **AND** records the snapshot location in the module's pool metadata

### Requirement: Requests for hibernated modules thaw transparently from disk
When a request arrives for a module whose instance has been hibernated, the host SHALL load the snapshot from disk, allocate a new memory block, restore the linear memory contents, and resume execution without performing a full JIT recompilation cold start.

#### Scenario: Thaw is faster than full cold start
- **WHEN** a request targets a module whose only available state is a hibernation snapshot on disk
- **THEN** the host reads the snapshot and restores it into a freshly allocated linear memory
- **AND** resumes execution without re-running the full JIT compilation pipeline
- **AND** the client observes the response with only the configured thaw latency overhead, not a full cold-start latency
- **AND** repeated invocations after the thaw operate at warm-start latency
