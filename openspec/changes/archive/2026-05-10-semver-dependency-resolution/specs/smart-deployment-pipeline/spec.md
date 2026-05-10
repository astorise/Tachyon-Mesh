# smart-deployment-pipeline

## MODIFIED Requirements

### Requirement: Override Conflict Detection
The core host SHALL compare each bundled dependency that carries a local
`source` path against a persistent cluster asset-version registry using real
SemVer semantics. A conflict SHALL be reported only when the cluster registry
holds a version that is strictly greater than the bundled version AND still
satisfies the same SemVer constraint declared in the manifest's `dependencies`
block.

#### Scenario: Cluster has a strictly higher compatible version
- **GIVEN** the cluster registry records `toto = 2.4.1`
- **AND** the bundle declares `toto: { version: "^2.0.0", source: "./assets/toto.wasm" }` with bundled version `2.3.5`
- **WHEN** the host processes the bundle
- **THEN** the response status is 428
- **AND** the JSON body identifies `toto` as a conflict with `bundledVersion=2.3.5` and `clusterVersion=2.4.1`

#### Scenario: Cluster version does not satisfy the constraint
- **GIVEN** the cluster registry records `toto = 3.0.0`
- **AND** the bundle declares `toto: { version: "^2.0.0", source: "./assets/toto.wasm" }`
- **WHEN** the host processes the bundle
- **THEN** no conflict is reported (3.0.0 is outside ^2.x)
- **AND** the bundle is applied

#### Scenario: Cluster version equals the bundled version
- **GIVEN** the cluster registry records `toto = 2.3.5`
- **AND** the bundle declares the same version
- **WHEN** the host processes the bundle
- **THEN** no conflict is reported

#### Scenario: Dependency without local source is never a conflict candidate
- **GIVEN** a manifest dependency has no `source` field
- **WHEN** the host processes the bundle
- **THEN** the dependency is skipped during conflict detection regardless of what the cluster registry holds

## ADDED Requirements

### Requirement: Asset Version Registry
`IntegrityConfig` SHALL persist a `asset_versions` map that records the
deployed SemVer string of each bundled asset after a successful bundle apply.
The map SHALL be updated inside the signed `integrity.lock` so it survives
reloads and propagates via gossip.

#### Scenario: Successful apply updates the registry
- **GIVEN** a bundle declares `toto: { version: "^2.3.5", source: "..." }`
- **WHEN** the host applies the bundle without conflicts
- **THEN** the written `integrity.lock` contains `asset_versions: { "toto": "2.3.5" }`
- **AND** a subsequent bundle apply for `toto` uses this entry as the cluster baseline
