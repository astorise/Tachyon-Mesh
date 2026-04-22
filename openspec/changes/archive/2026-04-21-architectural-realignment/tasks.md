# Tasks: Change 072 Implementation

- [x] Inject the `connection-overlay` form into `tachyon-ui/index.html` before the module script and wire the UI controller to the new `connect-btn` trigger.
- [x] Delete `core-host/src/asset_registry.rs` and `core-host/src/model_broker.rs`, remove their host-local module wiring, and replace the admin storage routes with system-component proxy handlers.
- [x] Add `systems/system-faas-registry` and `systems/system-faas-model-broker`, register them in the workspace/build pipeline, and implement disk-backed asset and multipart model storage.
- [x] Enforce bearer-token validation inside `core-host/src/server_h3.rs` for `/admin/*` requests through the `system-faas-auth` component before dispatch.
- [x] Validate the workspace (`openspec validate --changes`, `cargo check --workspace`) and capture proof snippets showing the overlay markup and the absence of host compilation for `model_broker.rs`.
