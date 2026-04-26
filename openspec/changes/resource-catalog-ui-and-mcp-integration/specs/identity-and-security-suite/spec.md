## ADDED Requirements

### Requirement: The desktop UI exposes a Resource Catalog view
The desktop UI SHALL register a `Resource Catalog` sidebar entry that switches to a `#view-resources` container rendering all logical mesh resources, distinguishing internal IPC aliases from external HTTPS egress targets and tagging entries that have not yet been re-sealed into `integrity.lock`.

#### Scenario: The operator opens the Resource Catalog
- **WHEN** the operator selects the `Resource Catalog` sidebar entry
- **THEN** the desktop frontend switches to `#view-resources`
- **AND** the view invokes the `get_resources` Tauri command
- **AND** rows are rendered with a colored badge per type (cyan = internal, blue = external, amber = pending seal)

#### Scenario: Pending overlay entries are visually distinguished from sealed entries
- **WHEN** an entry exists in the workspace overlay file but not in the sealed `integrity.lock`
- **THEN** the row is rendered with the amber `pending seal` badge
- **AND** the row exposes a tooltip indicating that a CLI re-seal is required to promote the resource

### Requirement: Tauri commands read sealed and pending mesh resources
The desktop backend SHALL register a `get_resources` Tauri command that returns the union of resources sealed in `integrity.lock` (via `tachyon_client::read_lockfile`) and resources staged in the workspace overlay file `tachyon.resources.json`. Overlay entries SHALL be flagged with `pending: true` so the UI can render the pending badge.

#### Scenario: get_resources merges sealed and overlay resources
- **WHEN** the desktop frontend invokes the `get_resources` Tauri command
- **THEN** the backend reads sealed resources from the workspace `integrity.lock`
- **AND** it merges entries from `tachyon.resources.json` with `pending: true`
- **AND** it returns a single deduplicated list keyed by resource name

### Requirement: Tauri commands stage mesh resources without re-sealing the lockfile
The desktop backend SHALL register `save_resource` and `delete_resource` Tauri commands that write to the workspace overlay file `tachyon.resources.json` rather than mutating `integrity.lock`. `save_resource` SHALL reject inputs whose name is empty and SHALL reject `external` resources whose target is not an HTTPS URL or a recognised loopback / `*.svc` cluster-local hostname. `delete_resource` SHALL succeed only when the resource exists in the overlay; resources that exist only in the sealed lockfile SHALL return an error directing the operator to perform a CLI re-seal.

#### Scenario: Saving an external resource validates the target
- **WHEN** the desktop frontend invokes `save_resource` with `type: "external"` and `target: "ftp://example.com"`
- **THEN** the backend rejects the request with a validation error
- **AND** the overlay file is left unchanged

#### Scenario: Saving a valid external resource writes to the overlay
- **WHEN** the desktop frontend invokes `save_resource` with `name: "stripe-api"`, `type: "external"`, `target: "https://api.stripe.com"`
- **THEN** the backend appends the entry to `tachyon.resources.json`
- **AND** subsequent `get_resources` calls include the resource flagged `pending: true`

#### Scenario: Deleting a sealed-only resource surfaces the re-seal hint
- **WHEN** the desktop frontend invokes `delete_resource` for a name that exists in `integrity.lock` but not in the overlay
- **THEN** the backend returns an error mentioning that a CLI re-seal is required
- **AND** the overlay file is left unchanged
