# turboquant-kv Delta

## ADDED Requirements

### Requirement: KV Partitions MUST be provisioned dynamically
The underlying Turboquant embedded database SHALL allocate isolated partitions (namespaces) based on the `kv_partitions` list in the declarative configuration.

#### Scenario: Backing up a KV partition to S3
- **GIVEN** an active S3 backend named `corporate-blob-store`
- **WHEN** a KV partition is configured with `sync_to_s3_backend_ref` pointing to that backend
- **THEN** the storage broker begins asynchronously writing the partition's SSTable snapshots to the configured S3 bucket using `system-faas-s3-proxy`.
