# edge-to-cloud-cdc-sync Specification

## Purpose
TBD - created by archiving change edge-to-cloud-cdc-sync. Update Purpose after archive.
## Requirements
### Requirement: Local writes to flagged resources emit asynchronous mutation events
When a FaaS writes to a persistence resource flagged with `sync_to_cloud: true` in `integrity.lock`, the `core-host` SHALL complete the local physical write first and then emit an asynchronous `tachyon.data.mutation` event onto the internal IPC bus, without delaying the write acknowledgement.

#### Scenario: Local KV write emits a mutation event
- **WHEN** a FaaS writes a key into a KV store flagged with `sync_to_cloud: true`
- **THEN** the host commits the write locally and acknowledges it to the FaaS without waiting for any cloud round-trip
- **AND** the host emits a `tachyon.data.mutation` event describing the write onto the internal IPC bus

### Requirement: system-faas-cdc spools and replays mutations with store-and-forward semantics
`system-faas-cdc` SHALL subscribe to `tachyon.data.mutation` events, spool them into a local persistent queue when the upstream cloud endpoint is unreachable, and drain the queue with exponential backoff once connectivity is restored.

#### Scenario: Cloud endpoint is offline then recovers
- **WHEN** `system-faas-cdc` receives mutation events while the cloud endpoint is unreachable
- **THEN** the events are appended to a local persistent queue without loss
- **AND** the FaaS continues attempting delivery using exponential backoff
- **WHEN** connectivity to the cloud endpoint is restored
- **THEN** the FaaS drains the queued mutations to the upstream endpoint in their original order
- **AND** mutations are acknowledged and removed from the local queue only after upstream success

