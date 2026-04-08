# Proposal: Change 049 - Stateful Scale-to-Zero (RAM Volume Hibernation)

## Context
Change 033 introduced RAM volume eviction to save memory. However, for stateful applications (like in-memory databases or caching layers), simply deleting the RAM volume causes data loss. To offer true "Scale-to-Zero" without data loss, we need a mechanism to hibernate the RAM volume to persistent storage (Disk) upon idle timeout, and restore it seamlessly upon the next incoming request.

## Objective
1. Implement a Volume Hibernation lifecycle (`Active` -> `Hibernating` -> `OnDisk`).
2. Delegate the I/O-heavy task of copying RAM to Disk to the `system-faas-storage-broker` to prevent blocking the `core-host` executor threads.
3. Update the Mesh Router to handle "Data Cold Starts" by pausing incoming requests while a volume is being swapped back into RAM.

## Scope
- Update `integrity.lock` to support `eviction_policy: "hibernate"` for RAM volumes.
- Add serialization/deserialization commands to the Storage Broker.
- Implement an async wait queue in the router for requests hitting hibernated targets.

## Success Metrics
- An idle FaaS with a 50MB RAM volume correctly dumps its state to a local `.snapshot` file and releases the memory.
- The next HTTP request targeting this FaaS waits gracefully (e.g., ~50ms delay) while the state is restored to RAM, and the FaaS resumes execution with intact data.