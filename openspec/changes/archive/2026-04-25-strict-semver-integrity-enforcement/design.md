# Design: Strict Parser Implementation

## 1. Core Host (`core-host`)
The struct representing the sealed configuration payload (e.g., `ConfigPayload` or `RouteMetadata`) must be updated:
- Remove `Option<String>` or custom `serde(default)` attributes for the `version` field. It must be a strict `String` (or `semver::Version`).
- Remove `Option<HashMap<String, String>>` or `serde(default)` for the `dependencies` field. It must be explicitly defined, even if the JSON representation is an empty object `{}`.

### Boot Sequence Update
During the `bootstrap_integrity_check()`:
1. The payload signature is verified against the public key.
2. The payload JSON is deserialized into the strict struct.
3. If deserialization fails (e.g., missing `version` key), the host panics with a dedicated error code: `ERR_INTEGRITY_SCHEMA_VIOLATION`.

## 2. Manifest Production
The current workspace no longer exposes a standalone `tachyon-cli` manifest generator. Instead, the
checked-in `integrity.lock` and the `core-host` test helpers are the manifest production paths that
must remain compatible with the strict schema.

- Canonical serialization of `IntegrityRoute` must always emit `version` and `dependencies`, even
  when the dependency map is empty.
- The checked-in `integrity.lock` must be regenerated so the embedded runtime payload satisfies the
  new strict parser.
