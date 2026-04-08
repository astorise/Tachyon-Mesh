# Specifications: Dynamic Bridging & System FaaS Controller

## 1. The Bridge WIT Interface (`tachyon:network/bridge`)
This interface allows a User FaaS to communicate with the `system-faas-bridge` to orchestrate the network.

    interface bridge-controller {
        record bridge-config {
            client-a-addr: string,
            client-b-addr: string,
            timeout-seconds: u32
        }

        // Requests the host to open two dynamic ports and relay traffic
        create-bridge: func(config: bridge-config) -> result<tuple<u16, u16>, string>;
        
        // Explicitly close a bridge
        destroy-bridge: func(bridge-id: string) -> result<_, string>;
    }

## 2. Host-Level Data Plane (The "Relay")
When a bridge is created:
- The `core-host` binds two ephemeral UDP sockets.
- It spawns a dedicated, lightweight Tokio task that loops: `socket_a.recv_from() -> socket_b.send_to()` and vice versa.
- **Critical:** No WASM code is invoked during this loop. The bytes stay in the Host's memory space.

## 3. Session Lifecycle & Security
- **Ephemeral Ports:** Ports are allocated from a specific range (e.g., 10000-20000).
- **Auto-Cleanup:** If no packets are detected on a bridge for X seconds, the Host automatically destroys the relay and frees the ports.
- **Identity:** Only the `system-faas-bridge` has the privilege to call the Host's internal `bind_dynamic` method. User FaaS must go through the Bridge Broker.