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

