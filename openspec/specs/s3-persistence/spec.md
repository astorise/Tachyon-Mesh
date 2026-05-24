# s3-persistence Specification

## Purpose
TBD - created by archiving change native-s3-persistence. Update Purpose after archive.
## Requirements
### Requirement: core-host restores all persistent state from S3 before serving requests
When the `s3-persistence` feature is enabled and S3 env vars are configured, `core-host` SHALL download all objects under the configured S3 prefix to the local data directory synchronously during startup, before binding the HTTP listener.

#### Scenario: Fresh pod restores auth state from S3
- **WHEN** a pod starts with `s3-persistence` enabled and `TACHYON_S3_BUCKET` configured
- **THEN** core-host downloads all files from `s3://<bucket>/<prefix>/` to `/data/` before accepting requests
- **AND** if the bucket contains `auth-state/admin.json`, the admin account is available immediately after startup

#### Scenario: S3 unavailable at startup
- **WHEN** the configured S3 endpoint is unreachable at startup
- **THEN** core-host logs a warning and continues with empty local state
- **AND** the HTTP server starts and serves requests normally

### Requirement: core-host flushes auth state to S3 immediately after each mutating auth operation
After any operation that modifies user records, sessions, or token state, core-host SHALL upload the changed auth-state directory to S3 before returning the HTTP response.

#### Scenario: Admin enrollment flushes auth state
- **WHEN** a client calls `POST /auth/signup/finalize` successfully
- **THEN** core-host uploads `/data/auth-state/` to S3 before returning the response
- **AND** the new user record is durable in S3 even if the pod crashes immediately after

#### Scenario: Login session flushes auth state
- **WHEN** a client calls `POST /auth/login/finalize` successfully
- **THEN** core-host uploads the session files to S3 before returning the response

### Requirement: core-host runs a periodic background flush for non-auth persistent files
core-host SHALL spawn a background task that uploads all files under the local data directory to S3 every 5 minutes, covering files not flushed by event-driven auth operations (tachyon.db, host-identity.key, certs).

#### Scenario: Background flush runs on schedule
- **WHEN** the `s3-persistence` feature is enabled and 5 minutes have elapsed since the last flush
- **THEN** core-host uploads all files under `/data/` to S3
- **AND** the flush does not block request handling

### Requirement: S3 credentials are configured via environment variables
The `s3-persistence` feature SHALL read all S3 connection parameters from environment variables so that credentials are never embedded in the integrity manifest or container image.

#### Scenario: All required env vars present
- **WHEN** `TACHYON_S3_ENDPOINT`, `TACHYON_S3_BUCKET`, `TACHYON_S3_ACCESS_KEY_ID`, and `TACHYON_S3_SECRET_ACCESS_KEY` are all set
- **THEN** core-host initializes the S3 backend and enables persistence

#### Scenario: S3 env vars absent
- **WHEN** `TACHYON_S3_BUCKET` is not set
- **THEN** core-host starts without S3 persistence (local-only mode)
- **AND** no error is logged at startup

### Requirement: s3-persistence feature compiles on musl and glibc without native dependencies
The `object_store` crate with `aws` feature SHALL compile successfully in both musl (Alpine) and glibc build environments without requiring cmake, nasm, or other native build tools beyond the Rust toolchain.

#### Scenario: s3-persistence compiles in Alpine musl builder
- **WHEN** `cargo build -p core-host --features s3-persistence` runs in the Alpine musl build stage
- **THEN** the build succeeds without native dependency errors

#### Scenario: s3-persistence compiles in Ubuntu glibc CI runner
- **WHEN** `cargo check -p core-host --features s3-persistence` runs in the standard CI job
- **THEN** the check passes without errors


## Requirements (s3-storage-backup)

### Requirement: build_s3_store is available as a shared primitive for volume backup
The `build_s3_store(bucket)` helper function in `volumes.rs` SHALL be reused by the volume backup module to build per-bucket ObjectStore instances using the same `TACHYON_S3_*` environment variables, without duplicating credential reading logic.

#### Scenario: Volume backup reuses existing S3 connection configuration
- **WHEN** the volume backup module creates an S3 client for backup operations
- **THEN** it calls `build_s3_store(bucket)` with the target bucket
- **AND** uses the same `TACHYON_S3_ENDPOINT`, `TACHYON_S3_ACCESS_KEY_ID`, `TACHYON_S3_SECRET_ACCESS_KEY`, and `TACHYON_S3_REGION` environment variables as the persistence backend
