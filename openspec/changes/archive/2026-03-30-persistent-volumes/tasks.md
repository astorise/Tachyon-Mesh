# Tasks: Change 022 Implementation

- [x] 1.1 Add route-scoped sealed volume metadata to `tachyon-cli` and the host
  integrity schema, including CLI parsing for `--volume`.
- [x] 1.2 Validate and normalize sealed volume mounts in `core-host`, then
  preopen them into legacy and component WASI contexts with read-only support.
- [x] 1.3 Add the `guest-volume` component guest and package it in the local
  build and CI workflows.
- [x] 1.4 Add automated coverage for manifest parsing, runtime volume
  validation, and persisted guest I/O through the mounted directory.
- [x] 1.5 Verify with `cargo fmt`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
  `cargo test --workspace`, and `openspec validate --all`.
