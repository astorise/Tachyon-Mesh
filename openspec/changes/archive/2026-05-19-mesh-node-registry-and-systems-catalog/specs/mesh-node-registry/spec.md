## ADDED Requirements

### Requirement: Enrollment ceremony runs inside the FaaS

The operator-side enrollment ceremony (PIN generation, CSR signing, approval state machine) SHALL execute inside the `system-faas-node-registry` WASM component, built against the `control-plane-faas` WIT world. `core-host` MUST NOT retain a parallel implementation of the ceremony; it SHALL only forward the relevant admin HTTP routes (`/admin/enrollment/*`, `/admin/nodes/*`) to the FaaS `handle-request` export.

#### Scenario: Approval routes are served by the FaaS

- **GIVEN** the host has loaded `system-faas-node-registry`
- **WHEN** an operator POSTs an approval to `/admin/enrollment/approve/{session_id}`
- **THEN** the host forwards the request to the FaaS `handle-request` export
- **AND** the FaaS performs the PIN check, signs the CSR, and returns the signed certificate in the response
- **AND** no `core-host` Rust module retains the approval state

### Requirement: Persistent enrolled-node registry

The `system-faas-node-registry` component SHALL persist a record for every node whose enrollment has been approved, keyed by the node's stable `node_id`. Persistence MUST go through the `kv-partition::table` WIT resource on a table named `"node-registry"`, which the host backs with a dedicated ReDB table inside `CoreStore`. Each record MUST survive host process restarts.

#### Scenario: Approved enrollment is persisted through kv-partition

- **WHEN** the FaaS completes an enrollment approval
- **THEN** it invokes `kv-partition::table::set` on the `"node-registry"` table with the `node_id` as key and a serialized `EnrolledNode` record as value
- **AND** subsequent calls to `list-enrolled-nodes` return the new entry with `status = "awaiting-capabilities"`
- **AND** the record contains the node's public key, the issuing operator's identity, and the approval timestamp

#### Scenario: Registry survives host restart

- **GIVEN** at least one approved node is present in the registry
- **WHEN** the host process restarts and reloads `system-faas-node-registry`
- **THEN** the registry MUST return the same node entries on first read after restart
- **AND** the `status` field MUST be set to `"unknown"` for every entry until a heartbeat refreshes it

### Requirement: Capability reporting endpoint

The host SHALL forward authenticated `POST /admin/nodes/{node_id}/capabilities` requests to the FaaS. The FaaS SHALL accept a `NodeCapabilities` payload and update the corresponding registry record through `kv-partition::table::set`.

#### Scenario: Node reports capabilities on first heartbeat

- **WHEN** an enrolled node POSTs a valid `NodeCapabilities` payload to `/admin/nodes/{node_id}/capabilities` with its mTLS identity
- **THEN** the host forwards the request to the FaaS
- **AND** the registry record for `node_id` is updated through `kv-partition` with the reported capabilities
- **AND** the record's `status` transitions to `"online"`
- **AND** the record's `last_seen` is set to the current host time

#### Scenario: Unknown node identity is rejected

- **WHEN** a node POSTs `NodeCapabilities` for a `node_id` not present in the registry
- **THEN** the FaaS returns a `404 Not Found` response and the host propagates it
- **AND** the registry state is unchanged

### Requirement: Registry query API

The FaaS SHALL expose two read functions exported through its WIT-defined query interface: `list-enrolled-nodes` returning every record, and `get-node-capabilities(node_id)` returning the record for a single node or absence. Both functions MUST be reachable through HTTP routes forwarded by the host (`GET /admin/nodes`, `GET /admin/nodes/{node_id}`).

#### Scenario: List returns every enrolled node

- **GIVEN** three nodes have been approved
- **WHEN** `list-enrolled-nodes` is invoked (directly or through `GET /admin/nodes`)
- **THEN** the result contains exactly three entries
- **AND** each entry exposes `node_id`, `status`, `last_seen`, and (if reported) `capabilities`

#### Scenario: Unknown node id returns absence

- **WHEN** `get-node-capabilities("does-not-exist")` is invoked
- **THEN** the result indicates the node is not present (`null` over HTTP, `None`-equivalent at the WIT boundary)
- **AND** the registry is not mutated

### Requirement: NodeCapabilities schema

`NodeCapabilities` SHALL contain at minimum: total RAM in MiB, available RAM in MiB, a list of declared accelerators (`gpu`, `tpu`, `npu`), per-GPU stats (id, model, VRAM total / used in MiB, compute utilization 0-100), an optional `region` string, and an optional `zone` string. The same schema MUST be representable as a Rust struct (for `tachyon-client` and the FaaS) and serialised as JSON bytes when stored through `kv-partition::table::set`.

#### Scenario: Schema round-trips through kv-partition

- **WHEN** a `NodeCapabilities` value is serialised by the FaaS, written via `kv-partition::table::set`, read back via `kv-partition::table::get`, and deserialised
- **THEN** the resulting struct carries identical RAM, accelerator, GPU, region, and zone fields
- **AND** no field is silently dropped

#### Scenario: Region and zone are optional

- **GIVEN** a node reports its capabilities without `region` or `zone`
- **WHEN** the record is persisted and re-read
- **THEN** both `region` and `zone` round-trip as `None` / `null`
- **AND** the absence of these fields does NOT prevent the row from being returned by `list-enrolled-nodes`

### Requirement: Last-seen tracking

The FaaS SHALL update each node's `last_seen` timestamp on every authenticated heartbeat, and transition `status` to `"stale"` if no heartbeat has been received for longer than the configured stale threshold (default 60 seconds). The stale sweep MUST run inside the FaaS `on-tick` export so it does not require a host-side scheduler.

#### Scenario: Status becomes stale after threshold

- **GIVEN** a node with `status = "online"` last seen 120 seconds ago
- **WHEN** the FaaS's `on-tick` export runs (or `list-enrolled-nodes` is invoked, whichever comes first)
- **THEN** the node's `status` is reported as `"stale"`
- **AND** the `last_seen` value remains the original timestamp
