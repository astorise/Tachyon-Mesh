# Tasks: Change 040 Implementation

**Agent Instruction:** Read `proposal.md` and the delta spec under `specs/`. Implement the host-level volume constraints and the storage broker system FaaS.

- [x] Reject writable direct host mounts for user targets and map read-only mounts with read-only WASI permissions.
- [x] Create the `system-faas-storage-broker` component and accept queued write requests over the internal mesh.
- [x] Process queued write requests sequentially inside the broker so filesystem mutations remain ordered and consistent.
- [x] Validate read-only enforcement and broker-mediated concurrent writes against a shared host volume.
