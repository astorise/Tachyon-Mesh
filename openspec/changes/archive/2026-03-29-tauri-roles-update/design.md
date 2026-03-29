## Summary

`tachyon-cli` already models route roles through separate regular and privileged route inputs, and
`core-host` already reads sealed `path` plus `role` metadata from `integrity.lock`. This change
does not introduce a new `module` field because the host still resolves the guest artifact name
from the normalized route path, which is the contract already implemented and covered by the main
specs.

## CLI Surface

- Keep `--route` for normal guest routes and `--system-route` for privileged routes.
- Continue normalizing paths before writing the manifest so the CLI and host share the same sealed
  route identity.
- Keep privileged route selection explicit through the flag used, with normal routes remaining the
  default path.

## Manifest Compatibility

- Preserve the current canonical payload shape with route entries containing only `path` and
  `role`.
- Leave signature generation and host build embedding unchanged.
- Avoid adding a second route-to-module contract on top of the route-role model already merged into
  `cryptographic-integrity`.

## Verification

- Validate the OpenSpec delta against `tauri-configurator`.
- Exercise `tachyon-cli`, workspace tests, and release builds before archive.
