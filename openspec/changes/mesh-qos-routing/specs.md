# Specifications: QoS-Aware Mesh Routing

## 1. Upgraded Telemetry Interface
The `core-host` now exposes detailed queues to the System FaaS.

    interface telemetry {
        record hardware-queue-stats {
            realtime-depth: u32,
            standard-depth: u32,
            batch-depth: u32,
        }
        
        get-accelerator-load: func() -> result<list<tuple<string, hardware-queue-stats>>, string>;
    }

*(Returns data like: `[("gpu", {realtime: 0, standard: 5, batch: 120}), ("npu", {...})]`)*

## 2. Asymmetric Routing Policies
When an HTTP request arrives, the `core-host` reads the target's configured QoS and preferred device. It applies the following logic before instantiating the WASM module:

- **Policy: RealTime**
  - **Threshold:** Local queue > 0.
  - **Action:** If local accelerator is busy, query the Routing Table for a peer with an idle accelerator. Forward immediately. High network cost is justified by latency requirements.

- **Policy: Standard**
  - **Threshold:** Local queue > 10 (or estimated wait > 100ms).
  - **Action:** Balance between local wait and network hop. Forward only if a peer is completely idle.

- **Policy: Batch**
  - **Threshold:** Local queue > 1000 (VRAM risk).
  - **Action:** Never overflow to a peer node's execution engine (saves bandwidth). If local queue is full, route to local `system-faas-buffer` (Change 038) to spill to disk.

## 3. The Global Hardware Map (Gossip Payload)
The `system-faas-gossip` now broadcasts compact UDP datagrams containing the hardware states. It computes the "Best Remote Node" for each QoS class and accelerator type, and updates the host's atomic routing table via the `update-target` hook.