# Tasks: Change 015 Implementation

## 1. Sealed Route Roles

- [x] 1.1 Extend `tachyon-cli`, `integrity.lock`, and `core-host` to support sealed route entries with `path` and `role`.
- [x] 1.2 Add host route lookup helpers so `core-host` can resolve whether a sealed route runs as `user` or `system`.

## 2. Privileged Telemetry Guest

- [x] 2.1 Extend `wit/tachyon.wit` with a `tachyon:telemetry/reader` interface and a dedicated `system-faas-guest` world.
- [x] 2.2 Add a `system-faas-prom` component guest that reads telemetry snapshots from the host and renders Prometheus text.
- [x] 2.3 Instantiate privileged component guests through a system-only linker and prove that the same guest fails when executed as role `user`.

## 3. QoS And Validation

- [x] 3.1 Track active request pressure and shed `system` routes with `503 Service Unavailable` once the host is above the configured threshold.
- [x] 3.2 Update CI, Docker packaging, and local tests to build and validate the system guest artifact.
- [x] 3.3 Verify the change with `cargo fmt`, `cargo clippy`, `cargo test`, and `openspec validate --all`.
