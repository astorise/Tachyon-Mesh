# ram-hibernation Specification

## Purpose
TBD - created by archiving change ram-hibernation. Update Purpose after archive.
## Requirements
### Requirement: RAM volumes can hibernate after an idle timeout
The integrity manifest SHALL allow RAM-backed volumes to declare an idle timeout and an `eviction_policy` of `hibernate` so the host can swap idle state to persistent storage instead of deleting it.

#### Scenario: A RAM volume is configured for hibernation
- **WHEN** a volume definition sets `type` to `ram`
- **AND** provides `idle_timeout`
- **AND** sets `eviction_policy` to `hibernate`
- **THEN** the host tracks the volume as hibernation-capable state instead of ephemeral-only state

### Requirement: Idle RAM volumes transition to an on-disk snapshot
The host SHALL mark an idle hibernating volume as unavailable for new mounts, coordinate a snapshot through the storage broker, and release the RAM allocation only after persistence succeeds.

#### Scenario: Idle timeout triggers swap-out
- **WHEN** the idle timeout expires for a hibernating RAM volume
- **THEN** the host transitions the volume state to `Hibernating`
- **AND** requests the storage broker to persist the RAM contents to disk
- **AND** transitions the volume state to `OnDisk` only after the snapshot is durably written

### Requirement: Requests resume after an on-disk volume is restored
The host SHALL suspend request execution for targets that require a volume in `OnDisk` state until the storage broker restores that volume back into RAM.

#### Scenario: A request arrives while a required volume is on disk
- **WHEN** a request targets a function that depends on a volume in `OnDisk` state
- **THEN** the host queues the request asynchronously
- **AND** asks the storage broker to restore the volume into RAM
- **AND** resumes the queued request only after the volume returns to `Active`

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

