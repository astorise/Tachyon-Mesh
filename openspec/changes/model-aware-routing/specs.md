# Specifications: Hot-VRAM Affinity

## 1. Upgraded Telemetry for VRAM State
The host telemetry now reports the exact models occupying the accelerators.

    interface telemetry {
        record hardware-state {
            queue-depth: u32,
            hot-models: list<string>
        }
        
        get-accelerator-load: func() -> result<list<tuple<string, hardware-state>>, string>;
    }

## 2. Model Affinity Routing Matrix
When the Host Router considers overflowing an AI request (via Change 046 logic), it applies a secondary filter:
- **Condition:** Does the remote peer have `target_model_alias` in its `hot-models` list?
- **RealTime QoS:** Strict Affinity. If no peer has the model hot, the request stays local (even if the local queue is heavy), because waiting 5 seconds in a local queue is always faster than a 30-second remote cold start.
- **Batch QoS:** Loose Affinity. If local queues are critically full, it can overflow to a peer without the model, forcing a remote cold start (acceptable for background jobs).