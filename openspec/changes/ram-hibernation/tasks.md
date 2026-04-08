# Tasks: Change 049 Implementation

**Agent Instruction:** Implement the hibernation lifecycle. Ensure the host never performs synchronous file I/O on the main thread during swap-in or swap-out.

- [x] Extend the host volume manager to track `Active`, `Hibernating`, and `OnDisk` states and switch idle hibernating volumes into the snapshot flow.
- [x] Add snapshot and restore broker endpoints that persist RAM volume contents asynchronously and notify the host on completion.
- [x] Suspend requests that need `OnDisk` volumes until restore completes, then resume execution against the reactivated RAM volume.
- [x] Validate swap-out and swap-in behavior with a hibernating volume that preserves file contents across eviction.

## Validation Notes
1. Configure a RAM volume with `eviction_policy = "hibernate"` and a short idle timeout.
2. Write a timestamp into the volume, wait for the snapshot to complete, then confirm RAM is freed and a disk snapshot exists.
3. Trigger the target again and verify the original timestamp is restored and returned successfully.
