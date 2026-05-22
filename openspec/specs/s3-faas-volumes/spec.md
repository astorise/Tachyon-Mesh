# s3-faas-volumes Specification

## Purpose
TBD - created by archiving change s3-faas-volumes. Update Purpose after archive.
## Requirements
### Requirement: integrity.lock accepts S3 volume type for user routes
The integrity schema SHALL accept `type: "s3"` as a valid volume type for `role: user` routes. The `host_path` field SHALL contain a valid S3 URL in the format `s3://bucket/prefix`.

#### Scenario: S3 volume declared in integrity.lock validates correctly
- **WHEN** a route with `type: user` declares a volume with `type: "s3"` and `host_path: "s3://my-bucket/datasets/v1"`
- **THEN** integrity verification passes
- **AND** the volume is registered for pre/post-exec S3 sync

#### Scenario: S3 volume on system route is rejected
- **WHEN** a route with `type: "system"` declares a volume with `type: "s3"`
- **THEN** integrity verification fails with a clear schema violation error

#### Scenario: Malformed S3 URL is rejected
- **WHEN** a route volume declares `type: "s3"` with `host_path: "not-an-s3-url"`
- **THEN** integrity verification fails with a descriptive error naming the invalid host_path

### Requirement: Guest receives S3 volume contents as a WASI preopened directory before execution
Before the guest WASM executes, core-host SHALL download all objects under the configured S3 prefix to a per-invocation temporary directory and preopen it at the declared `guest_path`.

#### Scenario: Read-write S3 volume is downloaded before guest execution
- **WHEN** a sealed route has an S3 volume with `readonly: false`
- **AND** the S3 prefix contains objects
- **THEN** the guest receives those objects as files in a WASI preopened directory at `guest_path`
- **AND** the guest can read and write files in that directory during execution

#### Scenario: Read-only S3 volume is downloaded and write-protected
- **WHEN** a sealed route has an S3 volume with `readonly: true`
- **AND** the S3 prefix contains objects
- **THEN** the guest receives those objects but any write attempt returns a permission error

### Requirement: Modified S3 volume contents are uploaded to S3 after successful guest execution
After a guest completes execution successfully with a read-write S3 volume, core-host SHALL upload all files from the temporary directory back to the configured S3 prefix.

#### Scenario: Guest writes a file and it persists to S3
- **WHEN** a guest writes `result.json` to its S3-backed volume at `guest_path`
- **AND** execution completes without error
- **THEN** `result.json` is uploaded to `s3://bucket/prefix/result.json`
- **AND** a subsequent invocation sees `result.json` in its preopened directory

#### Scenario: Failed execution does not upload to S3
- **WHEN** a guest execution fails (trap, timeout, OOM)
- **THEN** the temporary directory is discarded without uploading to S3
- **AND** the S3 prefix retains its state from before the invocation

### Requirement: S3 volume temporary directories are cleaned up after each invocation
Core-host SHALL delete the per-invocation temporary directory after S3 upload (or after discard on failure), regardless of execution outcome.

#### Scenario: Temp dir is removed after execution
- **WHEN** a guest invocation with an S3 volume completes
- **THEN** the temporary directory under `$TMPDIR/tachyon-s3-vol-*/` is removed
- **AND** no orphaned directories accumulate across invocations

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

