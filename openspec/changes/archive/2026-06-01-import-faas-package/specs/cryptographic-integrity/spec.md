## ADDED Requirements

### Requirement: Live manifest GET returns IntegrityConfig directly
The system SHALL parse `GET /admin/manifest` responses as raw `IntegrityConfig`
JSON (not the `{config_payload, public_key, signature}` file wrapper). Client
functions `get_manifest_config`, `load_live_config_payload`, and `get_active_config`
MUST deserialise the response directly.

#### Scenario: Client fetches live config for mutation
- **WHEN** `get_manifest_config()` is called while connected
- **THEN** the returned `serde_json::Value` is the `IntegrityConfig` object with `routes`, `resources`, `config_version`, etc. at the top level

### Requirement: patch_and_apply_manifest increments config_version
The system SHALL increment `config_version` by one before re-signing the patched
manifest payload. A node rejects any incoming manifest whose version is not
strictly greater than the current running version.

#### Scenario: Successful manifest patch
- **WHEN** `patch_and_apply_manifest` is called with a config fetched from the live node
- **THEN** `config_version` is incremented before signing
- **THEN** the POST to `/admin/manifest` succeeds with 2xx

#### Scenario: 409 is prevented
- **WHEN** the live node's `config_version` is N
- **THEN** the patched manifest carries version N+1 and is accepted

### Requirement: tachyon:// asset URIs are valid route target module values
The system SHALL accept `tachyon://sha256:<hex>` strings as the `module` field of
route targets. These URIs contain `/` characters but are not filesystem paths and
MUST NOT be rejected by the `normalize_route_target` path-safety check.

#### Scenario: Route with asset URI passes validation
- **WHEN** a manifest is submitted with a route target `module: "tachyon://sha256:abcdef…"`
- **THEN** the node accepts the manifest without returning 400
- **THEN** the runtime resolves the module via `resolve_asset_uri` at request time
