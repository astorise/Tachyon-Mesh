## ADDED Requirements

### Requirement: Host mounts sealed route volumes into the guest filesystem
The `core-host` runtime SHALL preopen every sealed route volume into the
request-scoped WASI context for both legacy WASI guests and Component Model
guests, while honoring the sealed read-only flag.

#### Scenario: Stateful guest persists data through a mounted directory
- **WHEN** `/api/guest-volume` is sealed with a host directory mounted at `/app/data`
- **AND** a client sends `POST Hello Stateful World` to `/api/guest-volume`
- **AND** the client later sends `GET /api/guest-volume`
- **THEN** the guest writes `state.txt` under `/app/data`
- **AND** the subsequent `GET` returns `Hello Stateful World`
- **AND** the host filesystem contains the persisted file in the mounted host directory

#### Scenario: Read-only guest volume denies writes
- **WHEN** a sealed route volume is mounted with `readonly = true`
- **AND** the guest attempts to write under the configured `guest_path`
- **THEN** the guest receives a WASI permission error
- **AND** the host volume contents are not modified
