## ADDED Requirements

### Requirement: S3 volume commit honors the volume write_mode
The S3 volume commit phase SHALL honor the volume's `consistency.write_mode` and select the appropriate upload strategy.

#### Scenario: LastWriteWins uploads unconditionally
- **WHEN** a volume has `consistency.write_mode: "last_write_wins"` (or no consistency block)
- **AND** the guest completes successfully with modified files
- **THEN** the commit uploads all files with simple PUT requests
- **AND** any concurrent writer's changes are silently overwritten

#### Scenario: OptimisticEtag aborts commit on conflict
- **WHEN** a volume has `consistency.write_mode: "optimistic_etag"`
- **AND** the commit phase issues conditional PUT requests with the originally observed ETag
- **AND** another writer has modified the object since download
- **THEN** the conditional PUT returns HTTP 412 Precondition Failed
- **AND** the runtime returns an error to the invocation caller
- **AND** the local temp directory is cleaned up

#### Scenario: PessimisticLock acquires distributed lock around invocation
- **WHEN** a volume has `consistency.write_mode: "pessimistic_lock"`
- **AND** an invocation arrives
- **THEN** the runtime acquires a `DistributedLock` keyed on `s3-vol:<route>:<guest_path>` before downloading the volume
- **AND** holds the lock until upload completes
- **AND** releases the lock unconditionally after upload (success or failure)
