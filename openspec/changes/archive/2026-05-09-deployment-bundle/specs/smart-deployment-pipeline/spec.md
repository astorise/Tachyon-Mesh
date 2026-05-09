# smart-deployment-pipeline

## ADDED Requirements

### Requirement: Client-Side Deployment Bundle
The Tachyon client SHALL package a domain configuration into a
single gzip-compressed tar archive containing a top-level
`manifest.yaml` and an optional `assets/` directory with local WASM
binaries. The client SHALL NOT generate `integrity.lock` locally; it
SHALL POST the bundle to `/admin/manifest/bundle` with content-type
`application/gzip`.

#### Scenario: Bundle layout is deterministic
- **WHEN** the client builds a deployment bundle
- **THEN** the archive contains exactly one `manifest.yaml` at the
  archive root
- **AND** every local asset is placed under `assets/` with a path
  matching the value declared in the manifest's `dependencies[].source`

#### Scenario: Client posts the bundle without producing integrity.lock
- **WHEN** `bundle_and_apply_manifest` is invoked
- **THEN** the client does not write a local `integrity.lock`
- **AND** the HTTP request goes to `/admin/manifest/bundle` with
  `Content-Type: application/gzip`

### Requirement: Server-Side Resolution and Locking
The core host SHALL accept the bundle, extract the manifest and
assets to a scratch directory, validate the SemVer dependency
constraints declared in the manifest, sign the resulting
`integrity.lock` with the host signing key, and return the new
configuration version.

#### Scenario: Successful bundle apply returns 200 with the version
- **GIVEN** a bundle whose manifest declares only resolvable
  dependencies
- **WHEN** the host processes the bundle
- **THEN** it generates an `integrity.lock` signed with the host key
- **AND** the response status is 200 with a JSON body containing the
  new `configVersion`

### Requirement: Override Conflict Detection
The core host SHALL detect when a bundled asset's pinned version is
shadowed by a strictly higher cluster-cached version that also
satisfies the manifest constraint, and SHALL respond with HTTP 428
including a JSON `conflicts` list naming each shadowed dependency,
its bundled version, and the cluster version.

#### Scenario: Bundle with override returns 428
- **GIVEN** the bundle pins `toto = 2.3.5` under a `^2.0.0`
  constraint and the cluster cache holds `toto = 2.4.1`
- **WHEN** the host processes the bundle
- **THEN** the response status is 428
- **AND** the JSON body contains a `conflicts` entry with
  `name=toto`, `bundled_version=2.3.5`, and `cluster_version=2.4.1`

### Requirement: Interactive Resolution Modal
Tachyon-UI SHALL intercept HTTP 428 responses from the bundle apply
flow and display a modal listing each conflict with two actions per
entry: "Use Cluster Version" (drops the local `source` so the cluster
cache version is used) and "Force Local Version" (rewrites the
constraint to strict equality `=x.y.z` matching the bundled asset).
The UI SHALL retry the bundle apply automatically once the operator
selects a resolution for every conflict.

#### Scenario: Operator chooses cluster version for a conflict
- **GIVEN** a conflict modal is shown for `toto = 2.3.5` vs cluster
  `2.4.1`
- **WHEN** the operator picks "Use Cluster Version"
- **THEN** the manifest entry for `toto` loses its `source` field
- **AND** the client rebuilds the bundle and re-POSTs it without the
  `assets/toto.wasm` payload

#### Scenario: Operator forces local version
- **WHEN** the operator picks "Force Local Version"
- **THEN** the manifest constraint becomes `=2.3.5`
- **AND** the rebuilt bundle is re-POSTed and accepted

### Requirement: Manifest Dependencies Schema
The manifest YAML SHALL accept a `dependencies` mapping where each
entry has a SemVer `version` constraint string and an optional local
`source` path pointing inside the bundle's `assets/` directory.

#### Scenario: Dependencies parse with optional source
- **WHEN** the host parses a manifest containing
  `dependencies: { toto: { version: "^2.0.0", source: "./assets/toto.wasm" } }`
- **THEN** the parser accepts the entry
- **AND** the resolver treats `toto` as eligible for the bundled
  asset path while still subject to the SemVer constraint
