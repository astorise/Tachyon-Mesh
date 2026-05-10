# Tasks

- [x] 1. Add `asset_versions: BTreeMap<String, String>` with `serde(default)`
       to `IntegrityConfig` in `domain_types.rs`.
- [x] 2. Remove `TACHYON_BUNDLE_FAKE_CONFLICTS` constant and its associated
       env-var mock logic from `integrity_config.rs`.
- [x] 3. Rewrite `detect_dependency_conflicts` to accept the runtime
       `asset_versions` map and perform real `semver::VersionReq` /
       `semver::Version` comparisons.
- [x] 4. Add `extract_semver_version` helper that strips range operators and
       returns a parseable version string.
- [x] 5. Update `admin_manifest_bundle_handler` to: (a) run conflict detection
       after parsing the config (not before), (b) populate `asset_versions` in
       the config before re-serialising, so the signed `integrity.lock`
       includes the deployed version map.
- [x] 6. Add eight unit tests in `integrity_admin.rs` covering the full
       conflict matrix.
- [x] 7. Verify `cargo check` and the full `integrity_admin` test suite pass
       with no regressions.
