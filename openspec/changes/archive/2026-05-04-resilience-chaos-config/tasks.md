# Execution Tasks for Codex

- [x] **Task 1**: Create the file `wit/config-resilience.wit` in the workspace root and populate it with the interface defined in `design.md`.
- [x] **Task 2**: Update the `system-faas-config-api` crate's `wit/world.wit` to include the `config-resilience` interface.
- [x] **Task 3**: In `system-faas-config-api/src/lib.rs`, implement the scaffolding for the `config-resilience` CRUD operations (returning `Ok(())` for now) to pass the compiler checks.
- [x] **Task 4**: Create the OpenSpec delta file in `specs/l7-resiliency/spec.md` (overwriting or appending to the existing one) to map the new UI/GitOps schema requirements.
