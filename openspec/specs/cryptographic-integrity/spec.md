# cryptographic-integrity Specification

## Purpose
Define how the workspace seals an integrity manifest and how `core-host` verifies that embedded configuration before serving traffic.
## Requirements
### Requirement: Host build embeds integrity material from the manifest
The `core-host` build pipeline SHALL read `../integrity.lock` during compilation, re-run the build when that file changes, and expose the sealed configuration payload, public key, and signature to the binary through compile-time environment variables.

#### Scenario: Rebuilding after the manifest changes
- **WHEN** `core-host` is compiled and `integrity.lock` exists
- **THEN** `build.rs` reads the manifest before Rust compilation finishes
- **AND** Cargo is instructed to re-run the build if `integrity.lock` changes
- **AND** the binary receives `FAAS_CONFIG`, `FAAS_PUBKEY`, and `FAAS_SIGNATURE` values derived from the manifest

### Requirement: Host validates the sealed configuration before serving traffic
At startup, `core-host` SHALL hash the embedded configuration payload, reconstruct the verifying key and signature from the embedded hex values, and refuse to start the HTTP server if signature verification fails.

#### Scenario: Startup succeeds with a valid sealed configuration
- **WHEN** `core-host` starts with embedded integrity values that match the sealed configuration payload
- **THEN** the host verifies the signature successfully
- **AND** the host logs that the integrity check passed
- **AND** the HTTP server continues booting normally

#### Scenario: Startup aborts after configuration tampering
- **WHEN** the embedded signature does not validate the sealed configuration payload
- **THEN** `core-host` aborts startup immediately
- **AND** the process surfaces an integrity validation failure before binding the HTTP server

### Requirement: Integrity manifest seals route execution roles
The workspace SHALL treat `integrity.lock` as the signed source of truth for each configured route,
storing the normalized route path, an execution role of `user` or `system`, a logical service
`name`, a semantic `version`, optional dependency constraints, optional `allowed_secrets`,
route-scaling fields `min_instances` plus `max_concurrency`, optional top-level logical
`resources` aliases, and optional route volume mounts containing `host_path`, `guest_path`, and
`readonly`.

#### Scenario: Host consumes a sealed manifest with explicit route SemVer metadata
- **WHEN** `core-host` starts with a signed manifest whose `/api/faas-a` entry declares `name = "faas-a"`, `version = "2.0.0"`, and a dependency map containing `faas-b = "^3.1.0"`
- **THEN** integrity validation succeeds
- **AND** the runtime preserves the normalized route path
- **AND** the runtime loads the declared service identity and dependency constraints from the signed manifest

#### Scenario: Host rejects a sealed manifest whose route SemVer metadata is incomplete
- **WHEN** `core-host` starts with a sealed manifest whose route entry omits `version` or `dependencies`
- **THEN** integrity validation fails with `ERR_INTEGRITY_SCHEMA_VIOLATION`
- **AND** the host aborts startup before binding the HTTP server

### Requirement: Host rejects ambiguous or invalid sealed route metadata
`core-host` SHALL normalize sealed route paths, reject duplicates, and refuse to start if any sealed route metadata is invalid.

#### Scenario: Startup aborts after duplicate route metadata
- **WHEN** the embedded configuration payload contains the same normalized route more than once
- **THEN** `core-host` fails integrity validation before serving traffic
- **AND** the error reports that the sealed route metadata is ambiguous

### Requirement: Integrity manifest seals logical resource aliases
The signed `integrity.lock` payload SHALL allow a top-level `resources` map that binds a logical
alias to either an internal sealed route reference or an external HTTPS endpoint plus an HTTP
method allow-list.

#### Scenario: Host accepts a sealed internal resource alias
- **WHEN** `core-host` starts with a signed manifest containing `resources.inventory-api` of
  `type = "internal"`, `target = "inventory"`, and `version_constraint = "^1.2.0"`
- **THEN** integrity validation succeeds only if a compatible sealed route is present
- **AND** the normalized alias is retained as signed runtime configuration

#### Scenario: Host accepts a sealed external resource alias
- **WHEN** `core-host` starts with a signed manifest containing `resources.payment-gateway` of
  `type = "external"`, `target = "https://api.stripe.com/v1"`, and
  `allowed_methods = ["POST"]`
- **THEN** integrity validation succeeds
- **AND** the runtime retains the external target and normalized method allow-list from the signed
  manifest

#### Scenario: Host rejects ambiguous or unsafe resource aliases
- **WHEN** a sealed manifest defines a resource name that collides with a route name, omits the
  external method allow-list, or points an external resource at a non-HTTPS target
- **THEN** integrity validation fails before the host starts serving traffic
