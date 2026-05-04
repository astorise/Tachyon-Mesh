# Execution Tasks for Codex

- [x] **Task 1 (Scaffolding)**: Create two new Cargo crates in the `systems/` directory: `system-faas-config-api` and `system-faas-gitops-broker`.
- [x] **Task 2 (Gitoxide Integration)**: Add `gix` (with suitable pure-rust features) to the `system-faas-gitops-broker` dependencies. Implement a basic function to initialize a git repo in a given directory path.
- [x] **Task 3 (WASI Volume Binding)**: Modify `core-host/src/host_core/component_hosts.rs` to ensure the GitOps broker instance is initialized with a preopened WASI directory (e.g., `Dir::from_std_file(...)` mapped to `/var/lib/tachyon/config-store/`).
- [x] **Task 4 (Event Channel)**: In `core-host/src/host_core.rs` (or equivalent state module), define a `tokio::sync::broadcast` channel for `ConfigUpdate` events to enable the Pub/Sub architecture.
- [x] **Task 5 (OpenSpec Traceability)**: Ensure the `specs/distributed-control-plane/spec.md` delta is accurate and reflects the new GitOps environment model.
