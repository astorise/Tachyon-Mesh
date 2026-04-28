# Proposal: Active Hot-Reload & Zero-Downtime Deployment

## Context
Tachyon Mesh already utilizes `ArcSwap` in its `AppState` to store the `IntegrityRuntime`. This allows for atomic updates to the router's configuration. However, the system currently lacks a trigger mechanism. Administrators must restart the host to apply changes to `integrity.lock` or deploy new Wasm modules, causing interruptions to active HTTP/3 streams and WebSocket connections.

## Proposed Solution
We will implement an **Active Watcher & Swap Cycle**:
1. **File Watcher:** Integrate the `notify` crate in `core-host` to monitor the `integrity.lock` file.
2. **Atomic Swap:** When a change is detected (and validated), the host will load the new manifest, instantiate the new Wasm modules, and call `state.runtime.store()` to swap the pointer.
3. **Graceful Transition:** Thanks to `ArcSwap`, existing requests will continue to use the "old" runtime until they finish, while all new incoming requests will automatically use the "new" one.

## Objectives
- Eliminate downtime for FaaS updates and resource catalog changes.
- Ensure that active network streams are never dropped during a configuration reload.
- Provide a seamless "Push-to-Deploy" experience from Tachyon Studio.