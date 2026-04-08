# Specifications: Hibernation Lifecycle

## 1. Schema Update (`integrity.lock`)
Volumes can now specify an eviction policy.

    {
        "volumes": {
            "cache-db": { 
                "type": "ram", 
                "size_mb": 128,
                "eviction_policy": "hibernate",
                "idle_timeout": "300s"
            }
        }
    }

## 2. The Hibernation Flow (Swap-Out)
1. **Trigger:** The 300s idle timeout is reached.
2. **Locking:** The host marks the volume state as `Hibernating`. No new instances can mount it.
3. **Spilling:** The host triggers the `system-faas-storage-broker`, asking it to copy the RAM volume contents to a persistent volume path (e.g., `/var/lib/tachyon/hibernation/<volume_id>.tar`).
4. **Release:** Upon completion, the broker notifies the host. The host drops the RAM allocation. State is now `OnDisk`.

## 3. The Restoration Flow (Swap-In)
1. **Trigger:** A request arrives for a target linked to this volume.
2. **Queueing:** The host router sees `OnDisk`. It places the request in an async wait queue.
3. **Loading:** The host asks the Storage Broker to read the `.tar` file back into a newly allocated RAM volume.
4. **Execution:** The state returns to `Active`. The router dequeues the HTTP request, instantiates the User FaaS, and proceeds.