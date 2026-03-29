## Why

The workspace already supports sealing route roles through `tachyon-cli --system-route`, and
`core-host` already enforces those sealed roles at runtime. The active change
`tauri-roles-update` was drafted in an outdated format and no longer matches the implementation
or the synced specs, which causes `openspec validate --all` to fail and blocks archive.

## What Changes

- Realign the change with the current role-aware `tachyon-cli` interface instead of proposing a
  second manifest schema.
- Modify `tauri-configurator` so its CLI requirement explicitly covers privileged system routes in
  addition to regular user routes.
- Document that the sealed manifest continues to derive guest module resolution from the normalized
  route path; this change only extends the configurator contract around route roles.
- Verify the existing Rust implementation and CI path against the repaired change artifacts.

## Capabilities

### Modified Capabilities

- `tauri-configurator`: describe role-aware route inputs for `tachyon-cli generate`.

## Impact

- Fixes OpenSpec validation for the active change.
- Keeps the `tauri-configurator` capability aligned with the shipped `--system-route` workflow.
- Avoids introducing a conflicting `module` field into the integrity schema that is already synced
  through `cryptographic-integrity`.
