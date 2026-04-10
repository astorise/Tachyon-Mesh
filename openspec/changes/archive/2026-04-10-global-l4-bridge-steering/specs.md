# Specifications: Mesh-Aware Media Proxy

## 1. Host Configuration: Public Endpoint
The `integrity.lock` must define how the node is reachable from the outside world for raw TCP/UDP, as NATs/LoadBalancers can hide the real IP.

    {
        "host": {
            "advertise_ip": "203.0.113.50"
        }
    }

## 2. Updated Bridge WIT Interface
The returned tuple must now include the IP address of the node hosting the bridge.

    interface bridge-controller {
        record bridge-endpoint {
            ip: string,
            port-a: u16,
            port-b: u16
        }

        create-bridge: func(config: bridge-config) -> result<bridge-endpoint, string>;
    }

## 3. Delegation Logic
1. **Request:** `User FaaS` calls `tachyon:network/bridge.create-bridge`.
2. **Evaluation:** `system-faas-bridge` checks local L4 capacity (via Telemetry).
3. **Local Allocation (Happy Path):** If capacity < 80%, calls Host FFI `bind_dynamic`, returns local `advertise_ip`.
4. **Mesh Delegation (Overflow):** If capacity > 80%, looks up the Gossip table for the least-loaded peer. Makes an mTLS call to the peer's `system-faas-bridge`. Returns the peer's `advertise_ip` and allocated ports.