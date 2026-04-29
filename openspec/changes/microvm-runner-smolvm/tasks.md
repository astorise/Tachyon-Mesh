# Implementation Tasks

## Phase 1: Schema and Dispatcher
- [x] Update `FaaSConfig` in `core-host` to support the new `FaaSRuntime` enum.
- [ ] Update the UI/CLI to allow users to select "MicroVM" and configure CPU/RAM limits.
- [x] Modify the `core-host` execution dispatcher to route MicroVM requests to the new system FaaS.

## Phase 2: MicroVM Runner Integration
- [x] Bootstrap `systems/system-faas-microvm-runner`.
- [ ] Add the `smolvm` crate as a dependency.
- [ ] Implement the VM boot sequence (allocating TAP network devices if needed, setting memory limits, and invoking KVM).

## Phase 3: Guest Communication (IPC Proxy)
- [ ] Implement the vsock/serial communication bridge in the runner.
- [ ] Write a tiny generic "Guest Agent" (in Rust or C) that runs inside the Alpine MicroVM to receive the Tachyon payload, execute the user's Python/Node script, and return the stdout/stderr.

## Phase 4: Validation
- [ ] **Native Payload Test:** Create a `.smolmachine` containing a Python script that imports `numpy` and performs a matrix calculation.
- [ ] Deploy it to Tachyon Mesh.
- [ ] Trigger the endpoint via HTTP/3. Verify the runner boots the VM, executes the Python script, and returns the correct result in < 500ms.
