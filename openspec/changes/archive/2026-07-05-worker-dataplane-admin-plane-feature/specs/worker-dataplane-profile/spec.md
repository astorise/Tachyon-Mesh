## ADDED Requirements

### Requirement: core-host MUST support a worker data-plane-only build profile
`core-host` SHALL provide a Cargo feature `admin-plane`, included in `default`, that gates the entire `/admin/*` authenticated HTTP surface (IAM, manifest/canary/chaos control, node registry, asset uploads, OpenAPI/Swagger docs, all mounted via `admin_plane::authenticated_routes`). Building with `--no-default-features` and the feature omitted SHALL produce a binary that does not mount that surface at all, while still serving FaaS routes and the enrollment-bootstrap endpoints.

#### Scenario: Default build exposes the full admin surface
- **WHEN** `core-host` is built or run with its default feature set
- **THEN** `/admin/nodes`, `/admin/iam/*`, `/admin/manifest`, and the other authenticated admin routes are mounted and reachable (subject to bearer-token auth)

#### Scenario: Worker build has no admin surface
- **WHEN** `core-host` is built with `--no-default-features` and `admin-plane` is not enabled
- **THEN** a request to `/admin/nodes` (or any other route from `admin_plane::authenticated_routes`) returns `404 Not Found` rather than `401 Unauthorized`, because the route is not registered in the router at all

### Requirement: Enrollment bootstrap MUST remain available without the admin-plane feature
`POST /admin/enrollment/start` and `GET /admin/enrollment/poll/{session_id}` SHALL be compiled and mounted unconditionally, regardless of whether the `admin-plane` feature is enabled, so a worker-profile node stays enrollable and remains a valid answering peer for another unenrolled node's outbound enrollment call. `POST /admin/enrollment/approve` (operator PIN approval) SHALL remain gated behind `admin-plane`, since approval is an operator action expected to happen against an admin-plane node.

#### Scenario: Worker node completes zero-touch/PIN enrollment bootstrap
- **GIVEN** a `core-host` binary built with `admin-plane` disabled
- **WHEN** a caller sends `POST /admin/enrollment/start` with a node public key
- **THEN** the request is forwarded to `system-faas-node-registry` and returns `201 Created` with a session id, exactly as on an admin-plane build
- **AND** `GET /admin/enrollment/poll/{session_id}` for that session returns `204 No Content` while pending, not `404`

#### Scenario: Enrollment approval still requires an admin-plane node
- **GIVEN** a `core-host` binary built with `admin-plane` disabled
- **WHEN** a caller sends `POST /admin/enrollment/approve`
- **THEN** the request returns `404 Not Found`, since that route is only mounted by `admin_plane::authenticated_routes`

### Requirement: Code orphaned by disabling admin-plane MUST NOT trigger dead-code CI failures
Any function, type, or field outside `admin_plane.rs`/`openapi.rs` whose only caller is the gated admin surface SHALL be annotated `#[cfg_attr(not(feature = "admin-plane"), allow(dead_code))]` (or, where it has no other caller including tests, hard `#[cfg(feature = "admin-plane")]`) so that both the default and worker-profile feature combinations compile cleanly under the CI feature-matrix's `RUSTFLAGS="-D dead_code"`.

#### Scenario: Worker-profile build has zero dead-code warnings
- **WHEN** `RUSTFLAGS="-D dead_code" cargo check -p core-host --no-default-features --features ring` is run
- **THEN** the build succeeds with no dead-code errors
