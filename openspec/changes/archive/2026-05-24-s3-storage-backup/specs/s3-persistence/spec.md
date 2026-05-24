## ADDED Requirements

### Requirement: build_s3_store is available as a shared primitive for volume backup
The `build_s3_store(bucket)` helper function in `volumes.rs` SHALL be reused by the volume backup module to build per-bucket ObjectStore instances using the same `TACHYON_S3_*` environment variables, without duplicating credential reading logic.

#### Scenario: Volume backup reuses existing S3 connection configuration
- **WHEN** the volume backup module creates an S3 client for backup operations
- **THEN** it calls `build_s3_store(bucket)` with the target bucket
- **AND** uses the same `TACHYON_S3_ENDPOINT`, `TACHYON_S3_ACCESS_KEY_ID`, `TACHYON_S3_SECRET_ACCESS_KEY`, and `TACHYON_S3_REGION` environment variables as the persistence backend
