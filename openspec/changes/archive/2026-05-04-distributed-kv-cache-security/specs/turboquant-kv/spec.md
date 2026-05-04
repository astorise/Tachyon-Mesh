# turboquant-kv Delta

## ADDED Requirements

### Requirement: Distributed AI KV Caches MUST be isolated by Tenant
When the `system-faas-model-broker` stores context tensors in Turboquant, the storage layer SHALL enforce logical isolation based on the `tenant_id` extracted from the active request context. A cache hit MUST NOT occur across different tenant boundaries, preventing prompt-bleeding vulnerabilities.

#### Scenario: Tenant A and Tenant B using the same model
- **GIVEN** a shared model and a distributed KV cache with `tenant_isolation: true`
- **WHEN** Tenant B submits a prompt identical to Tenant A's previous prompt
- **THEN** the cache engine treats it as a cache miss and recomputes the context, ensuring zero cross-tenant data exposure.

### Requirement: Gossiped Cache State MUST utilize Transparent Data Encryption
When configured, the cache synchronization protocol SHALL encrypt all KV cache tensors in transit and at rest using the cluster's TDE keys, ensuring that physical access to the Edge node or interception of the network overlay does not expose user inference data.

#### Scenario: Encrypting distributed KV cache replication
- **GIVEN** a distributed KV cache configured with transparent data encryption
- **WHEN** cache tensors are replicated to a peer node
- **THEN** the synchronization payload is encrypted with the active cluster TDE key
- **AND** persisted replicas remain encrypted at rest.
